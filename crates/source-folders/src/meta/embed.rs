//! Writing an XMP packet into a file: JPEG (APP1), TIFF and DNG (IFD0 tag 700).

use std::fs::File;
use std::io::Read;
use std::ops::Range;
use std::path::Path;

use super::xmp::{JPEG_HEADER, TIFF_TAG};

/// The most a JPEG segment can hold after its two length bytes.
const MAX_SEGMENT: usize = 0xFFFF - 2;
/// How much of a JPEG is read when looking for its metadata segments, at most.
const MAX_HEAD: usize = 16 << 20;

/// One marker segment of a JPEG (APPn, DQT, ...).
pub(crate) struct Segment {
    pub marker: u8,
    /// From the `0xFF` of the marker to the end of the segment.
    pub range: Range<usize>,
    /// What follows the two length bytes.
    pub payload: Range<usize>,
}

/// The marker segments before a JPEG's image data, and whether `bytes` ended inside them
/// (the rest of the file may hold more). `None` if this is not a JPEG.
pub(crate) fn jpeg_header(bytes: &[u8]) -> Option<(Vec<Segment>, bool)> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut segments = Vec::new();
    let mut at = 2;
    loop {
        let Some(&[first, marker]) = bytes.get(at..at + 2) else { return Some((segments, true)) };
        match marker {
            _ if first != 0xFF => return Some((segments, false)),
            // fill byte
            0xFF => {
                at += 1;
                continue;
            }
            // start of scan, end of image: the image data follows
            0xDA | 0xD9 => return Some((segments, false)),
            // markers without a length
            0x00 | 0x01 | 0xD0..=0xD8 => {
                at += 2;
                continue;
            }
            _ => {}
        }
        let Some(&[hi, lo]) = bytes.get(at + 2..at + 4) else { return Some((segments, true)) };
        let len = usize::from(u16::from_be_bytes([hi, lo]));
        if len < 2 {
            return Some((segments, false));
        }
        let end = at + 2 + len;
        if end > bytes.len() {
            return Some((segments, true));
        }
        segments.push(Segment { marker, range: at..end, payload: at + 4..end });
        at = end;
    }
}

/// The start of a JPEG file, long enough to hold all its marker segments (or as much as
/// the first 16 MB of the file give).
pub(crate) fn read_jpeg_head(path: &Path) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let mut head = Vec::new();
    let mut want = 256 << 10;
    loop {
        let more = want - head.len();
        let read = (&mut file).take(more as u64).read_to_end(&mut head).ok()?;
        let (_, truncated) = jpeg_header(&head)?;
        if !truncated || read < more || want >= MAX_HEAD {
            return Some(head);
        }
        want *= 4;
    }
}

/// Whether `s` is the segment that holds the main XMP packet.
fn is_xmp(bytes: &[u8], s: &Segment) -> bool {
    s.marker == 0xE1 && bytes[s.payload.clone()].starts_with(JPEG_HEADER)
}

/// `bytes`, a JPEG, with `xmp` as its XMP packet: in place of the APP1 segment that held one,
/// or else after the APP0 and EXIF segments at its start. Every other byte stays as it was.
/// Errors for a packet that does not fit a segment (65,533 bytes with the header) or a
/// file that is no JPEG.
pub fn embed_jpeg(bytes: &[u8], xmp: &str) -> Result<Vec<u8>, String> {
    let (segments, truncated) = jpeg_header(bytes).ok_or("not a JPEG file")?;
    if truncated {
        return Err("the JPEG ends inside its headers".to_owned());
    }
    let payload = JPEG_HEADER.len() + xmp.len();
    if payload > MAX_SEGMENT {
        let room = MAX_SEGMENT - JPEG_HEADER.len();
        return Err(format!("the XMP packet is {} bytes; a JPEG segment holds {room}", xmp.len()));
    }
    let mut segment = vec![0xFF, 0xE1];
    segment.extend(((payload + 2) as u16).to_be_bytes());
    segment.extend(JPEG_HEADER);
    segment.extend(xmp.as_bytes());

    let old: Vec<&Segment> = segments.iter().filter(|s| is_xmp(bytes, s)).collect();
    let mut out = Vec::with_capacity(bytes.len() + segment.len());
    if old.is_empty() {
        let leading =
            |s: &&Segment| s.marker == 0xE0 || (s.marker == 0xE1 && bytes[s.payload.clone()].starts_with(b"Exif\0\0"));
        let at = segments.iter().take_while(leading).last().map_or(2, |s| s.range.end);
        out.extend(&bytes[..at]);
        out.extend(&segment);
        out.extend(&bytes[at..]);
    } else {
        // the first packet is replaced; any other copy is dropped
        let mut at = 0;
        for (n, s) in old.iter().enumerate() {
            out.extend(&bytes[at..s.range.start]);
            if n == 0 {
                out.extend(&segment);
            }
            at = s.range.end;
        }
        out.extend(&bytes[at..]);
    }
    Ok(out)
}

