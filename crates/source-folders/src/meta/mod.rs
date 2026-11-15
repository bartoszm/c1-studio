//! What a photo file says about itself: EXIF, embedded XMP and IPTC, and an XMP sidecar
//! beside it. Reading is best effort: a field that cannot be read is left empty, never an
//! error.

pub mod embed;
pub mod exif;
pub mod iptc;
pub mod xmp;

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use c1model::{ColorLabel, Exif, GeoPoint, ImageFormat, Iptc, KeywordPath, Place};
use serde::{Deserialize, Serialize};

use self::xmp::XmpFields;

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
    let mut meta = exif::read(path, kind);
    let Embedded { iptc, xmp, size } = embedded(path, kind);
    apply(&mut meta, iptc);
    apply(&mut meta, xmp);
    if let Some((w, h)) = size {
        (meta.exif.width, meta.exif.height) = (Some(w), Some(h));
    }
    if let Some(xml) = sidecar.and_then(|p| std::fs::read(p).ok()) {
        apply(&mut meta, xmp::parse(&String::from_utf8_lossy(&xml)));
    }
    meta
}

/// What a file holds besides its EXIF.
#[derive(Default)]
struct Embedded {
    iptc: XmpFields,
    xmp: XmpFields,
    /// The pixels stored, from the image's own header where it is at hand (JPEG, PNG):
    /// truer than EXIF, which a PNG seldom has and a resized JPEG may not have updated.
    size: Option<(u32, u32)>,
}

/// The IPTC, the embedded XMP and the stored size of a file.
fn embedded(path: &Path, kind: FileKind) -> Embedded {
    match kind {
        FileKind::Jpeg => jpeg_embedded(path).unwrap_or_default(),
        FileKind::Tiff | FileKind::Raw => {
            let packet = File::open(path).ok().and_then(|f| xmp::packet_from_tiff_reader(&mut BufReader::new(f)));
            Embedded { xmp: packet.map(|p| xmp::parse(&p)).unwrap_or_default(), ..Embedded::default() }
        }
        FileKind::Png => Embedded { size: png_size(path), ..Embedded::default() },
    }
}

fn jpeg_embedded(path: &Path) -> Option<Embedded> {
    let head = embed::read_jpeg_head(path)?;
    let (segments, _) = embed::jpeg_header(&head)?;
    let iptc = segments.iter().filter(|s| s.marker == 0xED).find_map(|s| iptc::parse_app13(&head[s.payload.clone()]));
    let xmp = xmp::packet_from_jpeg(&head).map(|p| xmp::parse(&p));
    Some(Embedded {
        iptc: iptc.unwrap_or_default(),
        xmp: xmp.unwrap_or_default(),
        size: embed::jpeg_size(&head, &segments),
    })
}

