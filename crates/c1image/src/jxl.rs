//! Capture One's `.cop` previews: JPEG-XL holding linear camera-space RGB, before white
//! balance and before any film curve. Without white balance the image is strongly green:
//! a Fuji sensor's green channel is about twice as sensitive as its red.

use std::path::Path;

use c1model::ImageRef;
use jxl_oxide::JxlImage;
use jxl_oxide::color::{EnumColourEncoding, RenderingIntent, TransferFunction};

use crate::look::Look;
use crate::lut::Lut;
use crate::{Error, Result, RgbImage, decode_err};

/// The preview has no film curve, so its own maximum is not white. Scale so that this
/// percentile of all channel values is, and let the brightest half percent clip.
const HIGHLIGHT_PERCENTILE: f64 = 99.5;

/// Every n-th value is enough to place the percentile; 7 is coprime with 3 channels.
const PERCENTILE_STRIDE: usize = 7;

/// Rendered and the look differ in shape by more than this share: the look is from
/// before a crop and says nothing pixel by pixel.
const SHAPE_TOLERANCE: f64 = 0.02;

/// A table whose fitted pairs sit further from it than this (median, out of 255) was fitted
/// to frames that do not line up, such as a perspective fix only the look shows; it would
/// spread wrong colours. Typical fits sit at 4–13.
const MAX_RESIDUAL: f32 = 25.0;

/// How much of `ImageRef::look` a preview takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Matching {
    /// Black and white only.
    None,
    /// Black and white and the tone curve.
    Tones,
    /// Colours, through a table fitted to the look pixel by pixel; the tone curve when
    /// that cannot be fitted.
    Colours,
}

/// A linear preview made displayable, before rotation, taking `matching` of `r.look`.
pub fn render_preview(r: &ImageRef, matching: Matching) -> Result<RgbImage> {
    let path = r.path.as_path();
    // A missing or broken thumbnail only costs the look, not the preview.
    let look = r.look.as_deref().and_then(|p| Look::load(p).ok());
    let monochrome = r.monochrome || look.as_ref().is_some_and(|l| l.grey);
    // A fitted table learns the black-and-white mix as well, so it starts from colour.
    let fitting = matching == Matching::Colours && look.is_some();

    let (width, height, mut rgb) = decode_linear(path)?;
    let px = rgb.as_chunks_mut::<3>().0;
    if let Some(wb) = r.white_balance {
        for p in px.iter_mut() {
            p[0] *= wb[0];
            p[1] *= wb[1];
            p[2] *= wb[2];
        }
    }
    if monochrome && !fitting {
        for p in px.iter_mut() {
            *p = [0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]; 3];
        }
    }
    let ceiling = percentile(&rgb, HIGHLIGHT_PERCENTILE);
    let scale = if ceiling > 1e-6 { 1.0 / ceiling } else { 1.0 };
    let lut = srgb_lut();
    let last = (lut.len() - 1) as f32;
    let bytes = rgb.iter().map(|&v| lut[((v * scale).clamp(0.0, 1.0) * last + 0.5) as usize]).collect();
    let mut image =
        RgbImage::from_raw(width, height, bytes).ok_or_else(|| decode_err(path, "pixel buffer has the wrong size"))?;
    let Some(look) = &look else { return Ok(image) };
    match matching {
        Matching::None => {}
        Matching::Tones => look.apply_tones(&mut image),
        Matching::Colours => {
            match fit_colours(&image, look, r.rotation) {
                Some(table) => table.apply(&mut image),
                None => look.apply_tones(&mut image),
            }
            if monochrome {
                Look::make_grey(&mut image);
            }
        }
    }
    Ok(image)
}

/// The table that turns `image` (unrotated) into `look`, fitted at the look's size;
/// `None` when the two are not the same frame.
fn fit_colours(image: &RgbImage, look: &Look, rotation: u16) -> Option<Lut> {
    let target = crate::rotate(look.render.clone(), (360 - rotation % 360) % 360);
    let (w, h) = target.dimensions();
    let (iw, ih) = image.dimensions();
    let (a, b) = (f64::from(iw) * f64::from(h), f64::from(w) * f64::from(ih));
    if (a - b).abs() > SHAPE_TOLERANCE * a.max(b) {
        return None;
    }
    Lut::fit(&shrink(image, w, h), &target).filter(|table| table.residual <= MAX_RESIDUAL)
}

/// The mean of the pixels whose centres fall in each output pixel: a box filter, which is
/// all that fitting needs and several times faster than a resampling filter.
fn shrink(image: &RgbImage, width: u32, height: u32) -> RgbImage {
    let (iw, ih) = image.dimensions();
    let (w, h) = (width.min(iw).max(1) as usize, height.min(ih).max(1) as usize);
    let column: Vec<usize> = (0..iw as usize).map(|x| x * w / iw as usize).collect();
    let mut sums = vec![[0u32; 4]; w * h];
    for (y, row) in image.as_raw().chunks_exact(iw as usize * 3).enumerate() {
        let out = &mut sums[y * h / ih as usize * w..][..w];
        for (px, &x) in row.as_chunks::<3>().0.iter().zip(&column) {
            let s = &mut out[x];
            s[0] += u32::from(px[0]);
            s[1] += u32::from(px[1]);
            s[2] += u32::from(px[2]);
            s[3] += 1;
        }
    }
    let bytes = sums
        .iter()
        .flat_map(|s| std::array::from_fn::<u8, 3, _>(|c| ((s[c] + s[3] / 2) / s[3].max(1)) as u8))
        .collect();
    RgbImage::from_raw(w as u32, h as u32, bytes).expect("the buffer is sized from the dimensions")
}