/// The byte order (`true` for little endian) and IFD0 offset of a classic TIFF header.
pub(crate) fn tiff_header(header: &[u8]) -> Option<(bool, u32)> {
    let little = match header.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    if get_u16(header.get(2..)?, little) != 42 {
        return None;
    }
    Some((little, get_u32(header.get(4..)?, little)))
}

pub(crate) fn get_u16(bytes: &[u8], little: bool) -> u16 {
    let b = [bytes[0], bytes[1]];
    if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) }
}

pub(crate) fn get_u32(bytes: &[u8], little: bool) -> u32 {
    let b = [bytes[0], bytes[1], bytes[2], bytes[3]];
    if little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) }
}

fn put_u16(value: u16, little: bool) -> [u8; 2] {
    if little { value.to_le_bytes() } else { value.to_be_bytes() }
}

fn put_u32(value: u32, little: bool) -> [u8; 4] {
    if little { value.to_le_bytes() } else { value.to_be_bytes() }
}

/// `bytes`, a TIFF or DNG, with `xmp` as its XMP packet (IFD0 tag 700).
///
/// If the file already has a packet and the new one fits in its place, it is written over it,
/// padded with spaces. Otherwise the packet and a new copy of IFD0 (with the tag added or
/// replaced, entries still sorted) are appended and the header points to that IFD0; the old
/// IFD0 and packet stay as unused bytes, so every offset in the file stays valid. Errors for
/// BigTIFF and for anything that is not a TIFF.
pub fn embed_tiff(bytes: &[u8], xmp: &str) -> Result<Vec<u8>, String> {
    if bytes.get(2..4).is_some_and(|v| get_u16(v, bytes[0] == b'I') == 43) {
        return Err("BigTIFF is not supported".to_owned());
    }
    let (little, ifd0) = tiff_header(bytes).ok_or("not a TIFF file")?;
    let truncated = || "the TIFF ends inside its first IFD".to_owned();
    let ifd0 = ifd0 as usize;
    let count = usize::from(get_u16(bytes.get(ifd0..ifd0 + 2).ok_or_else(truncated)?, little));
    let entries_end = ifd0 + 2 + count * 12;
    let entries = bytes.get(ifd0 + 2..entries_end).ok_or_else(truncated)?;
    let next = bytes.get(entries_end..entries_end + 4).ok_or_else(truncated)?;

    let (old, mut kept): (Vec<&[u8]>, Vec<&[u8]>) =
        entries.chunks_exact(12).partition(|e| get_u16(e, little) == TIFF_TAG);
    if let Some(old) = old.first() {
        let (kind, len, at) =
            (get_u16(&old[2..], little), get_u32(&old[4..], little) as usize, get_u32(&old[8..], little) as usize);
        if matches!(kind, 1 | 2 | 7) && len > 4 && xmp.len() <= len && at + len <= bytes.len() {
            let mut out = bytes.to_vec();
            out[at..at + xmp.len()].copy_from_slice(xmp.as_bytes());
            out[at + xmp.len()..at + len].fill(b' ');
            return Ok(out);
        }
    }

    let mut out = bytes.to_vec();
    align(&mut out);
    let xmp_at = u32::try_from(out.len()).map_err(|_| "the file is too big for a TIFF".to_owned())?;
    let xmp_len = u32::try_from(xmp.len()).map_err(|_| "the XMP packet is too big".to_owned())?;
    out.extend(xmp.as_bytes());
    align(&mut out);

    let mut entry = Vec::with_capacity(12);
    entry.extend(put_u16(TIFF_TAG, little));
    entry.extend(put_u16(1, little)); // BYTE, as Adobe writes it
    entry.extend(put_u32(xmp_len, little));
    entry.extend(put_u32(xmp_at, little));
    kept.push(&entry);
    kept.sort_by_key(|e| get_u16(e, little));

    let new_ifd = u32::try_from(out.len()).map_err(|_| "the file is too big for a TIFF".to_owned())?;
    out.extend(put_u16(kept.len() as u16, little));
    kept.iter().for_each(|e| out.extend(*e));
    out.extend(next);
    out[4..8].copy_from_slice(&put_u32(new_ifd, little));
    Ok(out)
}

