//! Borrowing the look of a small finished render (Capture One's thumbnail) for a large
//! unedited one: black and white if the render is grey, and either its colours (see
//! [`crate::lut`]) or, when the two frames do not line up, its tone curve, by matching
//! the distribution of brightness.

use std::path::Path;

use crate::{Result, RgbImage};

/// Mean |R−G| + |B−G| below this (out of 255) is a black-and-white render; JPEG noise
/// leaves a true grey at 0–0.5, the least colourful colour photos sampled sit above 3.
const GREY_CHROMA: f64 = 2.0;

pub struct Look {
    pub grey: bool,
    /// Cumulative share of pixels at or below each luma level.
    cdf: [f64; 256],
    /// The render itself, as shown (after quarter turns).
    pub render: RgbImage,
}

impl Look {
    pub fn load(path: &Path) -> Result<Self> {
        Ok(Self::of(&crate::load_jpeg(path)?))
    }

    pub fn of(image: &RgbImage) -> Self {
        let mut chroma = 0u64;
        for p in image.pixels() {
            let [r, g, b] = p.0;
            chroma += u64::from(r.abs_diff(g)) + u64::from(b.abs_diff(g));
        }
        let n = (image.width() as usize * image.height() as usize).max(1);
        Self { grey: chroma as f64 / n as f64 <= GREY_CHROMA, cdf: cdf(&histogram(image)), render: image.clone() }
    }

    /// Sets every pixel to its brightness.
    pub fn make_grey(image: &mut RgbImage) {
        for p in image.pixels_mut() {
            p.0 = [luma(p.0); 3];
        }
    }

    /// Remaps brightness so that its distribution matches this look's, keeping each
    /// pixel's colour ratios.
    pub fn apply_tones(&self, image: &mut RgbImage) {
        let source = cdf(&histogram(image));
        let mut map = [0u8; 256];
        let mut to = 0usize;
        for (from, share) in source.iter().enumerate() {
            while to < 255 && self.cdf[to] < *share {
                to += 1;
            }
            map[from] = to as u8;
        }
        for p in image.pixels_mut() {
            let y = luma(p.0);
            let target = map[usize::from(y)];
            if y == 0 {
                p.0 = [target; 3];
            } else {
                let k = f32::from(target) / f32::from(y);
                p.0 = p.0.map(|c| (f32::from(c) * k).round().min(255.0) as u8);
            }
        }
    }
}

/// Rec. 709 weights on the 8-bit sRGB values: a fast stand-in for perceived brightness.
fn luma([r, g, b]: [u8; 3]) -> u8 {
    ((54 * u32::from(r) + 183 * u32::from(g) + 19 * u32::from(b)) >> 8) as u8
}

fn histogram(image: &RgbImage) -> [u64; 256] {
    let mut h = [0u64; 256];
    for p in image.pixels() {
        h[usize::from(luma(p.0))] += 1;
    }
    h
}

fn cdf(histogram: &[u64; 256]) -> [f64; 256] {
    let total = histogram.iter().sum::<u64>().max(1) as f64;
    let mut out = [0.0; 256];
    let mut running = 0u64;
    for (i, n) in histogram.iter().enumerate() {
        running += n;
        out[i] = running as f64 / total;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(f: impl Fn(u32) -> [u8; 3]) -> RgbImage {
        RgbImage::from_fn(256, 4, |x, _| image::Rgb(f(x)))
    }

    #[test]
    fn grey_and_colour() {
        assert!(Look::of(&ramp(|x| [x as u8; 3])).grey);
        assert!(!Look::of(&ramp(|x| [x as u8, (x / 2) as u8, 255 - x as u8])).grey);
    }

    #[test]
    fn tones_follow_the_look() {
        // The look is the ramp squeezed into the upper half; matching moves a full ramp there.
        let look = Look::of(&ramp(|x| [(128 + x / 2) as u8; 3]));
        let mut image = ramp(|x| [x as u8; 3]);
        look.apply_tones(&mut image);
        assert!(image.get_pixel(0, 0).0[0] >= 127);
        assert!(image.get_pixel(255, 0).0[0] >= 254);
        let mid = image.get_pixel(128, 0).0[0];
        assert!((185..=200).contains(&mid), "{mid}");
    }
}
