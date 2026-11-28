//! How a picture is framed, measured from its pixels alone: where its subject sits (on a
//! third, in the middle, in a corner), whether the edge of the frame cuts it, how busy the
//! edges are next to the rest, and whether a long near-level line (a horizon) is tilted.
//!
//! No model: the subject is what stands out of a saliency map (colour unlike the picture's
//! mean, and detail), the horizon is the strongest near-level line a Hough transform finds.
//! Good enough to tell frames of one scene apart, which is what choosing the best of a
//! group asks of it; not a judge of a single picture.

use image::imageops::FilterType;
use image::{GrayImage, RgbImage};
use serde::{Deserialize, Serialize};

use crate::colour::lab;
use crate::fit_to;

/// Bumped when what [`framing`] measures changes: framings kept of another version are
/// measured again.
pub const FRAMING_VERSION: u32 = 1;

/// The long edge lines and detail are measured at.
const EDGE: u32 = 256;
/// The long edge of the saliency map.
const MAP: u32 = 64;

/// How far (0..1 of the frame) a subject may be from a thirds point and still count as on it.
const NEAR_POINT: f32 = 0.10;
/// From a thirds line.
const NEAR_LINE: f32 = 0.06;
/// From the centre.
const NEAR_CENTRE: f32 = 0.10;
/// The outer share of the frame on each side that makes its edges.
const BAND: f32 = 0.08;
/// A near-level line is looked for within this many degrees of level, and one found
/// further off than [`LEVEL_ENDS`] is a diagonal meant as one.
const LINES_WITHIN: f32 = 10.0;
const LEVEL_ENDS: f32 = 10.0;
/// Of the frame's width, the share of edge points a line needs to count as a horizon: a
/// horizon runs across the frame, a line of perspective (a floor, a facade) seldom does.
const LINE_COVERAGE: f32 = 0.6;
/// Of a side of the frame, the share a patch of what stands out runs along to be background.
const EDGE_RUN: f32 = 0.3;

/// How a picture is framed, each part 0 to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Framing {
    /// All of it together: higher is better framed.
    pub score: f32,
    /// How well the subject sits: on a third, or in the middle; balanced when it fills the
    /// frame. 0.5 when no subject stands out.
    pub placement: f32,
    /// Where the subject's centre is, 0 to 1 across and down; none when nothing stands out.
    pub subject: Option<[f32; 2]>,
    /// How much of the subject the edge of the frame cuts off.
    pub cut: f32,
    /// How much busier the edges are than the picture as a whole.
    pub clutter: f32,
    /// Degrees off level of the longest near-level line, where there is one (positive:
    /// falling to the right).
    pub tilt: Option<f32>,
}

impl Framing {
    /// What a level horizon is missing: 0 at level, 1 tilted enough to see, 0 again for a
    /// line steep enough to be meant.
    pub fn tilt_cost(&self) -> f32 {
        let Some(t) = self.tilt else { return 0.0 };
        let a = t.abs();
        if a <= 0.5 {
            0.0
        } else if a < 2.0 {
            (a - 0.5) / 1.5
        } else if a <= 7.0 {
            1.0
        } else {
            ((LEVEL_ENDS - a) / (LEVEL_ENDS - 7.0)).clamp(0.0, 1.0)
        }
    }

    /// What stands out of the framing, in words.
    pub fn notes(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.subject.is_some() && self.placement >= 0.75 && self.cut < 0.5 {
            out.push(WELL_PLACED);
        }
        if self.cut >= 0.5 {
            out.push(CUT_OFF);
        }
        if self.clutter >= 0.5 {
            out.push(BUSY_EDGES);
        }
        if self.tilt_cost() >= 0.5 {
            out.push(TILTED);
        }
        out
    }
}

pub const WELL_PLACED: &str = "well placed";
pub const CUT_OFF: &str = "subject cut off";
pub const BUSY_EDGES: &str = "busy edges";
pub const TILTED: &str = "tilted horizon";

