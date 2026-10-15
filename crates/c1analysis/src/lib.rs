//! What a photo looks like, measured from a small render of it (Capture One's thumbnail,
//! which carries the photo's edits): its shape, main colours, exposure, sharpness, and
//! a perceptual hash for finding duplicates and bursts.
//!
//! Pure functions over pixels. The library keeps the results and indexes the classes
//! ([`Analysis::shape`], [`Analysis::hues`], [`Analysis::light`], [`Analysis::focus`]);
//! the app runs [`analyse`] in the background.

mod colour;
mod groups;

use image::imageops::FilterType;
use image::{GrayImage, RgbImage};
use serde::{Deserialize, Serialize};

pub use colour::{MONO, name_of};
pub use groups::{Groups, SERIES_MIN, Taken, distance, group};

/// Bumped when what [`analyse`] measures changes: stored results of another version are
/// measured again.
pub const VERSION: u32 = 1;

/// Bumped when the classes made of a measure change (thresholds, colour names): the
/// search index is rebuilt, the photos are not measured again.
pub const CLASSES: u32 = 3;

/// One of a photo's main colours.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Swatch {
    pub rgb: [u8; 3],
    /// Share of the picture, 0..1.
    pub share: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    /// Of the render measured (its shape is the photo's, crop and rotation included).
    pub width: u32,
    pub height: u32,
    /// The main colours, biggest first.
    pub palette: Vec<Swatch>,
    /// Mean colourfulness (CIE Lab chroma); near 0 for black and white.
    pub chroma: f32,
    /// Mean luminance, 0..1.
    pub brightness: f32,
    /// Spread of luminance (standard deviation), 0..0.5.
    pub contrast: f32,
    /// Share of pixels at the very top of the range (blown highlights).
    pub highlights: f32,
    /// Share of pixels at the very bottom (blocked shadows).
    pub shadows: f32,
    /// Detail in the sharpest part of the picture: the variance of the Laplacian in
    /// the best of a 4×4 grid of tiles, so a sharp subject on a soft background counts.
    pub sharpness: f32,
    /// Difference hash: 64 bits, close for pictures that look alike.
    pub hash: u64,
}

/// The longest edge measured. Capture One's thumbnails are about this size already.
const WORK_EDGE: u32 = 512;

pub fn analyse(image: &RgbImage) -> Analysis {
    let image = fit(image, WORK_EDGE);
    let (width, height) = image.dimensions();
    let gray = image::imageops::grayscale(&image);
    let (brightness, contrast, highlights, shadows) = tones(&gray);
    let (palette, chroma) = colour::palette(&image);
    Analysis {
        width,
        height,
        palette,
        chroma,
        brightness,
        contrast,
        highlights,
        shadows,
        sharpness: sharpness(&gray),
        hash: dhash(&gray),
    }
}

fn fit(image: &RgbImage, edge: u32) -> RgbImage {
    fit_to(image, edge, FilterType::Triangle)
}

pub(crate) fn fit_to(image: &RgbImage, edge: u32, filter: FilterType) -> RgbImage {
    let (w, h) = image.dimensions();
    if w.max(h) <= edge {
        return image.clone();
    }
    let scale = f64::from(edge) / f64::from(w.max(h));
    let size = |n: u32| ((f64::from(n) * scale).round() as u32).max(1);
    image::imageops::resize(image, size(w), size(h), filter)
}

/// Mean, standard deviation, and the shares of clipped highlights and shadows.
fn tones(gray: &GrayImage) -> (f32, f32, f32, f32) {
    let n = (gray.width() * gray.height()).max(1) as f64;
    let (mut sum, mut sq, mut high, mut low) = (0f64, 0f64, 0usize, 0usize);
    for p in gray.pixels() {
        let v = f64::from(p.0[0]) / 255.0;
        sum += v;
        sq += v * v;
        high += usize::from(p.0[0] >= 250);
        low += usize::from(p.0[0] <= 5);
    }
    let mean = sum / n;
    let sd = (sq / n - mean * mean).max(0.0).sqrt();
    (mean as f32, sd as f32, (high as f64 / n) as f32, (low as f64 / n) as f32)
}

