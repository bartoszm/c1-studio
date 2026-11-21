//! Lining a render up with another render of the same frame before their colours are
//! paired.
//!
//! Capture One's thumbnail is framed a little unlike the preview C1 Studio renders. Lens
//! distortion and perspective corrections that only the thumbnail has move parts of the
//! picture by a few thumbnail pixels, sometimes more than ten. A colour table fitted
//! to pairs from slightly different spots learns a flatter curve than the real one,
//! because least squares pulls towards the mean: shadows come out too light and
//! highlights too dark. So the render is first warped onto the thumbnail.
//!
//! How: the shift of the frame as a whole is found first, at quarter size. Then blocks of
//! the thumbnail are found in the render near that shift by normalised cross-correlation,
//! at half size and then at full size, to a fraction of a pixel. A smooth warp, quadratic
//! in x and y, is fitted to their displacements. Blocks that disagree with it (repeating
//! patterns, local edits) are dropped, and the render is resampled through it.

use crate::RgbImage;

/// Side of a block, in pixels of the target.
const BLOCK: usize = 32;
/// Between the corners of neighbouring blocks.
const SPACING: usize = 28;
/// Search reach for the whole frame's shift at quarter size (±48 px at full size).
const GLOBAL_REACH: i32 = 12;
/// Search reach for each block around the whole frame's shift at half size (±20 px at
/// full size), and around that at full size. A block whose best match lies at the edge
/// of its reach may lie beyond it, and is not trusted.
const LOCAL_REACH: i32 = 10;
const FINE_REACH: i32 = 2;
/// A warp that moves any part of the frame further than this (target pixels) has
/// swung out where no block holds it; the two are paired as they are instead.
const MAX_SHIFT: f32 = 60.0;
/// A block whose best match correlates less than this is not trusted.
const MIN_CORRELATION: f32 = 0.6;
/// A block whose grey levels vary less than this (standard deviation, out of 255) is
/// too flat to place.
const MIN_TEXTURE: f32 = 3.0;
/// Fewer trusted blocks than this cannot pin a warp down.
const MIN_BLOCKS: usize = 8;
/// After fitting, blocks further than these (pixels) from the warp are dropped and the
/// warp is fitted again.
const OUTLIER_STEPS: [f64; 2] = [3.0, 1.5];

/// `source` resampled onto `target`'s grid, and which of its pixels came from inside
/// `source` (row by row).
pub struct Aligned {
    pub image: RgbImage,
    pub valid: Vec<bool>,
    /// The largest displacement the warp makes anywhere in the frame, in target pixels.
    pub largest_shift: f32,
    warp: Warp,
}

impl Aligned {
    /// Another image of the source's size (such as the source with other colours),
    /// warped alike; `None` for an image of another size.
    pub fn resample(&self, image: &RgbImage) -> Option<RgbImage> {
        let (w, h) = image.dimensions();
        (w as usize == self.warp.w && h as usize == self.warp.h).then(|| self.warp.apply(image).image)
    }
}