/// A grid of values, row by row.
struct Grid {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

impl Grid {
    fn at(&self, x: usize, y: usize) -> f32 {
        self.v[y * self.w + x]
    }

    fn mean(&self) -> f32 {
        if self.v.is_empty() { 0.0 } else { self.v.iter().sum::<f32>() / self.v.len() as f32 }
    }

    /// Each value the mean of its 3×3 neighbourhood (clamped at the edges).
    fn smooth(&self) -> Grid {
        let (w, h) = (self.w as isize, self.h as isize);
        let mut v = Vec::with_capacity(self.v.len());
        for y in 0..h {
            for x in 0..w {
                let (mut sum, mut n) = (0.0, 0.0);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (xx, yy) = (x + dx, y + dy);
                        if (0..w).contains(&xx) && (0..h).contains(&yy) {
                            sum += self.v[(yy * w + xx) as usize];
                            n += 1.0;
                        }
                    }
                }
                v.push(sum / n);
            }
        }
        Grid { w: self.w, h: self.h, v }
    }

    /// Divided by its 99th percentile and kept to 0..1: a few extreme values do not
    /// flatten the rest. All 0 for a grid with nothing in it.
    fn normalised(mut self) -> Grid {
        let top = nth(&self.v, ((self.v.len().max(1) - 1) as f32 * 0.99) as usize);
        let top = if top > 1e-6 { top } else { self.v.iter().copied().fold(0.0, f32::max) };
        for v in &mut self.v {
            *v = if top > 1e-6 { (*v / top).clamp(0.0, 1.0) } else { 0.0 };
        }
        self
    }

    /// The mean over the outer `share` of each side, and over the rest.
    fn band_mean(&self, share: f32) -> f32 {
        let bx = ((self.w as f32 * share).round() as usize).max(1);
        let by = ((self.h as f32 * share).round() as usize).max(1);
        let (mut sum, mut n) = (0.0, 0.0);
        for y in 0..self.h {
            for x in 0..self.w {
                if x < bx || y < by || x + bx >= self.w || y + by >= self.h {
                    sum += self.at(x, y);
                    n += 1.0;
                }
            }
        }
        if n > 0.0 { sum / n } else { 0.0 }
    }
}

/// The value at place `at` of `values` put in order (the last, past the end); 0 of none.
fn nth(values: &[f32], at: usize) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    let at = at.min(v.len() - 1);
    *v.select_nth_unstable_by(at, f32::total_cmp).1
}

/// Sobel gradients of `gray`, 0 along its border.
fn sobel(gray: &GrayImage) -> (Grid, Grid) {
    let (w, h) = (gray.width() as usize, gray.height() as usize);
    let mut gx = vec![0.0; w * h];
    let mut gy = vec![0.0; w * h];
    let raw = gray.as_raw();
    let px = |x: usize, y: usize| f32::from(raw[y * w + x]);
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let (a, b, c) = (px(x - 1, y - 1), px(x, y - 1), px(x + 1, y - 1));
            let (d, f) = (px(x - 1, y), px(x + 1, y));
            let (g, hh, i) = (px(x - 1, y + 1), px(x, y + 1), px(x + 1, y + 1));
            gx[y * w + x] = (c + 2.0 * f + i) - (a + 2.0 * d + g);
            gy[y * w + x] = (g + 2.0 * hh + i) - (a + 2.0 * b + c);
        }
    }
    (Grid { w, h, v: gx }, Grid { w, h, v: gy })
}

/// `detail` (at the measuring size) pooled into a `w`×`h` grid: each cell the mean of the
/// pixels it covers.
fn pool(detail: &Grid, w: usize, h: usize) -> Grid {
    let mut v = Vec::with_capacity(w * h);
    for my in 0..h {
        let (y0, y1) = (my * detail.h / h, ((my + 1) * detail.h / h).max(my * detail.h / h + 1).min(detail.h));
        for mx in 0..w {
            let (x0, x1) = (mx * detail.w / w, ((mx + 1) * detail.w / w).max(mx * detail.w / w + 1).min(detail.w));
            let (mut sum, mut n) = (0.0, 0.0);
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += detail.at(x, y);
                    n += 1.0;
                }
            }
            v.push(if n > 0.0 { sum / n } else { 0.0 });
        }
    }
    Grid { w, h, v }
}

