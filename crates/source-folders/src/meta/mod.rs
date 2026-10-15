//! What a photo file says about itself: EXIF, embedded XMP and IPTC, and an XMP sidecar
//! beside it. Reading is best effort: a field that cannot be read is left empty, never an
//! error.

pub mod embed;
pub mod exif;
pub mod iptc;
pub mod xmp;

use std::path::Path;

use c1model::{ColorLabel, Exif, GeoPoint, ImageFormat, Iptc, KeywordPath, Place};
use serde::{Deserialize, Serialize};

/// The kinds of file the source reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileKind {
    Jpeg,
    Png,
    Tiff,
    /// A camera raw (DNG included); shown through the JPEG it embeds.
    Raw,
}

/// Raw extensions read (lower case). CR3 (ISO-BMFF) is not: its embedded JPEG is not
/// found by `c1image`'s reader.
pub const RAW_EXTENSIONS: [&str; 14] =
    ["raf", "dng", "nef", "nrw", "cr2", "arw", "srf", "sr2", "orf", "rw2", "pef", "srw", "3fr", "iiq"];

impl FileKind {
    /// The kind for a file extension (any case, no dot), or `None` for files that are not
    /// photos this source reads.
    pub fn from_ext(ext: &str) -> Option<Self> {
        let ext = ext.to_ascii_lowercase();
        match ext.as_str() {
            "jpg" | "jpeg" | "jpe" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "tif" | "tiff" => Some(Self::Tiff),
            e if RAW_EXTENSIONS.contains(&e) => Some(Self::Raw),
            _ => None,
        }
    }

    pub fn image_format(self) -> ImageFormat {
        match self {
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Png => ImageFormat::Png,
            Self::Tiff => ImageFormat::Tiff,
            Self::Raw => ImageFormat::Raw,
        }
    }

    /// XMP can be embedded in the file itself (when changing originals is allowed); a
    /// sidecar is written for the others.
    pub fn embeds_xmp(self, ext: &str) -> bool {
        matches!(self, Self::Jpeg | Self::Tiff) || ext.eq_ignore_ascii_case("dng")
    }
}

/// Everything read from a file and its sidecar.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileMeta {
    /// Unix epoch milliseconds. EXIF DateTimeOriginal with its offset; without one, read
    /// as this machine's local time.
    pub captured_at: Option<i64>,
    pub exif: Exif,
    /// EXIF orientation, as clockwise degrees to turn the stored pixels: 0, 90, 180, 270.
    /// Mirrored orientations count as their turn.
    pub orientation: u16,
    pub rating: Option<u8>,
    pub color: Option<ColorLabel>,
    pub keywords: Vec<KeywordPath>,
    pub iptc: Iptc,
    pub place: Place,
    pub geo: Option<GeoPoint>,
}

/// Reads `path` (a file of `kind`) and, if given, its XMP `sidecar`. Where both say
/// something, the sidecar wins (it is where other programs write), then embedded XMP, then
/// IPTC, then EXIF.
pub fn read(path: &Path, kind: FileKind, sidecar: Option<&Path>) -> FileMeta {
    let _ = (path, kind, sidecar);
    FileMeta::default() // WP1
}