/// `source`, already at `target`'s size, warped so that each pixel shows the spot the
/// target's pixel shows. `None` when too few parts of the two can be matched (a flat or
/// very dark frame, or two different pictures), or when the warp would move parts of the
/// frame implausibly far; pair them as they are then.
pub fn align(source: &RgbImage, target: &RgbImage) -> Option<Aligned> {
    if source.dimensions() != target.dimensions() {
        return None;
    }
    let (s, t) = (Grey::of(source), Grey::of(target));
    let (w, h) = (t.w, t.h);
    if w < BLOCK + 2 * FINE_REACH as usize || h < BLOCK + 2 * FINE_REACH as usize {
        return None;
    }
    let (s2, t2) = (s.half(), t.half());
    let global = global_shift(&s2.half(), &t2.half());
    let around = (2 * global.0, 2 * global.1);
    let origins: Vec<(usize, usize)> = (0..=h - BLOCK)
        .step_by(SPACING)
        .flat_map(|y0| (0..=w - BLOCK).step_by(SPACING).map(move |x0| (x0, y0)))
        .collect();
    let place = |&(x0, y0): &(usize, usize)| -> Option<Match> {
        let coarse = t2.block(x0 / 2, y0 / 2, BLOCK / 2)?;
        let (cx, cy, _) = best(&coarse, &s2, around, LOCAL_REACH)?;
        if (cx - around.0).abs() == LOCAL_REACH || (cy - around.1).abs() == LOCAL_REACH {
            return None;
        }
        let fine = t.block(x0, y0, BLOCK)?;
        let (fx, fy, ncc) = best(&fine, &s, (2 * cx, 2 * cy), FINE_REACH)?;
        if ncc < MIN_CORRELATION {
            return None;
        }
        let (sx, sy) = subpixel(&fine, &s, fx, fy, ncc);
        let centre = ((x0 + BLOCK / 2) as f64, (y0 + BLOCK / 2) as f64);
        Some(Match { centre, shift: (f64::from(fx) + sx, f64::from(fy) + sy), weight: f64::from(ncc) })
    };
    // The blocks are placed independently, a share of them per thread.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let share = origins.len().div_ceil(threads).max(1);
    let blocks: Vec<Match> = std::thread::scope(|scope| {
        let placing: Vec<_> = origins
            .chunks(share)
            .map(|part| scope.spawn(move || part.iter().filter_map(place).collect::<Vec<_>>()))
            .collect();
        placing.into_iter().flat_map(|h| h.join().expect("a block search does not panic")).collect()
    });
    let warp = Warp::fit(&blocks, w, h)?;
    let Resampled { image, valid, largest_shift } = warp.apply(source);
    (largest_shift <= MAX_SHIFT).then_some(Aligned { image, valid, largest_shift, warp })
}

/// The shift of the frame as a whole, at the size of `source` and `target`: where the
/// middle of the target, as large a square as the reach leaves room for, best matches.
/// No shift when that square is too flat to place.
fn global_shift(source: &Grey, target: &Grey) -> (i32, i32) {
    let reach = GLOBAL_REACH as usize;
    let n = target.w.min(target.h).saturating_sub(2 * reach);
    if n < BLOCK / 4 {
        return (0, 0);
    }
    let (x0, y0) = ((target.w - n) / 2, (target.h - n) / 2);
    target.block(x0, y0, n).and_then(|b| best(&b, source, (0, 0), GLOBAL_REACH)).map_or((0, 0), |(x, y, _)| (x, y))
}

struct Resampled {
    image: RgbImage,
    valid: Vec<bool>,
    largest_shift: f32,
}

struct Match {
    centre: (f64, f64),
    shift: (f64, f64),
    weight: f64,
}

/// Grey levels, 0–255.
struct Grey {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

/// A block of the target with its mean taken out, ready to correlate.
struct Block {
    x0: usize,
    y0: usize,
    n: usize,
    /// Pixel minus the block's mean, row by row.
    centred: Vec<f32>,
    /// Square root of the sum of squares of `centred`.
    norm: f32,
}

impl Grey {
    fn of(image: &RgbImage) -> Self {
        let v = image
            .pixels()
            .map(|p| 0.2126 * f32::from(p.0[0]) + 0.7152 * f32::from(p.0[1]) + 0.0722 * f32::from(p.0[2]))
            .collect();
        Self { w: image.width() as usize, h: image.height() as usize, v }
    }

    fn half(&self) -> Self {
        let (w, h) = (self.w / 2, self.h / 2);
        let mut v = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let at = |dx, dy| self.v[(2 * y + dy) * self.w + 2 * x + dx];
                v.push((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4.0);
            }
        }
        Self { w, h, v }
    }