/// The variance of the Laplacian in the sharpest of a 4×4 grid of tiles.
fn sharpness(gray: &GrayImage) -> f32 {
    let (w, h) = gray.dimensions();
    if w < 8 || h < 8 {
        return 0.0;
    }
    let px = |x: u32, y: u32| f64::from(gray.get_pixel(x, y).0[0]);
    let mut best = 0f64;
    for ty in 0..4 {
        for tx in 0..4 {
            let (x0, x1) = ((tx * w / 4).max(1), ((tx + 1) * w / 4).min(w - 1));
            let (y0, y1) = ((ty * h / 4).max(1), ((ty + 1) * h / 4).min(h - 1));
            let (mut sum, mut sq, mut n) = (0f64, 0f64, 0f64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let lap = px(x - 1, y) + px(x + 1, y) + px(x, y - 1) + px(x, y + 1) - 4.0 * px(x, y);
                    sum += lap;
                    sq += lap * lap;
                    n += 1.0;
                }
            }
            if n > 0.0 {
                let mean = sum / n;
                best = best.max(sq / n - mean * mean);
            }
        }
    }
    best as f32
}

/// Difference hash: the picture as 9×8 greys, one bit per neighbour pair (left brighter
/// than right).
fn dhash(gray: &GrayImage) -> u64 {
    let small = image::imageops::resize(gray, 9, 8, FilterType::Triangle);
    let mut hash = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            hash <<= 1;
            hash |= u64::from(small.get_pixel(x, y).0[0] > small.get_pixel(x + 1, y).0[0]);
        }
    }
    hash
}

/// Shape classes.
pub const LANDSCAPE: &str = "landscape";
pub const PORTRAIT: &str = "portrait";
pub const SQUARE: &str = "square";
pub const PANORAMA: &str = "panorama";

/// Light classes.
pub const DARK: &str = "dark";
pub const MEDIUM: &str = "medium";
pub const BRIGHT: &str = "bright";
pub const CLIPPED: &str = "clipped highlights";
pub const BLOCKED: &str = "blocked shadows";
pub const FLAT: &str = "low contrast";

/// Focus classes.
pub const SHARP: &str = "sharp";
pub const SOFT: &str = "soft";
pub const BLURRY: &str = "blurry";

/// Sharpness below which a photo is soft, and below that blurry: the variance of the
/// Laplacian of a 512 px render, set from the fuji catalog's spread.
pub const SOFT_BELOW: f32 = 700.0;
pub const BLURRY_BELOW: f32 = 300.0;

impl Analysis {
    pub fn shape(&self) -> &'static str {
        let ratio = self.width as f32 / self.height.max(1) as f32;
        if ratio >= 2.0 || ratio <= 0.5 {
            PANORAMA
        } else if ratio > 1.1 {
            LANDSCAPE
        } else if ratio < 1.0 / 1.1 {
            PORTRAIT
        } else {
            SQUARE
        }
    }

    /// The colours the picture is made of: black and white, or the colours (not black,
    /// white or grey) covering a sixth of it or more, else its biggest colour if it shows
    /// at all, else `muted`.
    pub fn hues(&self) -> Vec<&'static str> {
        if self.chroma < MONO {
            return vec![colour::BLACK_AND_WHITE];
        }
        let mut by_name: Vec<(&'static str, f32)> = Vec::new();
        for s in &self.palette {
            let name = name_of(s.rgb);
            if colour::NEUTRAL.contains(&name) {
                continue;
            }
            match by_name.iter_mut().find(|(n, _)| *n == name) {
                Some((_, share)) => *share += s.share,
                None => by_name.push((name, s.share)),
            }
        }
        by_name.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut out: Vec<&'static str> = by_name.iter().filter(|(_, s)| *s >= 1.0 / 6.0).map(|(n, _)| *n).collect();
        if out.is_empty() {
            out.push(by_name.first().filter(|(_, s)| *s >= 0.05).map_or(colour::MUTED, |(n, _)| n));
        }
        out
    }

    /// How light the picture is, and what is lost at either end.
    pub fn light(&self) -> Vec<&'static str> {
        let mut out = vec![if self.brightness < 0.2 {
            DARK
        } else if self.brightness > 0.55 {
            BRIGHT
        } else {
            MEDIUM
        }];
        if self.highlights > 0.03 {
            out.push(CLIPPED);
        }
        if self.shadows > 0.2 {
            out.push(BLOCKED);
        }
        if self.contrast < 0.12 {
            out.push(FLAT);
        }
        out
    }

    pub fn focus(&self) -> &'static str {
        if self.sharpness < BLURRY_BELOW {
            BLURRY
        } else if self.sharpness < SOFT_BELOW {
            SOFT
        } else {
            SHARP
        }
    }
}