/// Pads with zeros to a multiple of four bytes.
fn align(bytes: &mut Vec<u8>) {
    bytes.resize(bytes.len().next_multiple_of(4), 0);
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Cursor;

    use ::exif::{In, Reader, Tag};
    use image::codecs::jpeg::JpegEncoder;
    use image::codecs::tiff::TiffEncoder;
    use image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage};

    use super::*;
    use crate::meta::exif::tests::sample_exif;
    use crate::meta::xmp::{packet_from_jpeg, packet_from_tiff, tests::LIGHTROOM};

    pub(crate) fn pixels() -> RgbImage {
        RgbImage::from_fn(24, 16, |x, y| Rgb([(x * 10) as u8, (y * 15) as u8, 90]))
    }

    /// A JPEG as the `image` crate writes it (JFIF APP0 first).
    pub(crate) fn plain_jpeg() -> Vec<u8> {
        let img = pixels();
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 90).write_image(&img, 24, 16, ExtendedColorType::Rgb8).unwrap();
        out
    }

    pub(crate) fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, marker];
        out.extend(((payload.len() + 2) as u16).to_be_bytes());
        out.extend(payload);
        out
    }

    /// `jpeg` with `extra` segments right after its APP0.
    pub(crate) fn with_segments(jpeg: &[u8], extra: &[Vec<u8>]) -> Vec<u8> {
        let (segments, _) = jpeg_header(jpeg).unwrap();
        let at = segments.iter().take_while(|s| s.marker == 0xE0).last().map_or(2, |s| s.range.end);
        let mut out = jpeg[..at].to_vec();
        extra.iter().for_each(|s| out.extend(s));
        out.extend(&jpeg[at..]);
        out
    }

    /// A JPEG with EXIF (`sample_exif`), IPTC and, if given, an XMP packet.
    pub(crate) fn rich_jpeg(xmp: Option<&str>) -> Vec<u8> {
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend(sample_exif(false));
        let mut extra = vec![segment(0xE1, &exif)];
        if let Some(xmp) = xmp {
            extra.push(segment(0xE1, &[JPEG_HEADER, xmp.as_bytes()].concat()));
        }
        extra.push(segment(0xED, &crate::meta::iptc::tests::sample_app13()));
        with_segments(&plain_jpeg(), &extra)
    }

    pub(crate) fn plain_tiff() -> Vec<u8> {
        let mut out = Vec::new();
        TiffEncoder::new(Cursor::new(&mut out)).write_image(&pixels(), 24, 16, ExtendedColorType::Rgb8).unwrap();
        out
    }

    fn decoded(bytes: &[u8]) -> RgbImage {
        image::load_from_memory(bytes).unwrap().to_rgb8()
    }

    const XMP: &str = "<?xpacket begin=\"\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/><?xpacket end=\"w\"?>";

    #[test]
    fn jpeg_gets_a_segment_after_exif() {
        let original = rich_jpeg(None);
        let out = embed_jpeg(&original, XMP).unwrap();
        assert_eq!(packet_from_jpeg(&out).as_deref(), Some(XMP));
        // only the segment was added, right after the APP0 and EXIF segments
        let (segments, _) = jpeg_header(&original).unwrap();
        let at = segments.iter().take_while(|s| s.marker == 0xE0 || s.marker == 0xE1).last().unwrap().range.end;
        let added = JPEG_HEADER.len() + XMP.len() + 4;
        assert_eq!(out.len(), original.len() + added);
        assert_eq!(&out[..at], &original[..at]);
        assert_eq!(&out[at + added..], &original[at..]);
        // everything still reads
        assert_eq!(decoded(&out), decoded(&original));
        let exif = Reader::new().read_from_container(&mut Cursor::new(&out)).unwrap();
        assert_eq!(exif.get_field(Tag::Model, In::PRIMARY).unwrap().display_value().to_string(), "\"X-T5\"");
    }

    #[test]
    fn jpeg_packet_is_replaced_not_added() {
        let original = rich_jpeg(Some(LIGHTROOM));
        assert_eq!(packet_from_jpeg(&original).as_deref(), Some(LIGHTROOM));
        let out = embed_jpeg(&original, XMP).unwrap();
        assert_eq!(packet_from_jpeg(&out).as_deref(), Some(XMP));
        let (segments, _) = jpeg_header(&out).unwrap();
        assert_eq!(segments.iter().filter(|s| is_xmp(&out, s)).count(), 1);
        // the same place as before, and the rest untouched
        let plain = rich_jpeg(None);
        let again = embed_jpeg(&plain, XMP).unwrap();
        assert_eq!(embed_jpeg(&again, XMP).unwrap(), again);
        let positions = |bytes: &[u8]| jpeg_header(bytes).unwrap().0.iter().map(|s| s.marker).collect::<Vec<_>>();
        assert_eq!(positions(&out), positions(&original));
        assert_eq!(decoded(&out), decoded(&original));
    }

    #[test]
    fn jpeg_without_other_segments_and_limits() {
        // no APP0 at all: right after the SOI
        let bare = [&[0xFF, 0xD8][..], &segment(0xDB, &[0; 3]), &[0xFF, 0xDA, 0, 2, 1, 2, 3, 0xFF, 0xD9]].concat();
        let out = embed_jpeg(&bare, XMP).unwrap();
        assert_eq!(&out[2..4], &[0xFF, 0xE1]);
        assert_eq!(packet_from_jpeg(&out).as_deref(), Some(XMP));

        let room = MAX_SEGMENT - JPEG_HEADER.len();
        assert!(embed_jpeg(&bare, &" ".repeat(room)).is_ok());
        assert!(embed_jpeg(&bare, &" ".repeat(room + 1)).is_err());
        assert!(embed_jpeg(b"GIF89a", XMP).is_err());
        assert!(embed_jpeg(&bare[..4], XMP).is_err());
    }

    #[test]
    fn tiff_packet_is_appended_and_replaced() {
        let original = plain_tiff();
        assert_eq!(packet_from_tiff(&original), None);
        let out = embed_tiff(&original, XMP).unwrap();
        assert_eq!(packet_from_tiff(&out).as_deref(), Some(XMP));
        // the old bytes are all still there, and the TIFF decodes the same
        assert_eq!(&out[8..original.len()], &original[8..]);
        assert_eq!(decoded(&out), decoded(&original));
        assert_eq!(decoded(&out), pixels());
        assert_eq!(get_u32(&out[4..], true) % 4, 0, "the new IFD0 is word aligned");

        // a longer packet is appended again; the packet is still one
        let longer = format!("{XMP}{}", " ".repeat(100));
        let again = embed_tiff(&out, &longer).unwrap();
        assert_eq!(packet_from_tiff(&again).as_deref(), Some(longer.trim_end()));
        assert_eq!(ifd0_tags(&again).iter().filter(|t| **t == TIFF_TAG).count(), 1);
        assert_eq!(decoded(&again), pixels());

        // a shorter one goes over the old, and the file does not grow
        let shorter = embed_tiff(&again, "<x/>12345").unwrap();
        assert_eq!(shorter.len(), again.len());
        assert_eq!(packet_from_tiff(&shorter).as_deref(), Some("<x/>12345"));
        assert_eq!(decoded(&shorter), pixels());
    }

    /// The tags of IFD0, in file order.
    fn ifd0_tags(bytes: &[u8]) -> Vec<u16> {
        let (little, ifd0) = tiff_header(bytes).unwrap();
        let ifd0 = ifd0 as usize;
        let n = usize::from(get_u16(&bytes[ifd0..], little));
        (0..n).map(|i| get_u16(&bytes[ifd0 + 2 + i * 12..], little)).collect()
    }

    #[test]
    fn tiff_keeps_exif_in_either_byte_order() {
        for little in [false, true] {
            let original = sample_exif(little);
            let out = embed_tiff(&original, XMP).unwrap();
            assert_eq!(packet_from_tiff(&out).as_deref(), Some(XMP));
            let tags = ifd0_tags(&out);
            assert!(tags.windows(2).all(|w| w[0] < w[1]), "IFD0 sorted: {tags:?}");
            let (before, after) = (Reader::new().read_raw(original).unwrap(), Reader::new().read_raw(out).unwrap());
            let fields =
                |e: &::exif::Exif| e.fields().map(|f| (f.tag, f.display_value().to_string())).collect::<Vec<_>>();
            let got = fields(&after);
            assert!(fields(&before).iter().all(|f| got.contains(f)), "{got:?}");
            assert!(got.iter().any(|(t, _)| t.number() == TIFF_TAG));
        }
    }

    #[test]
    fn tiff_refusals() {
        assert!(embed_tiff(b"II\x2b\x00\x08\x00\x00\x00\x00\x00\x00\x00", XMP).unwrap_err().contains("BigTIFF"));
        assert!(
            embed_tiff(b"MM\x00\x2b\x00\x08\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00", XMP)
                .unwrap_err()
                .contains("BigTIFF")
        );
        assert!(embed_tiff(b"not a tiff at all", XMP).is_err());
        assert!(embed_tiff(&plain_tiff()[..20], XMP).is_err());
    }
}
