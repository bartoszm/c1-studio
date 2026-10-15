//! Display-ready sRGB pixels from an [`ImageRef`], whatever source and tier it came from.
//!
//! Three kinds of input:
//! - JPEG (Capture One's `.cot` thumbnails, JPEG originals): decoded, EXIF orientation
//!   applied unless the [`ImageRef`] says to read it as stored, then framed as it says.
//! - JPEG-XL holding linear camera-space RGB before white balance (Capture One's `.cop`
//!   previews): white balance, highlight scaling and the sRGB curve are applied here,
//!   then the quarter turn and straightening, the colours of the source's own small
//!   render, fitted pixel by pixel, and the crop.
//! - PNG and TIFF originals: decoded like a JPEG.
//! - Camera raws: the full-size JPEG the camera embedded, framed alike. Not a raw
//!   conversion, so no edits show beyond the turn and crop.

mod geometry;
mod jxl;
mod look;
mod lut;
mod raw;

use std::io::Cursor;
use std::path::{Path, PathBuf};

use c1model::{ImageFormat, ImageRef};
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageReader};

pub use geometry::{crop, straighten};
pub use image::RgbImage;
pub use jxl::{Matching, decode_linear, jxl_size, render_preview};
pub use look::Look;
pub use lut::Lut;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("reading {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("decoding {path}: {message}")]
    Decode { path: PathBuf, message: String },
    #[error("{path}: no embedded JPEG preview")]
    NoPreview { path: PathBuf },
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn decode_err(path: &Path, e: impl std::fmt::Display) -> Error {
    Error::Decode { path: path.to_owned(), message: e.to_string() }
}

pub(crate) fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.to_owned(), source }
}

pub fn load(r: &ImageRef) -> Result<RgbImage> {
    let exif = !r.as_stored;
    let image = match r.format {
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::Tiff => {
            jpeg(&std::fs::read(&r.path).map_err(io_err(&r.path))?, &r.path, exif)
        }
        ImageFormat::JpegXl => return Ok(frame(render_preview(r, Matching::Colours)?, r)),
        ImageFormat::Raw => match raw::embedded_jpeg(&r.path)? {
            Some(bytes) => jpeg(&bytes, &r.path, exif),
            None => Err(Error::NoPreview { path: r.path.clone() }),
        },
    }?;
    Ok(frame(straighten(rotate(image, r.rotation), r.straighten), r))
}

/// `image`, turned and straightened as `r` says, cut to `r.crop`.
pub fn frame(image: RgbImage, r: &ImageRef) -> RgbImage {
    match &r.crop {
        Some(c) => crop(&image, c, r.straighten),
        None => image,
    }
}

/// Turns clockwise by `degrees`, a multiple of 90; anything else is left as is.
pub fn rotate(image: RgbImage, degrees: u16) -> RgbImage {
    match degrees % 360 {
        90 => image::imageops::rotate90(&image),
        180 => image::imageops::rotate180(&image),
        270 => image::imageops::rotate270(&image),
        _ => image,
    }
}

/// [`load`], scaled down so that the longer edge is at most `long_edge`.
pub fn load_fitted(r: &ImageRef, long_edge: u32) -> Result<RgbImage> {
    Ok(fit(load(r)?, long_edge))
}

pub fn fit(image: RgbImage, long_edge: u32) -> RgbImage {
    let (w, h) = image.dimensions();
    if long_edge == 0 || w.max(h) <= long_edge {
        return image;
    }
    let scale = f64::from(long_edge) / f64::from(w.max(h));
    let size = |n: u32| ((f64::from(n) * scale).round() as u32).max(1);
    image::imageops::resize(&image, size(w), size(h), FilterType::CatmullRom)
}

/// `image` as a JPEG file's bytes.
pub fn encode_jpeg(image: &RgbImage, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(image)
        .expect("encoding into memory does not fail");
    out
}

pub(crate) fn load_jpeg(path: &Path) -> Result<RgbImage> {
    jpeg(&std::fs::read(path).map_err(io_err(path))?, path, true)
}

/// The full-size JPEG a camera raw embeds (Fuji RAF, TIFF-based raws such as DNG, NEF,
/// CR2, ARW), or `None` if it has none this reader finds.
pub fn embedded_jpeg(path: &Path) -> Result<Option<Vec<u8>>> {
    raw::embedded_jpeg(path)
}

/// Decodes a JPEG (or PNG or TIFF: the format is sniffed), turned as its EXIF orientation
/// says when `exif`.
fn jpeg(bytes: &[u8], path: &Path, exif: bool) -> Result<RgbImage> {
    let mut decoder = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(io_err(path))?
        .into_decoder()
        .map_err(|e| decode_err(path, e))?;
    let orientation = decoder.orientation().map_err(|e| decode_err(path, e))?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| decode_err(path, e))?;
    if exif {
        image.apply_orientation(orientation);
    }
    Ok(image.into_rgb8())
}