#[cfg(test)]
mod tests {
    use image::Rgb;

    use super::*;

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(w, h, Rgb(rgb))
    }

    /// Black and white squares, `cell` pixels wide: full of edges.
    fn checks(w: u32, h: u32, cell: u32) -> RgbImage {
        RgbImage::from_fn(
            w,
            h,
            |x, y| if (x / cell + y / cell).is_multiple_of(2) { Rgb([255; 3]) } else { Rgb([0; 3]) },
        )
    }

    #[test]
    fn shapes() {
        assert_eq!(analyse(&solid(600, 400, [90; 3])).shape(), LANDSCAPE);
        assert_eq!(analyse(&solid(400, 600, [90; 3])).shape(), PORTRAIT);
        assert_eq!(analyse(&solid(500, 500, [90; 3])).shape(), SQUARE);
        assert_eq!(analyse(&solid(1200, 400, [90; 3])).shape(), PANORAMA);
        // Measured at most 512 px long, keeping the shape.
        let a = analyse(&solid(3000, 2000, [90; 3]));
        assert_eq!((a.width, a.height), (512, 341));
    }

    #[test]
    fn colours_and_black_and_white() {
        let blue = analyse(&solid(300, 200, [30, 70, 200]));
        assert_eq!(blue.hues(), ["blue"]);
        assert!(blue.palette[0].share > 0.99);
        // Half red, half green: both count.
        let two = RgbImage::from_fn(300, 200, |x, _| if x < 150 { Rgb([210, 30, 30]) } else { Rgb([40, 160, 50]) });
        let mut hues = analyse(&two).hues();
        hues.sort();
        assert_eq!(hues, ["green", "red"]);
        assert_eq!(analyse(&checks(300, 200, 10)).hues(), ["black & white"]);
    }

    #[test]
    fn light() {
        assert_eq!(analyse(&solid(300, 200, [20; 3])).light()[0], DARK);
        assert_eq!(analyse(&solid(300, 200, [128; 3])).light()[0], MEDIUM);
        let white = analyse(&solid(300, 200, [255; 3]));
        assert_eq!(white.light(), [BRIGHT, CLIPPED, FLAT]);
        assert!(!analyse(&checks(300, 200, 20)).light().contains(&FLAT));
    }

    #[test]
    fn sharp_and_blurry() {
        let sharp = analyse(&checks(512, 340, 4));
        let flat = analyse(&solid(512, 340, [100; 3]));
        let soft = analyse(&image::imageops::blur(&checks(512, 340, 16), 6.0));
        assert_eq!(sharp.focus(), SHARP);
        assert_eq!(flat.focus(), BLURRY);
        assert!(soft.sharpness < sharp.sharpness && soft.sharpness > flat.sharpness);
        // A sharp patch on a plain background still counts as sharp.
        let mut subject = solid(512, 340, [100; 3]);
        image::imageops::replace(&mut subject, &checks(128, 85, 4), 200, 120);
        assert_eq!(analyse(&subject).focus(), SHARP);
    }

    #[test]
    fn alike_pictures_hash_alike() {
        let a = image::imageops::blur(&checks(400, 300, 50), 2.0);
        let smaller = image::imageops::resize(&a, 200, 150, FilterType::Triangle);
        let other = image::imageops::blur(&checks(400, 300, 23), 2.0);
        let (ha, hs, ho) = (analyse(&a).hash, analyse(&smaller).hash, analyse(&other).hash);
        assert!(distance(ha, hs) <= 4, "{}", distance(ha, hs));
        assert!(distance(ha, ho) > 12, "{}", distance(ha, ho));
    }
}