/// What stands out: colour unlike the picture's mean (Lab, a little blurred) and detail,
/// half each, on a small grid.
fn saliency(image: &RgbImage, magnitude: &Grid) -> Grid {
    let small = fit_to(image, MAP, FilterType::Triangle);
    let (w, h) = (small.width() as usize, small.height() as usize);
    let labs: Vec<[f32; 3]> = small.pixels().map(|p| lab(p.0)).collect();
    let channel = |c: usize| Grid { w, h, v: labs.iter().map(|l| l[c]).collect() }.smooth();
    let (l, a, b) = (channel(0), channel(1), channel(2));
    let (ml, ma, mb) = (l.mean(), a.mean(), b.mean());
    let colour = Grid {
        w,
        h,
        v: (0..w * h).map(|i| ((l.v[i] - ml).powi(2) + (a.v[i] - ma).powi(2) + (b.v[i] - mb).powi(2)).sqrt()).collect(),
    }
    .normalised();
    let detail = pool(magnitude, w, h).normalised();
    Grid { w, h, v: colour.v.iter().zip(&detail.v).map(|(c, d)| 0.5 * c + 0.5 * d).collect() }.smooth()
}

/// The subject: the biggest connected patch of what stands out well above the rest.
struct Subject {
    /// Its centre (weighted by how much it stands out), 0..1 across and down.
    centre: [f32; 2],
    /// Its share of the frame.
    area: f32,
    cut: f32,
}

