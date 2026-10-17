//! What a photo looks like, measured from a small render of it (Capture One's thumbnail,
//! which carries the photo's edits): its shape, main colours, exposure, sharpness, and
//! a perceptual hash for finding duplicates and bursts.
//!
//! Pure functions over pixels. The library keeps the results and indexes the classes
//! ([`Analysis::shape`], [`Analysis::hues`], [`Analysis::light`], [`Analysis::focus`]),
//! drawn where [`ClassSettings`] puts the lines; the app runs [`analyse`] in the
//! background.

mod colour;
mod groups;

use image::imageops::FilterType;
use image::{GrayImage, RgbImage};
use serde::{Deserialize, Serialize};

pub use colour::{HUES, MONO, name_of};
pub use groups::{DUPLICATE_MAX, Groups, Taken, distance, group};

/// Bumped when what [`analyse`] measures changes: stored results of another version are
/// measured again.
pub const VERSION: u32 = 1;

/// Bumped when the classes made of a measure change (colour names, what a class means):
/// the search index is rebuilt, the photos are not measured again. Moving a threshold is
/// a [`ClassSettings`] change, not this.
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

/// Where the lines between classes lie: what a photo has to measure to count as a colour,
/// as dark, as soft, as a burst. Kept per library; changing them classes the measured
/// photos again without measuring them. The defaults are what the app started with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClassSettings {
    /// Mean Lab chroma below which a picture is black and white.
    pub mono_chroma: f32,
    /// Share of the picture (0..1) a colour covers to be one of its colours.
    pub colour_share: f32,
    /// Share its biggest colour needs when none reaches `colour_share`; else `muted`.
    pub colour_floor: f32,
    /// Mean luminance (0..1) below which a picture is dark, and above which it is bright.
    pub dark_below: f32,
    pub bright_above: f32,
    /// Share of pixels at the top of the range that makes clipped highlights.
    pub clipped_share: f32,
    /// Share of pixels at the bottom that makes blocked shadows.
    pub blocked_share: f32,
    /// Luminance spread below which a picture has low contrast.
    pub flat_below: f32,
    /// Sharpness (variance of the Laplacian of a 512 px render) below which a photo is
    /// soft, and below that blurry.
    pub soft_below: f32,
    pub blurry_below: f32,
    /// Width over height beyond which a photo is landscape or portrait (else square), and
    /// beyond which it is a panorama.
    pub square_ratio: f32,
    pub panorama_ratio: f32,
    /// Hash bits apart at most for two images taken at the same instant to be duplicates
    /// (0 to [`DUPLICATE_MAX`]).
    pub duplicate_bits: u32,
    /// For a series: hash bits apart at most from the photo before, the longest wait for
    /// the next one, and how many photos make one.
    pub series_bits: u32,
    pub series_gap_ms: i64,
    pub series_min: usize,
}

impl Default for ClassSettings {
    fn default() -> Self {
        Self {
            mono_chroma: colour::MONO,
            colour_share: 1.0 / 6.0,
            colour_floor: 0.05,
            dark_below: 0.2,
            bright_above: 0.55,
            clipped_share: 0.03,
            blocked_share: 0.2,
            flat_below: 0.12,
            // Set from the fuji catalog's spread.
            soft_below: 700.0,
            blurry_below: 300.0,
            square_ratio: 1.1,
            panorama_ratio: 2.0,
            duplicate_bits: 3,
            series_bits: 22,
            series_gap_ms: 4_000,
            series_min: 3,
        }
    }
}

impl ClassSettings {
    /// The settings with every value brought into a range that means something, and the
    /// classes kept in order (blurry below soft, dark below bright, square before panorama).
    pub fn sanitized(&self) -> Self {
        let d = Self::default();
        let fin = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        let unit = |v: f32, fallback: f32| fin(v, fallback).clamp(0.0, 1.0);
        let soft = fin(self.soft_below, d.soft_below).max(0.0);
        let dark = unit(self.dark_below, d.dark_below);
        let square = fin(self.square_ratio, d.square_ratio).max(1.0);
        Self {
            mono_chroma: fin(self.mono_chroma, d.mono_chroma).max(0.0),
            colour_share: unit(self.colour_share, d.colour_share),
            colour_floor: unit(self.colour_floor, d.colour_floor),
            dark_below: dark,
            bright_above: unit(self.bright_above, d.bright_above).max(dark),
            clipped_share: unit(self.clipped_share, d.clipped_share),
            blocked_share: unit(self.blocked_share, d.blocked_share),
            flat_below: unit(self.flat_below, d.flat_below),
            soft_below: soft,
            blurry_below: fin(self.blurry_below, d.blurry_below).clamp(0.0, soft),
            square_ratio: square,
            panorama_ratio: fin(self.panorama_ratio, d.panorama_ratio).max(square),
            duplicate_bits: self.duplicate_bits.min(DUPLICATE_MAX),
            series_bits: self.series_bits.min(64),
            series_gap_ms: self.series_gap_ms.max(0),
            series_min: self.series_min.max(2),
        }
    }
}