/// The pixel size in a PNG's header (IHDR, always its first chunk).
fn png_size(path: &Path) -> Option<(u32, u32)> {
    let mut head = [0u8; 24];
    File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head[..8] != b"\x89PNG\r\n\x1a\n" || &head[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let h = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    (w > 0 && h > 0).then_some((w, h))
}

/// Lays `over` on `meta`: what it says replaces what `meta` has.
fn apply(meta: &mut FileMeta, over: XmpFields) {
    meta.rating = over.rating.or(meta.rating);
    meta.color = over.color.or(meta.color);
    meta.geo = over.geo.or(meta.geo);
    if !over.keywords.is_empty() {
        meta.keywords = over.keywords;
    }
    let (i, o) = (&mut meta.iptc, over.iptc);
    for (field, new) in [
        (&mut i.title, o.title),
        (&mut i.headline, o.headline),
        (&mut i.description, o.description),
        (&mut i.creator, o.creator),
        (&mut i.copyright, o.copyright),
        (&mut i.job_id, o.job_id),
        (&mut i.instructions, o.instructions),
        (&mut i.source, o.source),
    ] {
        replace(field, new);
    }
    let (p, o) = (&mut meta.place, over.place);
    for (field, new) in [
        (&mut p.city, o.city),
        (&mut p.state, o.state),
        (&mut p.country, o.country),
        (&mut p.country_code, o.country_code),
        (&mut p.location, o.location),
    ] {
        replace(field, new);
    }
}

fn replace(field: &mut Option<String>, new: Option<String>) {
    if new.is_some() {
        *field = new;
    }
}

#[cfg(test)]
mod tests {
    use super::embed::tests::{plain_tiff, rich_jpeg};
    use super::*;

    const SIDECAR: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
      <rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="5" xmp:Label="Green"/></rdf:RDF></x:xmpmeta>"#;
    const EMBEDDED: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
      <rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/"
        xmp:Rating="2" xmp:Label="Red"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">From XMP</rdf:li></rdf:Alt></dc:title>
      </rdf:Description></rdf:RDF></x:xmpmeta>"#;

    #[test]
    fn layers_in_order_of_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg = dir.path().join("a.jpg");
        std::fs::write(&jpeg, rich_jpeg(Some(EMBEDDED))).unwrap();
        let sidecar = dir.path().join("a.xmp");
        std::fs::write(&sidecar, SIDECAR).unwrap();

        let meta = read(&jpeg, FileKind::Jpeg, None);
        assert_eq!(meta.exif.camera_model.as_deref(), Some("X-T5"));
        assert!(meta.captured_at.is_some() && meta.geo.is_some());
        // XMP over IPTC over EXIF
        assert_eq!(meta.iptc.title.as_deref(), Some("From XMP"));
        assert_eq!(meta.iptc.headline.as_deref(), Some("A headline"));
        assert_eq!(meta.place.city.as_deref(), Some("Wrocław"));
        assert_eq!(meta.keywords.len(), 2);
        assert_eq!((meta.rating, meta.color), (Some(2), Some(ColorLabel::Red)));
        assert_eq!(meta.orientation, 90);

        // the sidecar over all of them, where it speaks
        let meta = read(&jpeg, FileKind::Jpeg, Some(&sidecar));
        assert_eq!((meta.rating, meta.color), (Some(5), Some(ColorLabel::Green)));
        assert_eq!(meta.iptc.title.as_deref(), Some("From XMP"));
    }

    #[test]
    fn a_dng_is_read_for_its_exif_and_embedded_xmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.dng");
        let tiff = embed::embed_tiff(&exif::tests::sample_exif(true), EMBEDDED).unwrap();
        std::fs::write(&path, tiff).unwrap();
        let meta = read(&path, FileKind::Raw, None);
        assert_eq!(meta.exif.camera_model.as_deref(), Some("X-T5"));
        assert_eq!(meta.rating, Some(2));
        assert_eq!(meta.iptc.title.as_deref(), Some("From XMP"));

        let tif = dir.path().join("b.tif");
        std::fs::write(&tif, embed::embed_tiff(&plain_tiff(), EMBEDDED).unwrap()).unwrap();
        assert_eq!(read(&tif, FileKind::Tiff, None).color, Some(ColorLabel::Red));
    }

    #[test]
    fn a_raw_with_only_a_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("a.nef");
        std::fs::write(&raw, b"not a raw file").unwrap();
        let sidecar = dir.path().join("a.xmp");
        std::fs::write(&sidecar, SIDECAR).unwrap();
        let meta = read(&raw, FileKind::Raw, Some(&sidecar));
        assert_eq!(meta.rating, Some(5));
        assert_eq!(meta.exif, Exif::default());
    }

    #[test]
    fn nothing_readable_is_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.jpg");
        std::fs::write(&junk, b"\xff\xd8\xff\xe1\x00\x10 truncated").unwrap();
        for kind in [FileKind::Jpeg, FileKind::Png, FileKind::Tiff, FileKind::Raw] {
            assert_eq!(read(&junk, kind, Some(&junk)), FileMeta::default());
            assert_eq!(
                read(&dir.path().join("missing"), kind, Some(&dir.path().join("none.xmp"))),
                FileMeta::default()
            );
        }
    }

    #[test]
    fn sizes_come_from_the_image_headers() {
        let dir = tempfile::tempdir().unwrap();
        // The EXIF says 6240 × 4160 (the camera's); the JPEG holds 24 × 16 pixels.
        let jpeg = dir.path().join("a.jpg");
        std::fs::write(&jpeg, rich_jpeg(None)).unwrap();
        let meta = read(&jpeg, FileKind::Jpeg, None);
        assert_eq!((meta.exif.width, meta.exif.height), (Some(24), Some(16)));
        assert_eq!(meta.exif.camera_model.as_deref(), Some("X-T5"));

        let tiff = dir.path().join("a.tif");
        std::fs::write(&tiff, plain_tiff()).unwrap();
        let meta = read(&tiff, FileKind::Tiff, None);
        assert_eq!((meta.exif.width, meta.exif.height), (Some(24), Some(16)));

        // A PNG without EXIF has its size all the same.
        let png = dir.path().join("a.png");
        image::RgbImage::new(30, 50).save(&png).unwrap();
        let meta = read(&png, FileKind::Png, None);
        assert_eq!((meta.exif.width, meta.exif.height), (Some(30), Some(50)));
    }

    #[test]
    fn kinds_by_extension() {
        assert_eq!(FileKind::from_ext("JPG"), Some(FileKind::Jpeg));
        assert_eq!(FileKind::from_ext("RAF"), Some(FileKind::Raw));
        assert_eq!(FileKind::from_ext("txt"), None);
        assert!(
            FileKind::Raw.embeds_xmp("DNG") && !FileKind::Raw.embeds_xmp("raf") && !FileKind::Png.embeds_xmp("png")
        );
    }
}