fn subject(s: &Grid) -> Option<Subject> {
    let n = s.v.len();
    if n == 0 {
        return None;
    }
    let mean = s.mean();
    let sd = (s.v.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n as f32).sqrt();
    if mean < 1e-6 || sd / mean < 0.35 {
        return None;
    }
    let mask: Vec<bool> = s.v.iter().map(|&v| v >= mean + sd).collect();
    // The 4-connected patches of the mask.
    let mut label = vec![usize::MAX; n];
    let mut patches: Vec<Vec<usize>> = Vec::new();
    for start in 0..n {
        if !mask[start] || label[start] != usize::MAX {
            continue;
        }
        let mut patch = Vec::new();
        let mut stack = vec![start];
        label[start] = start;
        while let Some(i) = stack.pop() {
            patch.push(i);
            let (x, y) = (i % s.w, i / s.w);
            let mut near = |j: usize| {
                if mask[j] && label[j] == usize::MAX {
                    label[j] = start;
                    stack.push(j);
                }
            };
            if x > 0 {
                near(i - 1);
            }
            if x + 1 < s.w {
                near(i + 1);
            }
            if y > 0 {
                near(i - s.w);
            }
            if y + 1 < s.h {
                near(i + s.w);
            }
        }
        patches.push(patch);
    }
    // A patch running along a long stretch of the frame's edge is background (a bright sky,
    // a lit wall, a sign at the side), not a subject the edge cuts: the subject is the
    // biggest patch that is not.
    let background = |patch: &[usize]| {
        let along = |on: &dyn Fn(usize, usize) -> bool, by: &dyn Fn(usize, usize) -> usize, of: usize| {
            let mut at: Vec<usize> =
                patch.iter().filter(|&&i| on(i % s.w, i / s.w)).map(|&i| by(i % s.w, i / s.w)).collect();
            at.sort_unstable();
            at.dedup();
            at.len() as f32 / of as f32 >= EDGE_RUN
        };
        along(&|_, y| y < 2, &|x, _| x, s.w)
            || along(&|_, y| y + 2 >= s.h, &|x, _| x, s.w)
            || along(&|x, _| x < 2, &|_, y| y, s.h)
            || along(&|x, _| x + 2 >= s.w, &|_, y| y, s.h)
    };
    let (backgrounds, subjects): (Vec<Vec<usize>>, Vec<Vec<usize>>) = patches.into_iter().partition(|p| background(p));
    let best = subjects.into_iter().max_by_key(Vec::len).unwrap_or_default();
    let area = best.len() as f32 / n as f32;
    let weight = |i: usize| (s.v[i] - mean).max(0.0);
    // What stands out besides the background.
    let all: f32 = (0..n).map(weight).sum::<f32>() - backgrounds.iter().flatten().map(|&i| weight(i)).sum::<f32>();
    let mass: f32 = best.iter().map(|&i| weight(i)).sum();
    if !(0.005..=0.6).contains(&area) || all <= 0.0 || mass < 0.25 * all {
        return None;
    }
    let (mut cx, mut cy) = (0.0, 0.0);
    for &i in &best {
        cx += weight(i) * ((i % s.w) as f32 + 0.5);
        cy += weight(i) * ((i / s.w) as f32 + 0.5);
    }
    let centre = [cx / mass / s.w as f32, cy / mass / s.h as f32];
    // Cut: on each side, how much of the patch lies in the outer two lines of the grid, next
    // to how wide (or tall) the patch is. A subject cut at the bottom is the way a portrait
    // from the waist up is framed: it counts less.
    let cut = if area > 0.35 {
        0.0
    } else {
        let xs = best.iter().map(|&i| i % s.w);
        let ys = best.iter().map(|&i| i / s.w);
        let (x0, x1) = (xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0));
        let (y0, y1) = (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0));
        let (wide, tall) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        let count = |f: &dyn Fn(usize, usize) -> bool| best.iter().filter(|&&i| f(i % s.w, i / s.w)).count() as f32;
        let top = count(&|_, y| y < 2) / (2.0 * wide);
        let bottom = count(&|_, y| y + 2 >= s.h) / (2.0 * wide);
        let left = count(&|x, _| x < 2) / (2.0 * tall);
        let right = count(&|x, _| x + 2 >= s.w) / (2.0 * tall);
        top.max(left).max(right).max(0.4 * bottom).clamp(0.0, 1.0)
    };
    Some(Subject { centre, area, cut })
}

/// How well a subject centred at `c` (0..1) covering `area` of the frame sits.
fn placement(c: [f32; 2], area: f32) -> f32 {
    let [cx, cy] = c;
    let third = |v: f32| (v - 1.0 / 3.0).abs().min((v - 2.0 / 3.0).abs());
    let points = (third(cx).powi(2) + third(cy).powi(2)).sqrt();
    let on_point = (-(points / NEAR_POINT).powi(2)).exp();
    // On a thirds line, away from the frame's edges along it.
    let inner = |v: f32| if (0.15..=0.85).contains(&v) { 1.0 } else { 0.5 };
    let line = |d: f32, along: f32| 0.8 * (-(d / NEAR_LINE).powi(2)).exp() * inner(along);
    let on_line = line(third(cx), cy).max(line(third(cy), cx));
    let centre = ((cx - 0.5).powi(2) + (cy - 0.5).powi(2)).sqrt();
    let on_centre = 0.75 * (-(centre / NEAR_CENTRE).powi(2)).exp();
    let placed = 0.2 + 0.8 * on_point.max(on_line).max(on_centre);
    // A subject that fills the frame sits nowhere in particular: it should be balanced.
    let fill = ((area - 0.25) / 0.25).clamp(0.0, 1.0);
    let balance = 1.0 - 2.0 * (cx - 0.5).abs();
    (1.0 - fill) * placed + fill * balance
}

