//! The main colours of a picture: k-means in CIE Lab over a small copy, and a name for
//! each colour that a sidebar can list.

use image::RgbImage;
use image::imageops::FilterType;

use crate::{HueStat, Swatch};

/// Mean Lab chroma below which a picture is black and white (toned prints included).
pub const MONO: f32 = 4.0;
/// Tint spread ([`crate::Analysis::tint_spread`]) below which a picture is black and white.
/// In the fuji catalog every photo Capture One renders black and white, toned ones included,
/// is below 1.7; muted colour photos (a red jacket by a grey waterfall, a night street) above 3.
pub const MONO_SPREAD: f32 = 2.5;
pub const BLACK_AND_WHITE: &str = "black & white";
/// A colour photo with no colour to speak of.
pub const MUTED: &str = "muted";
/// Names that are not colours.
pub const NEUTRAL: [&str; 3] = ["black", "white", "grey"];
/// Every name [`name_of`] gives that is a colour, in the order of [`crate::Analysis::hue_shares`].
pub const HUES: [&str; 9] = ["brown", "red", "orange", "yellow", "green", "teal", "blue", "purple", "pink"];

const K: usize = 5;
const ITERATIONS: usize = 10;

/// sRGB (0..255) → CIE Lab (D65).
pub(crate) fn lab(rgb: [u8; 3]) -> [f32; 3] {
    let lin = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(rgb[0]), lin(rgb[1]), lin(rgb[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// Per colour name, on a small copy of the picture: how much of it has that name, the
/// biggest single patch of it, and how vivid those pixels are. A red coat on a grey street
/// is a patch; red pixels scattered through a brown wall are not.
pub(crate) fn hue_stats(image: &RgbImage) -> [HueStat; HUES.len()] {
    let small = crate::fit_to(image, 64, FilterType::Triangle);
    let (w, h) = small.dimensions();
    let n = (w * h) as usize;
    let mut out = [HueStat::default(); HUES.len()];
    if n == 0 {
        return out;
    }
    let name: Vec<Option<usize>> = small.pixels().map(|p| HUES.iter().position(|hue| *hue == name_of(p.0))).collect();
    let chroma: Vec<f32> = small
        .pixels()
        .map(|p| {
            let l = lab(p.0);
            (l[1] * l[1] + l[2] * l[2]).sqrt()
        })
        .collect();
    for named in name.iter().flatten() {
        out[*named].share += 1.0;
    }
    // The biggest 4-connected patch of each name, and how vivid that patch is: a big pale
    // area does not borrow the vividness of a few bright specks elsewhere.
    let mut seen = vec![false; n];
    let mut stack = Vec::new();
    for start in 0..n {
        let Some(i) = name[start] else { continue };
        if seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut size, mut vividness) = (0usize, 0f32);
        while let Some(at) = stack.pop() {
            size += 1;
            vividness += chroma[at];
            let (x, y) = ((at % w as usize) as u32, (at / w as usize) as u32);
            let mut visit = |nx: u32, ny: u32| {
                let next = (ny * w + nx) as usize;
                if !seen[next] && name[next] == Some(i) {
                    seen[next] = true;
                    stack.push(next);
                }
            };
            if x > 0 {
                visit(x - 1, y);
            }
            if x + 1 < w {
                visit(x + 1, y);
            }
            if y > 0 {
                visit(x, y - 1);
            }
            if y + 1 < h {
                visit(x, y + 1);
            }
        }
        if size as f32 > out[i].patch {
            out[i].patch = size as f32;
            out[i].chroma = vividness / size as f32;
        }
    }
    for stat in &mut out {
        stat.share /= n as f32;
        stat.patch /= n as f32;
    }
    out
}

/// What [`hue_stats`] would say of a palette alone (every swatch one patch): for analyses
/// made up in tests.
pub fn stats_from_palette(palette: &[Swatch], chroma: f32) -> [HueStat; HUES.len()] {
    let mut out = [HueStat::default(); HUES.len()];
    for swatch in palette {
        if let Some(i) = HUES.iter().position(|h| *h == name_of(swatch.rgb)) {
            out[i].share += swatch.share;
            out[i].patch = out[i].patch.max(swatch.share);
            out[i].chroma = chroma;
        }
    }
    out
}

/// The main colours, biggest first, the mean chroma, and the tint spread
/// ([`crate::Analysis::tint_spread`]).
pub(crate) fn palette(image: &RgbImage) -> (Vec<Swatch>, f32, f32) {
    let small = crate::fit_to(image, 64, FilterType::Triangle);
    let pixels: Vec<([f32; 3], [u8; 3])> = small.pixels().map(|p| (lab(p.0), p.0)).collect();
    if pixels.is_empty() {
        return (Vec::new(), 0.0, 0.0);
    }
    let chroma = pixels.iter().map(|(l, _)| (l[1] * l[1] + l[2] * l[2]).sqrt()).sum::<f32>() / pixels.len() as f32;
    let spread = tint_spread(pixels.iter().map(|(l, _)| l));

    // Start from pixels spread through the lightness range: the same picture always
    // gives the same palette.
    let mut by_light: Vec<usize> = (0..pixels.len()).collect();
    by_light.sort_by(|&a, &b| pixels[a].0[0].total_cmp(&pixels[b].0[0]));
    let k = K.min(pixels.len());
    let mut centres: Vec<[f32; 3]> = (0..k).map(|i| pixels[by_light[(2 * i + 1) * pixels.len() / (2 * k)]].0).collect();
    let mut owner = vec![0usize; pixels.len()];
    for _ in 0..ITERATIONS {
        for (i, (p, _)) in pixels.iter().enumerate() {
            owner[i] = (0..k).min_by(|&a, &b| dist2(p, &centres[a]).total_cmp(&dist2(p, &centres[b]))).unwrap_or(0);
        }
        let mut sums = vec![([0f32; 3], 0usize); k];
        for (i, (p, _)) in pixels.iter().enumerate() {
            let s = &mut sums[owner[i]];
            for (sum, v) in s.0.iter_mut().zip(p) {
                *sum += v;
            }
            s.1 += 1;
        }
        for (c, (sum, n)) in centres.iter_mut().zip(&sums) {
            if *n > 0 {
                *c = [sum[0] / *n as f32, sum[1] / *n as f32, sum[2] / *n as f32];
            }
        }
    }
    // Each cluster shown as the mean of its pixels in sRGB.
    let mut rgb_sums = vec![([0u64; 3], 0u64); k];
    for (i, (_, rgb)) in pixels.iter().enumerate() {
        let s = &mut rgb_sums[owner[i]];
        for (sum, v) in s.0.iter_mut().zip(rgb) {
            *sum += u64::from(*v);
        }
        s.1 += 1;
    }
    let mut swatches: Vec<Swatch> = rgb_sums
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(sum, n)| Swatch {
            rgb: [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8],
            share: n as f32 / pixels.len() as f32,
        })
        .collect();
    swatches.sort_by(|a, b| b.share.total_cmp(&a.share));
    (swatches, chroma, spread)
}

/// How far the pixels' tints stray from the picture's own: the 95th percentile of each
/// pixel's (a, b) distance from the mean (a, b). A black and white picture, toned or not,
/// has one tint throughout (0–2); a muted colour one keeps faces, clothes and signs that
/// stray from it even when its mean chroma is low.
fn tint_spread<'a>(labs: impl Iterator<Item = &'a [f32; 3]> + Clone) -> f32 {
    let (mut sa, mut sb, mut n) = (0f32, 0f32, 0usize);
    for l in labs.clone() {
        sa += l[1];
        sb += l[2];
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    let (ma, mb) = (sa / n as f32, sb / n as f32);
    let mut d: Vec<f32> = labs.map(|l| ((l[1] - ma).powi(2) + (l[2] - mb).powi(2)).sqrt()).collect();
    let at = (d.len() - 1) * 95 / 100;
    *d.select_nth_unstable_by(at, f32::total_cmp).1
}

fn dist2(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// A plain name for a colour: black, white, grey, brown, or a hue.
pub fn name_of(rgb: [u8; 3]) -> &'static str {
    let [r, g, b] = rgb.map(|c| f32::from(c) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let (v, delta) = (max, max - min);
    let s = if max > 0.0 { delta / max } else { 0.0 };
    if v < 0.16 {
        return "black";
    }
    if s < 0.16 || delta < 0.06 {
        return if v > 0.85 { "white" } else { "grey" };
    }
    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    match hue {
        h if !(15.0..345.0).contains(&h) => {
            if v < 0.45 {
                "brown"
            } else {
                "red"
            }
        }
        h if h < 45.0 => {
            if v < 0.6 {
                "brown"
            } else {
                "orange"
            }
        }
        h if h < 70.0 => {
            if v < 0.5 {
                "brown"
            } else {
                "yellow"
            }
        }
        h if h < 165.0 => "green",
        h if h < 200.0 => "teal",
        h if h < 260.0 => "blue",
        h if h < 295.0 => "purple",
        _ => "pink",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for (rgb, name) in [
            ([10, 10, 12], "black"),
            ([250, 250, 248], "white"),
            ([128, 130, 128], "grey"),
            ([200, 30, 30], "red"),
            ([240, 140, 30], "orange"),
            ([120, 70, 30], "brown"),
            ([230, 210, 40], "yellow"),
            ([50, 160, 60], "green"),
            ([30, 160, 170], "teal"),
            ([40, 80, 210], "blue"),
            ([130, 50, 200], "purple"),
            ([230, 90, 170], "pink"),
        ] {
            assert_eq!(name_of(rgb), name, "{rgb:?}");
        }
    }

    #[test]
    fn grey_has_no_chroma() {
        let l = lab([128, 128, 128]);
        assert!(l[1].abs() < 0.5 && l[2].abs() < 0.5, "{l:?}");
        assert!((lab([255, 255, 255])[0] - 100.0).abs() < 0.5);
    }
}