    /// The block at `(x0, y0)`, `n` on a side, or `None` if it is too flat to place.
    fn block(&self, x0: usize, y0: usize, n: usize) -> Option<Block> {
        if x0 + n > self.w || y0 + n > self.h {
            return None;
        }
        let mut values = Vec::with_capacity(n * n);
        for y in y0..y0 + n {
            values.extend_from_slice(&self.v[y * self.w + x0..][..n]);
        }
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        let centred: Vec<f32> = values.iter().map(|v| v - mean).collect();
        let norm = centred.iter().map(|v| v * v).sum::<f32>().sqrt();
        (norm / (n as f32) >= MIN_TEXTURE).then_some(Block { x0, y0, n, centred, norm })
    }
}

/// Normalised cross-correlation of `block` with `source` shifted by `(dx, dy)`; `None`
/// when the shifted block leaves the source.
fn correlate(block: &Block, source: &Grey, dx: i32, dy: i32) -> Option<f32> {
    let (x, y) = (block.x0 as i64 + i64::from(dx), block.y0 as i64 + i64::from(dy));
    let n = block.n;
    if x < 0 || y < 0 || x as usize + n > source.w || y as usize + n > source.h {
        return None;
    }
    let (x, y) = (x as usize, y as usize);
    let (mut sum, mut squares, mut cross) = (0.0f32, 0.0f32, 0.0f32);
    for row in 0..n {
        let s = &source.v[(y + row) * source.w + x..][..n];
        let t = &block.centred[row * n..][..n];
        for (a, b) in s.iter().zip(t) {
            sum += a;
            squares += a * a;
            cross += a * b;
        }
    }
    let spread = (squares - sum * sum / (n * n) as f32).max(0.0).sqrt();
    // `cross` needs no mean taken out of the source: the block's centred values sum to 0.
    Some(if spread * block.norm > 1e-6 { cross / (spread * block.norm) } else { 0.0 })
}

/// The shift within `reach` of `around` that correlates best, and its correlation.
fn best(block: &Block, source: &Grey, around: (i32, i32), reach: i32) -> Option<(i32, i32, f32)> {
    let mut found: Option<(i32, i32, f32)> = None;
    for dy in around.1 - reach..=around.1 + reach {
        for dx in around.0 - reach..=around.0 + reach {
            if let Some(c) = correlate(block, source, dx, dy)
                && found.is_none_or(|(.., b)| c > b)
            {
                found = Some((dx, dy, c));
            }
        }
    }
    found
}

/// The fraction of a pixel to add to the best shift, from a parabola through it and its
/// neighbours on each axis.
fn subpixel(block: &Block, source: &Grey, dx: i32, dy: i32, peak: f32) -> (f64, f64) {
    let offset = |before: Option<f32>, after: Option<f32>| match (before, after) {
        (Some(a), Some(b)) => {
            let curve = a - 2.0 * peak + b;
            if curve < -1e-6 { f64::from(((a - b) / (2.0 * curve)).clamp(-0.5, 0.5)) } else { 0.0 }
        }
        _ => 0.0,
    };
    (
        offset(correlate(block, source, dx - 1, dy), correlate(block, source, dx + 1, dy)),
        offset(correlate(block, source, dx, dy - 1), correlate(block, source, dx, dy + 1)),
    )
}

/// A displacement over the frame, quadratic in x and y.
struct Warp {
    /// Coefficients of `[1, u, v, u², uv, v²]` for x and y, where u and v run from −½
    /// to ½ across the frame.
    x: [f64; 6],
    y: [f64; 6],
    w: usize,
    h: usize,
}

impl Warp {
    fn fit(blocks: &[Match], w: usize, h: usize) -> Option<Self> {
        let mut kept: Vec<&Match> = blocks.iter().collect();
        let mut warp = Self::solve(&kept, w, h)?;
        for limit in OUTLIER_STEPS {
            kept.retain(|m| {
                let (dx, dy) = warp.at(m.centre.0, m.centre.1);
                (dx - m.shift.0).hypot(dy - m.shift.1) <= limit
            });
            warp = Self::solve(&kept, w, h)?;
        }
        Some(warp)
    }