/// Degrees off level of the longest near-level line, if one spans enough of the picture: a
/// Hough transform over ±[`LINES_WITHIN`]°, of the strong edges (by the gradients `gx` and
/// `gy`, and their `magnitude`) running near level.
fn tilt(gx: &Grid, gy: &Grid, magnitude: &Grid) -> Option<f32> {
    let (w, h) = (gx.w, gx.h);
    if w < 8 || h < 8 {
        return None;
    }
    let magnitude = &magnitude.v;
    let strong = nth(magnitude, (magnitude.len() - 1) * 9 / 10).max(80.0);
    const STEP: f32 = 0.25;
    let steps = (2.0 * LINES_WITHIN / STEP) as usize + 1;
    let angles: Vec<(f32, f32, f32)> = (0..steps)
        .map(|k| {
            let deg = -LINES_WITHIN + k as f32 * STEP;
            let r = deg.to_radians();
            (deg, r.cos(), r.sin())
        })
        .collect();
    let offset = w as f32;
    let span = h + 2 * w + 2;
    let mut votes = vec![0u32; steps * span];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if magnitude[i] < strong || gy.v[i].abs() < 1e-3 {
                continue;
            }
            // The edge runs across its gradient: its slope is -gx/gy.
            let along = (-gx.v[i] / gy.v[i]).atan().to_degrees();
            if along.abs() > LINES_WITHIN + 5.0 {
                continue;
            }
            for (k, &(_, cos, sin)) in angles.iter().enumerate() {
                let rho = y as f32 * cos - x as f32 * sin + offset;
                let bin = rho.round() as usize;
                if bin < span {
                    votes[k * span + bin] += 1;
                }
            }
        }
    }
    let mut best = (0u32, 0usize);
    for k in 0..steps {
        let row = &votes[k * span..(k + 1) * span];
        for b in 1..span - 1 {
            let sum = row[b - 1] + row[b] + row[b + 1];
            if sum > best.0 {
                best = (sum, k);
            }
        }
    }
    // An edge is two pixels thick at the measuring size: a full-width one has twice the
    // frame's width in points.
    (best.0 as f32 / (2.0 * w as f32) >= LINE_COVERAGE).then(|| angles[best.1].0)
}

/// How the picture is framed.
pub fn framing(image: &RgbImage) -> Framing {
    let image = fit_to(image, EDGE, FilterType::Triangle);
    let gray = image::imageops::grayscale(&image);
    let (gx, gy) = sobel(&gray);
    let magnitude = Grid { w: gx.w, h: gx.h, v: gx.v.iter().zip(&gy.v).map(|(x, y)| (x * x + y * y).sqrt()).collect() };
    let s = saliency(&image, &magnitude);
    let found = subject(&s);
    let (placement, cut, centre) = match &found {
        Some(sub) => (placement(sub.centre, sub.area), sub.cut, Some(sub.centre)),
        None => (0.5, 0.0, None),
    };
    // Busy edges: detail and salience in the outer band, next to the whole picture's.
    let overall = magnitude.mean();
    let detail_ratio = if overall < 2.0 { 0.0 } else { magnitude.band_mean(BAND) / overall };
    let s_mean = s.mean();
    let salience_ratio = if s_mean < 1e-6 { 0.0 } else { s.band_mean(BAND) / s_mean };
    let clutter = ((0.5 * detail_ratio + 0.5 * salience_ratio - 1.0) / 0.6).clamp(0.0, 1.0);
    let mut out = Framing { score: 0.0, placement, subject: centre, cut, clutter, tilt: tilt(&gx, &gy, &magnitude) };
    out.score = (0.55 * placement + 0.45 * (1.0 - clutter) - 0.35 * cut - 0.30 * out.tilt_cost()).clamp(0.0, 1.0);
    out
}

#[cfg(test)]
mod tests {
    use image::Rgb;

    use super::*;

    const GREY: Rgb<u8> = Rgb([128, 128, 128]);
    const RED: Rgb<u8> = Rgb([210, 30, 30]);

