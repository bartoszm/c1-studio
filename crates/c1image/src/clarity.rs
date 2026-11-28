//! Local contrast, as Capture One's Clarity and Structure add it: the difference between
//! each pixel's brightness and its blurred neighbourhood, amplified. Clarity works on a
//! broad neighbourhood, Structure on fine detail. A thumbnail of the edits is too small to
//! show either, so a preview coloured from one is flat until this adds them back.

use c1model::LocalContrast;

use crate::RgbImage;

/// Neighbourhood sizes, as fractions of the picture's long side (Gaussian sigma).
const CLARITY_SIGMA: f32 = 0.02;
const STRUCTURE_SIGMA: f32 = 0.003;

/// How much of the detail a setting of 100 adds to a colour photo. Fitted to Capture One
/// exports of six colour photos (`docs/PREVIEW-COLOURS.md`): the preview's local contrast
/// at both sizes comes within about one level (out of 255) of the export's.
const CLARITY_GAIN: f32 = 0.35;
const STRUCTURE_GAIN: f32 = 0.6;

/// The share of those a black-and-white photo takes. Capture One's exports of twelve
/// black-and-white photos (Fujifilm and Ricoh, Clarity 10–31, Structure 30–57) show about
/// half the local contrast the same settings add to colour: at full strength their
/// previews came out too crisp at both sizes, at half they come within a few percent.
const MONOCHROME_SHARE: f32 = 0.5;

/// What `contrast` adds to a black-and-white photo (see `MONOCHROME_SHARE`).
pub fn in_black_and_white(contrast: LocalContrast) -> LocalContrast {
    LocalContrast { clarity: contrast.clarity * MONOCHROME_SHARE, structure: contrast.structure * MONOCHROME_SHARE }
}

/// A change towards white or black is in full until it is this close (out of 1) to the
/// end, then eases off to nothing there, so highlights do not clip.
const SHOULDER: f32 = 0.15;

/// Adds `contrast` to `image` in place. Brightness changes; each pixel's colour offsets
/// (R−Y, B−Y) stay, so greys stay grey.
pub fn apply(image: &mut RgbImage, contrast: LocalContrast) {
    if contrast.is_none() {
        return;
    }
    let (w, h) = (image.width() as usize, image.height() as usize);
    if w < 2 || h < 2 {
        return;
    }
    let long = w.max(h) as f32;
    let luma: Vec<f32> = image
        .pixels()
        .map(|p| (0.2126 * f32::from(p.0[0]) + 0.7152 * f32::from(p.0[1]) + 0.0722 * f32::from(p.0[2])) / 255.0)
        .collect();
    let mut delta = vec![0.0f32; luma.len()];
    for (setting, sigma, gain) in
        [(contrast.clarity, CLARITY_SIGMA, CLARITY_GAIN), (contrast.structure, STRUCTURE_SIGMA, STRUCTURE_GAIN)]
    {
        if setting == 0.0 {
            continue;
        }
        let amount = gain * setting / 100.0;
        let blurred = gaussian(&luma, w, h, sigma * long);
        for ((d, l), b) in delta.iter_mut().zip(&luma).zip(&blurred) {
            *d += amount * (l - b);
        }
    }
    for ((p, d), l) in image.pixels_mut().zip(&delta).zip(&luma) {
        // Room left towards white or black: see `SHOULDER`.
        let room = if *d > 0.0 { 1.0 - l } else { *l };
        let add = d * (room / SHOULDER).min(1.0) * 255.0;
        p.0 = p.0.map(|c| (f32::from(c) + add).round().clamp(0.0, 255.0) as u8);
    }
}

/// A Gaussian blur of `sigma` pixels, approximated by three box blurs each way.
fn gaussian(values: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    // A box of half-width r has variance r(r+1)/3, so three of them r(r+1) = sigma².
    let r = (((4.0 * sigma * sigma + 1.0).sqrt() - 1.0) / 2.0).round().max(1.0) as usize;
    let mut a = values.to_vec();
    let mut b = vec![0.0f32; values.len()];
    for _ in 0..3 {
        box_rows(&a, &mut b, w, h, r);
        box_cols(&b, &mut a, w, h, r);
    }
    a
}

fn box_rows(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    for y in 0..h {
        let row = &src[y * w..][..w];
        let out = &mut dst[y * w..][..w];
        box_line(|i| row[i], |i, v| out[i] = v, w, r);
    }
}

fn box_cols(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    for x in 0..w {
        box_line(|i| src[i * w + x], |i, v| dst[i * w + x] = v, h, r);
    }
}

/// A running mean of half-width `r` along a line of `n`, the ends repeated.
fn box_line(get: impl Fn(usize) -> f32, mut set: impl FnMut(usize, f32), n: usize, r: usize) {
    let at = |i: isize| get(i.clamp(0, n as isize - 1) as usize);
    let r = r as isize;
    let mut sum: f32 = (-r..=r).map(at).sum();
    let width = (2 * r + 1) as f32;
    for i in 0..n as isize {
        set(i as usize, sum / width);
        sum += at(i + r + 1) - at(i - r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge() -> RgbImage {
        RgbImage::from_fn(200, 100, |x, _| image::Rgb(if x < 100 { [80; 3] } else { [160; 3] }))
    }

    #[test]
    fn nothing_to_add_leaves_the_image() {
        let mut image = edge();
        apply(&mut image, LocalContrast::default());
        assert_eq!(image, edge());
    }

    #[test]
    fn contrast_grows_at_an_edge_and_flat_areas_stay() {
        let mut image = edge();
        apply(&mut image, LocalContrast { clarity: 50.0, structure: 50.0 });
        // Darker just before the edge, brighter just after, unchanged far from it.
        assert!(image.get_pixel(97, 50).0[0] < 80);
        assert!(image.get_pixel(102, 50).0[0] > 160);
        assert_eq!(image.get_pixel(0, 50).0[0], 80);
        assert_eq!(image.get_pixel(199, 50).0[0], 160);
    }

    #[test]
    fn grey_stays_grey() {
        let mut image = edge();
        apply(&mut image, LocalContrast { clarity: 80.0, structure: 80.0 });
        assert!(image.pixels().all(|p| p.0[0] == p.0[1] && p.0[1] == p.0[2]));
    }

    #[test]
    fn negative_settings_soften() {
        let mut image = edge();
        apply(&mut image, LocalContrast { clarity: -50.0, structure: 0.0 });
        assert!(image.get_pixel(97, 50).0[0] > 80);
        assert!(image.get_pixel(102, 50).0[0] < 160);
    }
}
