//! The main colours of a picture: k-means in CIE Lab over a small copy, and a name for
//! each colour that a sidebar can list.

use image::RgbImage;
use image::imageops::FilterType;

use crate::Swatch;

/// Mean Lab chroma below which a picture is black and white (toned prints included).
pub const MONO: f32 = 4.0;
pub const BLACK_AND_WHITE: &str = "black & white";
/// A colour photo with no colour to speak of.
pub const MUTED: &str = "muted";
/// Names that are not colours, for [`crate::Analysis::hues`].
pub const NEUTRAL: [&str; 3] = ["black", "white", "grey"];
/// Every name [`name_of`] gives that is a colour, in the order of [`crate::Analysis::hue_shares`].
pub const HUES: [&str; 9] = ["brown", "red", "orange", "yellow", "green", "teal", "blue", "purple", "pink"];

const K: usize = 5;
const ITERATIONS: usize = 10;

/// sRGB (0..255) → CIE Lab (D65).
fn lab(rgb: [u8; 3]) -> [f32; 3] {
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

/// The main colours, biggest first, and the mean chroma.
pub(crate) fn palette(image: &RgbImage) -> (Vec<Swatch>, f32) {
    let small = crate::fit_to(image, 64, FilterType::Triangle);
    let pixels: Vec<([f32; 3], [u8; 3])> = small.pixels().map(|p| (lab(p.0), p.0)).collect();
    if pixels.is_empty() {
        return (Vec::new(), 0.0);
    }
    let chroma = pixels.iter().map(|(l, _)| (l[1] * l[1] + l[2] * l[2]).sqrt()).sum::<f32>() / pixels.len() as f32;

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
    (swatches, chroma)
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