    /// A `w`×`h` grey picture with a red disc of radius `r` centred at (`cx`, `cy`), given
    /// as shares of the frame so the same scene can be drawn at any size.
    fn disc(w: u32, h: u32, cx: f32, cy: f32, r: f32) -> RgbImage {
        let (cx, cy, r) = (cx * w as f32, cy * h as f32, r * w as f32);
        RgbImage::from_fn(w, h, |x, y| {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            if dx * dx + dy * dy <= r * r { RED } else { GREY }
        })
    }

    /// Light above a line through the middle at `deg` degrees (falling to the right), dark
    /// below, with the edge drawn smooth.
    fn horizon(w: u32, h: u32, deg: f32) -> RgbImage {
        let slope = deg.to_radians().tan();
        RgbImage::from_fn(w, h, |x, y| {
            let line = h as f32 / 2.0 + (x as f32 + 0.5 - w as f32 / 2.0) * slope;
            let below = (y as f32 + 0.5 - line + 0.5).clamp(0.0, 1.0);
            let v = (200.0 * (1.0 - below) + 60.0 * below) as u8;
            Rgb([v, v, v.saturating_add(10)])
        })
    }

    #[test]
    fn a_subject_on_a_third_is_placed_better_than_in_a_corner() {
        let third = framing(&disc(300, 200, 1.0 / 3.0, 1.0 / 3.0, 0.05));
        let corner = framing(&disc(300, 200, 0.14, 0.2, 0.05));
        let [cx, cy] = third.subject.expect("the disc stands out");
        assert!((cx - 1.0 / 3.0).abs() < 0.05 && (cy - 1.0 / 3.0).abs() < 0.05, "{cx} {cy}");
        assert!(third.placement >= corner.placement + 0.2, "{third:?} {corner:?}");
        assert!(third.notes().contains(&WELL_PLACED), "{third:?}");
        assert!(!corner.notes().contains(&WELL_PLACED), "{corner:?}");
        assert!(third.score > corner.score);
    }

    #[test]
    fn a_centred_subject_is_not_punished_hard() {
        let centred = framing(&disc(300, 200, 0.5, 0.5, 0.05));
        assert!(centred.placement >= 0.6, "{centred:?}");
    }

    #[test]
    fn the_edge_cutting_the_subject_counts() {
        let top = framing(&disc(300, 200, 0.5, 0.025, 0.1));
        let inside = framing(&disc(300, 200, 0.5, 0.5, 0.1));
        let bottom = framing(&disc(300, 200, 0.5, 0.975, 0.1));
        assert!(top.cut >= 0.5, "{top:?}");
        assert!(top.notes().contains(&CUT_OFF));
        assert!(inside.cut < 0.1, "{inside:?}");
        // Cut at the bottom is how a portrait is framed: it costs less.
        assert!(bottom.cut < top.cut, "{bottom:?} {top:?}");
    }

    #[test]
    fn a_bright_sky_along_the_top_is_background_not_a_cut_subject() {
        let scene = RgbImage::from_fn(300, 200, |x, y| {
            let (dx, dy) = (x as f32 - 100.0, y as f32 - 120.0);
            if y < 30 {
                Rgb([250, 250, 255])
            } else if dx * dx + dy * dy < 225.0 {
                RED
            } else {
                GREY
            }
        });
        let f = framing(&scene);
        let [cx, cy] = f.subject.expect("the disc is the subject");
        assert!((cx - 1.0 / 3.0).abs() < 0.05 && (cy - 0.6).abs() < 0.05, "{f:?}");
        assert!(f.cut < 0.1, "{f:?}");
    }

    #[test]
    fn busy_edges() {
        let busy = RgbImage::from_fn(300, 200, |x, y| {
            let edge = x < 24 || y < 16 || x >= 276 || y >= 184;
            let (dx, dy) = (x as f32 - 150.0, y as f32 - 100.0);
            if edge {
                if (x / 4 + y / 4) % 2 == 0 { Rgb([240; 3]) } else { Rgb([20; 3]) }
            } else if dx * dx + dy * dy < 400.0 {
                RED
            } else {
                GREY
            }
        });
        let b = framing(&busy);
        assert!(b.clutter >= 0.5, "{b:?}");
        assert!(b.notes().contains(&BUSY_EDGES));
        let plain = framing(&disc(300, 200, 0.5, 0.5, 0.07));
        assert!(plain.clutter < 0.2, "{plain:?}");
        assert!(plain.score > b.score);
    }