impl Analysis {
    pub fn shape(&self, s: &ClassSettings) -> &'static str {
        let ratio = self.width as f32 / self.height.max(1) as f32;
        if ratio >= s.panorama_ratio || ratio <= 1.0 / s.panorama_ratio {
            PANORAMA
        } else if ratio > s.square_ratio {
            LANDSCAPE
        } else if ratio < 1.0 / s.square_ratio {
            PORTRAIT
        } else {
            SQUARE
        }
    }

    /// The colours the picture is made of: black and white, or the colours (not black,
    /// white or grey) covering [`ClassSettings::colour_share`] of it or more, else its
    /// biggest colour if it shows at all, else `muted`.
    pub fn hues(&self, s: &ClassSettings) -> Vec<&'static str> {
        if self.chroma < s.mono_chroma {
            return vec![colour::BLACK_AND_WHITE];
        }
        let mut by_name: Vec<(&'static str, f32)> = Vec::new();
        for swatch in &self.palette {
            let name = name_of(swatch.rgb);
            if colour::NEUTRAL.contains(&name) {
                continue;
            }
            match by_name.iter_mut().find(|(n, _)| *n == name) {
                Some((_, share)) => *share += swatch.share,
                None => by_name.push((name, swatch.share)),
            }
        }
        by_name.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut out: Vec<&'static str> =
            by_name.iter().filter(|(_, share)| *share >= s.colour_share).map(|(n, _)| *n).collect();
        if out.is_empty() {
            out.push(by_name.first().filter(|(_, share)| *share >= s.colour_floor).map_or(colour::MUTED, |(n, _)| n));
        }
        out
    }

    /// The share of the picture (0..1) in each of [`HUES`], in that order: what the colours
    /// are ranked by. Black, white and grey are in none.
    pub fn hue_shares(&self) -> [f32; HUES.len()] {
        let mut out = [0.0; HUES.len()];
        for swatch in &self.palette {
            if let Some(i) = HUES.iter().position(|h| *h == name_of(swatch.rgb)) {
                out[i] += swatch.share;
            }
        }
        out
    }

    /// How light the picture is, and what is lost at either end.
    pub fn light(&self, s: &ClassSettings) -> Vec<&'static str> {
        let mut out = vec![if self.brightness < s.dark_below {
            DARK
        } else if self.brightness > s.bright_above {
            BRIGHT
        } else {
            MEDIUM
        }];
        if self.highlights > s.clipped_share {
            out.push(CLIPPED);
        }
        if self.shadows > s.blocked_share {
            out.push(BLOCKED);
        }
        if self.contrast < s.flat_below {
            out.push(FLAT);
        }
        out
    }

    pub fn focus(&self, s: &ClassSettings) -> &'static str {
        if self.sharpness < s.blurry_below {
            BLURRY
        } else if self.sharpness < s.soft_below {
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

    fn d() -> ClassSettings {
        ClassSettings::default()
    }

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
        assert_eq!(analyse(&solid(600, 400, [90; 3])).shape(&d()), LANDSCAPE);
        assert_eq!(analyse(&solid(400, 600, [90; 3])).shape(&d()), PORTRAIT);
        assert_eq!(analyse(&solid(500, 500, [90; 3])).shape(&d()), SQUARE);
        assert_eq!(analyse(&solid(1200, 400, [90; 3])).shape(&d()), PANORAMA);
        // Measured at most 512 px long, keeping the shape.
        let a = analyse(&solid(3000, 2000, [90; 3]));
        assert_eq!((a.width, a.height), (512, 341));
    }

    #[test]
    fn shapes_follow_the_settings() {
        let a = analyse(&solid(600, 400, [90; 3]));
        assert_eq!(a.shape(&ClassSettings { square_ratio: 1.6, ..d() }), SQUARE);
        assert_eq!(a.shape(&ClassSettings { panorama_ratio: 1.4, ..d() }), PANORAMA);
    }

    #[test]
    fn colours_and_black_and_white() {
        let blue = analyse(&solid(300, 200, [30, 70, 200]));
        assert_eq!(blue.hues(&d()), ["blue"]);
        assert!(blue.palette[0].share > 0.99);
        // Half red, half green: both count.
        let two = RgbImage::from_fn(300, 200, |x, _| if x < 150 { Rgb([210, 30, 30]) } else { Rgb([40, 160, 50]) });
        let mut hues = analyse(&two).hues(&d());
        hues.sort();
        assert_eq!(hues, ["green", "red"]);
        assert_eq!(analyse(&checks(300, 200, 10)).hues(&d()), ["black & white"]);
    }

    #[test]
    fn a_colour_needs_the_share_asked_for() {
        // A third red, two thirds green.
        let two = RgbImage::from_fn(300, 200, |x, _| if x < 100 { Rgb([210, 30, 30]) } else { Rgb([40, 160, 50]) });
        let a = analyse(&two);
        let mut hues = a.hues(&d());
        hues.sort();
        assert_eq!(hues, ["green", "red"]);
        assert_eq!(a.hues(&ClassSettings { colour_share: 0.5, ..d() }), ["green"]);
        // Nothing reaches 90%: the biggest colour stands for the photo if it shows at all.
        assert_eq!(a.hues(&ClassSettings { colour_share: 0.9, ..d() }), ["green"]);
        assert_eq!(a.hues(&ClassSettings { colour_share: 0.9, colour_floor: 0.9, ..d() }), ["muted"]);
        assert_eq!(a.hues(&ClassSettings { mono_chroma: 1000.0, ..d() }), ["black & white"]);
    }

    #[test]
    fn hue_shares_add_up_per_name() {
        let two = RgbImage::from_fn(300, 200, |x, _| if x < 100 { Rgb([210, 30, 30]) } else { Rgb([40, 160, 50]) });
        let shares = analyse(&two).hue_shares();
        let of = |name: &str| shares[HUES.iter().position(|h| *h == name).unwrap()];
        assert!((of("red") - 1.0 / 3.0).abs() < 0.05, "{}", of("red"));
        assert!((of("green") - 2.0 / 3.0).abs() < 0.05, "{}", of("green"));
        assert_eq!(of("blue"), 0.0);
        assert!(analyse(&checks(300, 200, 10)).hue_shares().iter().all(|s| *s == 0.0));
    }

    #[test]
    fn light() {
        assert_eq!(analyse(&solid(300, 200, [20; 3])).light(&d())[0], DARK);
        assert_eq!(analyse(&solid(300, 200, [128; 3])).light(&d())[0], MEDIUM);
        let white = analyse(&solid(300, 200, [255; 3]));
        assert_eq!(white.light(&d()), [BRIGHT, CLIPPED, FLAT]);
        assert!(!analyse(&checks(300, 200, 20)).light(&d()).contains(&FLAT));
        let grey = analyse(&solid(300, 200, [128; 3]));
        assert_eq!(grey.light(&ClassSettings { bright_above: 0.4, ..d() })[0], BRIGHT);
        assert_eq!(grey.light(&ClassSettings { dark_below: 0.6, ..d() })[0], DARK);
        assert!(!white.light(&ClassSettings { clipped_share: 1.0, ..d() }).contains(&CLIPPED));
    }

    #[test]
    fn sharp_and_blurry() {
        let sharp = analyse(&checks(512, 340, 4));
        let flat = analyse(&solid(512, 340, [100; 3]));
        let soft = analyse(&image::imageops::blur(&checks(512, 340, 16), 6.0));
        assert_eq!(sharp.focus(&d()), SHARP);
        assert_eq!(flat.focus(&d()), BLURRY);
        assert!(soft.sharpness < sharp.sharpness && soft.sharpness > flat.sharpness);
        // A sharp patch on a plain background still counts as sharp.
        let mut subject = solid(512, 340, [100; 3]);
        image::imageops::replace(&mut subject, &checks(128, 85, 4), 200, 120);
        assert_eq!(analyse(&subject).focus(&d()), SHARP);
        let strict = ClassSettings { soft_below: sharp.sharpness + 1.0, ..d() };
        assert_eq!(sharp.focus(&strict), SOFT);
    }

    #[test]
    fn settings_are_kept_in_order() {
        let wild = ClassSettings {
            dark_below: 0.7,
            bright_above: 0.3,
            soft_below: 100.0,
            blurry_below: 500.0,
            square_ratio: 0.2,
            panorama_ratio: 0.5,
            duplicate_bits: 40,
            series_min: 0,
            colour_share: f32::NAN,
            ..d()
        };
        let s = wild.sanitized();
        assert!(s.dark_below <= s.bright_above);
        assert!(s.blurry_below <= s.soft_below);
        assert!(s.square_ratio >= 1.0 && s.panorama_ratio >= s.square_ratio);
        assert_eq!((s.duplicate_bits, s.series_min), (DUPLICATE_MAX, 2));
        assert_eq!(s.colour_share, d().colour_share);
        assert_eq!(d().sanitized(), d());
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