/// Width, height and interleaved linear RGB, where 1.0 is the file's intensity target —
/// the values libjxl (and the Python `imagecodecs`) returns.
pub fn decode_linear(path: &Path) -> Result<(u32, u32, Vec<f32>)> {
    let mut image = JxlImage::builder().open(path).map_err(|e| decode_err(path, e))?;
    // Capture One declares a 2 000-nit intensity target. Asked for SDR output, jxl-oxide
    // tone-maps that down to 255 nits (Rec. 2408), which bends the linear data. Asked for
    // PQ it keeps absolute luminance, which is turned back into linear values below.
    image.request_color_encoding(EnumColourEncoding {
        tf: TransferFunction::Pq,
        ..EnumColourEncoding::srgb_linear(RenderingIntent::Relative)
    });
    let target = image.image_header().metadata.tone_mapping.intensity_target;
    // Capture One writes two frames; the last is the full image.
    let Some(last) = image.num_loaded_keyframes().checked_sub(1) else {
        return Err(decode_err(path, "no frames"));
    };
    let render = image.render_frame(last).map_err(|e| decode_err(path, e))?;
    let frame = render.image_all_channels();
    let channels = frame.channels();
    if channels < 3 {
        return Err(decode_err(path, format!("{channels} channel(s), expected RGB")));
    }
    let mut rgb: Vec<f32> = if channels == 3 {
        frame.buf().to_vec()
    } else {
        frame.buf().chunks_exact(channels).flat_map(|px| [px[0], px[1], px[2]]).collect()
    };
    let pq = PqTable::new(target);
    for v in &mut rgb {
        *v = pq.linear(*v);
    }
    let (width, height) = (frame.width() as u32, frame.height() as u32);
    if rgb.len() != width as usize * height as usize * 3 {
        return Err(Error::Decode { path: path.to_owned(), message: "frame size does not match the header".into() });
    }
    Ok((width, height, rgb))
}

/// The PQ curve (SMPTE ST 2084) from signal to linear light, tabulated: evaluating it
/// takes two `powf` per value, too slow for 13 million values per preview.
struct PqTable {
    table: Vec<f32>,
}

impl PqTable {
    const STEPS: usize = 1 << 16;

    /// Linear values come out relative to `intensity_target` nits.
    fn new(intensity_target: f32) -> Self {
        let (m1, m2) = (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0);
        let (c1, c2, c3) = (3424.0 / 4096.0, 2413.0 / 4096.0 * 32.0, 2392.0 / 4096.0 * 32.0);
        let scale = 10_000.0 / f64::from(intensity_target);
        let table = (0..=Self::STEPS)
            .map(|i| {
                let p = (i as f64 / Self::STEPS as f64).powf(1.0 / m2);
                (scale * ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1)) as f32
            })
            .collect();
        Self { table }
    }

    fn linear(&self, signal: f32) -> f32 {
        let x = signal.clamp(0.0, 1.0) * Self::STEPS as f32;
        let i = (x as usize).min(Self::STEPS - 1);
        let f = x - i as f32;
        self.table[i] + (self.table[i + 1] - self.table[i]) * f
    }
}

/// Approximate: computed over a strided sample, which moves it by far less than one
/// output level.
fn percentile(values: &[f32], p: f64) -> f32 {
    let mut sample: Vec<f32> = values.iter().step_by(PERCENTILE_STRIDE).copied().collect();
    if sample.is_empty() {
        return 0.0;
    }
    let k = ((p / 100.0) * (sample.len() - 1) as f64).round() as usize;
    *sample.select_nth_unstable_by(k, f32::total_cmp).1
}

/// Linear [0, 1] in 65 536 steps → 8-bit sRGB. Fine enough that shadows do not band.
fn srgb_lut() -> Vec<u8> {
    let n = 1 << 16;
    (0..n)
        .map(|i| {
            let v = i as f64 / (n - 1) as f64;
            let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
            (s * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_curve_ends_and_midpoint() {
        let lut = srgb_lut();
        assert_eq!(lut[0], 0);
        assert_eq!(*lut.last().unwrap(), 255);
        // 18% grey is about 118 in sRGB.
        assert_eq!(lut[(0.18 * 65535.0) as usize], 118);
    }

    #[test]
    fn pq_matches_the_formula() {
        let pq = PqTable::new(2000.0);
        assert_eq!(pq.linear(0.0), 0.0);
        // 100 nits is PQ 0.5081 (ITU-R BT.2100 reference value).
        assert!((pq.linear(0.508_078) * 2000.0 - 100.0).abs() < 0.05, "{}", pq.linear(0.508_078) * 2000.0);
        // 10 000 nits, the top of the curve.
        assert!((pq.linear(1.0) * 2000.0 - 10_000.0).abs() < 0.5);
    }

    #[test]
    fn percentile_of_a_ramp() {
        let ramp: Vec<f32> = (0..70_000).map(|i| i as f32).collect();
        let p = percentile(&ramp, 99.5);
        assert!((p - 69_650.0).abs() < 50.0, "{p}");
    }
}