    #[test]
    fn a_tilted_horizon_is_found_and_a_diagonal_is_not_one() {
        let tilted = framing(&horizon(300, 200, 3.0));
        let t = tilted.tilt.expect("the line is found");
        assert!((t - 3.0).abs() < 0.5, "{tilted:?}");
        assert!(tilted.notes().contains(&TILTED));
        let level = framing(&horizon(300, 200, 0.0));
        assert!(level.tilt.expect("the line is found").abs() < 0.5, "{level:?}");
        assert!(!level.notes().contains(&TILTED));
        assert!(level.score > tilted.score);
        let steep = framing(&horizon(300, 200, 25.0));
        assert!(!steep.notes().contains(&TILTED), "{steep:?}");
        assert_eq!(steep.tilt_cost(), 0.0);
    }

    #[test]
    fn a_plain_picture_is_measured_safely() {
        let f = framing(&RgbImage::from_pixel(300, 200, GREY));
        assert_eq!(f.subject, None);
        assert_eq!((f.cut, f.clutter, f.tilt), (0.0, 0.0, None));
        assert!(f.score.is_finite() && (0.0..=1.0).contains(&f.score), "{f:?}");
        assert!(f.notes().is_empty());
        // Tiny pictures too.
        let tiny = framing(&RgbImage::from_pixel(3, 2, GREY));
        assert!(tiny.score.is_finite());
    }

    /// With `C1_BURSTS` naming a folder of folders, each the JPEGs of one group:
    /// `cargo test -p c1analysis --release -- --ignored --nocapture bursts`. Prints each
    /// frame's framing and how far apart the frames of a group are, for judging by eye.
    #[test]
    #[ignore = "needs a folder of bursts"]
    fn the_framing_of_the_frames_of_bursts() {
        let bursts = std::path::PathBuf::from(std::env::var("C1_BURSTS").expect("C1_BURSTS"));
        let mut groups: Vec<_> = std::fs::read_dir(&bursts).unwrap().map(|e| e.unwrap().path()).collect();
        groups.sort();
        for group in groups.iter().filter(|g| g.is_dir()) {
            let mut files: Vec<_> = std::fs::read_dir(group).unwrap().map(|e| e.unwrap().path()).collect();
            files.retain(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg")));
            files.sort();
            println!("{}", group.file_name().unwrap().to_string_lossy());
            let mut scores = Vec::new();
            for file in files {
                let started = std::time::Instant::now();
                let image = image::open(&file).unwrap().to_rgb8();
                let f = framing(&image);
                scores.push(f.score);
                let at = f.subject.map_or("-".to_owned(), |[x, y]| format!("{x:.2},{y:.2}"));
                let tilt = f.tilt.map_or("-".to_owned(), |t| format!("{t:+.2}"));
                println!(
                    "  {:<14} score {:.2} placed {:.2} at {at:<9} cut {:.2} clutter {:.2} tilt {tilt:<6} {:?} {} ms",
                    file.file_name().unwrap().to_string_lossy(),
                    f.score,
                    f.placement,
                    f.cut,
                    f.clutter,
                    f.notes(),
                    started.elapsed().as_millis()
                );
            }
            let (lo, hi) = scores.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), &v| (l.min(v), h.max(v)));
            println!("  spread {:.2}", hi - lo);
        }
    }

    #[test]
    fn the_size_of_the_render_does_not_matter() {
        let big = framing(&disc(3000, 2000, 2.0 / 3.0, 1.0 / 3.0, 0.05));
        let small = framing(&disc(300, 200, 2.0 / 3.0, 1.0 / 3.0, 0.05));
        assert!((big.score - small.score).abs() < 0.05, "{big:?} {small:?}");
    }
}