    fn solve(blocks: &[&Match], w: usize, h: usize) -> Option<Self> {
        if blocks.len() < MIN_BLOCKS {
            return None;
        }
        let mut ata = [[0.0f64; 6]; 6];
        let mut atb = [[0.0f64; 2]; 6];
        let mut mass = 0.0;
        for m in blocks {
            let b = basis(m.centre.0 / w as f64 - 0.5, m.centre.1 / h as f64 - 0.5);
            mass += m.weight;
            for i in 0..6 {
                for j in 0..6 {
                    ata[i][j] += m.weight * b[i] * b[j];
                }
                atb[i][0] += m.weight * b[i] * m.shift.0;
                atb[i][1] += m.weight * b[i] * m.shift.1;
            }
        }
        // Blocks bunched in one part of the frame say little about the curvature far
        // from them: a little ridge on the quadratic terms keeps the warp from swinging
        // out there.
        for (i, row) in ata.iter_mut().enumerate().skip(3) {
            row[i] += 1e-3 * mass;
        }
        let c = solve6(ata, atb)?;
        Some(Self { x: std::array::from_fn(|i| c[i][0]), y: std::array::from_fn(|i| c[i][1]), w, h })
    }

    /// The displacement at pixel `(x, y)` of the target.
    fn at(&self, x: f64, y: f64) -> (f64, f64) {
        let b = basis(x / self.w as f64 - 0.5, y / self.h as f64 - 0.5);
        let dot = |c: &[f64; 6]| c.iter().zip(&b).map(|(c, b)| c * b).sum::<f64>();
        (dot(&self.x), dot(&self.y))
    }

    fn apply(&self, source: &RgbImage) -> Resampled {
        let (w, h) = (self.w, self.h);
        let mut bytes = Vec::with_capacity(w * h * 3);
        let mut valid = Vec::with_capacity(w * h);
        let mut largest = 0.0f64;
        for y in 0..h {
            for x in 0..w {
                // Pixel centres: a block's centre is where its pixels' centres average.
                let (dx, dy) = self.at(x as f64 + 0.5, y as f64 + 0.5);
                largest = largest.max(dx.hypot(dy));
                let (sx, sy) = (x as f64 + dx, y as f64 + dy);
                if sx < 0.0 || sy < 0.0 || sx > (w - 1) as f64 || sy > (h - 1) as f64 {
                    bytes.extend([0, 0, 0]);
                    valid.push(false);
                    continue;
                }
                let (x0, y0) = (sx as u32, sy as u32);
                let (x1, y1) = ((x0 + 1).min(w as u32 - 1), (y0 + 1).min(h as u32 - 1));
                let (fx, fy) = ((sx - f64::from(x0)) as f32, (sy - f64::from(y0)) as f32);
                let p = |x, y| source.get_pixel(x, y).0;
                let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
                for k in 0..3 {
                    let top = f32::from(a[k]) * (1.0 - fx) + f32::from(b[k]) * fx;
                    let bottom = f32::from(c[k]) * (1.0 - fx) + f32::from(d[k]) * fx;
                    bytes.push((top * (1.0 - fy) + bottom * fy + 0.5) as u8);
                }
                valid.push(true);
            }
        }
        let image = RgbImage::from_raw(w as u32, h as u32, bytes).expect("the buffer is sized from the dimensions");
        Resampled { image, valid, largest_shift: largest as f32 }
    }
}

fn basis(u: f64, v: f64) -> [f64; 6] {
    [1.0, u, v, u * u, u * v, v * v]
}

/// Gaussian elimination with partial pivoting, for two right-hand sides; `None` if the
/// system is singular.
fn solve6(mut a: [[f64; 6]; 6], mut b: [[f64; 2]; 6]) -> Option<[[f64; 2]; 6]> {
    for col in 0..6 {
        let pivot = (col..6).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        a.swap(col, pivot);
        b.swap(col, pivot);
        let p = a[col][col];
        if p.abs() < 1e-12 {
            return None;
        }
        let (pa, pb) = (a[col], b[col]);
        for row in (0..6).filter(|&r| r != col) {
            let f = a[row][col] / p;
            for (v, q) in a[row].iter_mut().zip(pa) {
                *v -= f * q;
            }
            for (v, q) in b[row].iter_mut().zip(pb) {
                *v -= f * q;
            }
        }
    }
    Some(std::array::from_fn(|i| [b[i][0] / a[i][i], b[i][1] / a[i][i]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A textured picture: overlapping waves and a few hard edges, nothing that repeats
    /// within a block's search reach.
    fn scene(w: u32, h: u32, warp: impl Fn(f64, f64) -> (f64, f64)) -> RgbImage {
        RgbImage::from_fn(w, h, |x, y| {
            let (x, y) = warp(f64::from(x), f64::from(y));
            let v = 128.0
                + 50.0 * (x * 0.11).sin() * (y * 0.07).cos()
                + 40.0 * ((x + 2.0 * y) * 0.031).sin()
                + if (x / 37.0).floor() as i64 % 3 == (y / 29.0).floor() as i64 % 2 { 30.0 } else { -30.0 };
            let v = v.clamp(0.0, 255.0) as u8;
            image::Rgb([v, v / 2 + 40, 255 - v])
        })
    }

    fn mean_error(a: &RgbImage, b: &RgbImage, valid: &[bool]) -> f64 {
        let (mut sum, mut n) = (0u64, 0u64);
        for ((p, q), &ok) in a.pixels().zip(b.pixels()).zip(valid) {
            if ok {
                sum += (0..3).map(|c| u64::from(p.0[c].abs_diff(q.0[c]))).sum::<u64>();
                n += 3;
            }
        }
        sum as f64 / n.max(1) as f64
    }

    #[test]
    fn finds_a_shift_and_a_zoom() {
        let target = scene(450, 300, |x, y| (x, y));
        // The source shows each spot 6 px right and 4 px down, and 2% larger from the
        // centre.
        let source = scene(450, 300, |x, y| ((x - 225.0) / 1.02 + 225.0 - 6.0, (y - 150.0) / 1.02 + 150.0 - 4.0));
        let before = mean_error(&source, &target, &vec![true; 450 * 300]);
        let aligned = align(&source, &target).expect("aligned");
        let after = mean_error(&aligned.image, &target, &aligned.valid);
        assert!(after < before / 4.0, "{before:.1} → {after:.1}");
        assert!(aligned.largest_shift > 6.0, "{}", aligned.largest_shift);
    }

    #[test]
    fn finds_a_shift_beyond_a_blocks_own_reach() {
        // 36 px: more than a block searches around the frame's shift, found as the frame's.
        let target = scene(450, 300, |x, y| (x, y));
        let source = scene(450, 300, |x, y| (x - 36.0, y + 9.0));
        let aligned = align(&source, &target).expect("aligned");
        assert!((aligned.largest_shift - 37.1).abs() < 1.0, "{}", aligned.largest_shift);
        assert!(mean_error(&aligned.image, &target, &aligned.valid) < 2.0);
    }

    #[test]
    fn resampling_needs_the_source_size() {
        let target = scene(300, 200, |x, y| (x, y));
        let aligned = align(&target, &target).expect("aligned");
        assert!(aligned.resample(&target).is_some());
        assert!(aligned.resample(&RgbImage::new(299, 200)).is_none());
    }

    #[test]
    fn a_flat_frame_is_not_aligned() {
        let flat = RgbImage::from_pixel(450, 300, image::Rgb([90, 90, 90]));
        assert!(align(&flat, &flat).is_none());
    }

    #[test]
    fn an_aligned_pair_stays_put() {
        let target = scene(300, 200, |x, y| (x, y));
        let aligned = align(&target, &target).expect("aligned");
        assert!(aligned.largest_shift < 0.3, "{}", aligned.largest_shift);
        assert!(mean_error(&aligned.image, &target, &aligned.valid) < 0.6);
    }
}
