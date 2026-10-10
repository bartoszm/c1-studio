//! The JPEG a camera embeds in its raw file: full size in a Fuji RAF, the largest of the
//! IFD previews in TIFF-based raws (DNG, CR2, NEF, ARW).

use std::cmp::Reverse;
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::{Result, io_err};

/// Anything bigger is not a preview.
const MAX_JPEG: u32 = 64 << 20;
/// Smaller is a thumbnail, not worth showing at screen size.
const MIN_JPEG: u32 = 4096;
/// IFDs sit near the start of the file; this much covers every camera seen so far.
const HEAD: u64 = 1 << 20;

pub(crate) fn embedded_jpeg(path: &Path) -> Result<Option<Vec<u8>>> {
    let mut file = File::open(path).map_err(io_err(path))?;
    let mut head = Vec::new();
    (&mut file).take(HEAD).read_to_end(&mut head).map_err(io_err(path))?;
    let candidates = if head.starts_with(b"FUJIFILM") { raf(&head) } else { tiff(&head) };
    for (offset, len) in candidates {
        let mut bytes = vec![0; len as usize];
        if file.seek(SeekFrom::Start(u64::from(offset))).is_err() || file.read_exact(&mut bytes).is_err() {
            continue;
        }
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) && !lossless(&bytes) {
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

/// A RAF stores the offset and length of its JPEG as big-endian `u32`s at byte 84.
fn raf(head: &[u8]) -> Vec<(u32, u32)> {
    let Some(b) = head.get(84..92) else { return vec![] };
    let offset = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let len = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    if (1..MAX_JPEG).contains(&len) { vec![(offset, len)] } else { vec![] }
}

fn type_size(kind: u16) -> usize {
    match kind {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => 0,
    }
}

/// Every (offset, length) of an image an IFD points at, largest first.
fn tiff(head: &[u8]) -> Vec<(u32, u32)> {
    let le = match head.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return vec![],
    };
    let u16_at = |at: usize| {
        head.get(at..at + 2)
            .map(|b| if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let u32_at = |at: usize| {
        head.get(at..at + 4).map(|b| {
            let a = [b[0], b[1], b[2], b[3]];
            if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) }
        })
    };

    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut queue: Vec<u32> = u32_at(4).into_iter().collect();
    while let Some(ifd) = queue.pop() {
        let ifd = ifd as usize;
        if ifd == 0 || !seen.insert(ifd) {
            continue;
        }
        let Some(count) = u16_at(ifd) else { continue };
        let (mut jpeg, mut jpeg_len, mut strip, mut strip_len) = (None, None, None, None);
        for i in 0..usize::from(count) {
            let entry = ifd + 2 + i * 12;
            let (Some(tag), Some(kind), Some(n)) = (u16_at(entry), u16_at(entry + 2), u32_at(entry + 4)) else { break };
            // Values that fit in four bytes sit in the entry itself; others are pointed to.
            let data = if type_size(kind) * n as usize > 4 {
                match u32_at(entry + 8) {
                    Some(pointer) => pointer as usize,
                    None => continue,
                }
            } else {
                entry + 8
            };
            let value = |k: usize| match kind {
                3 => u16_at(data + 2 * k).map(u32::from),
                4 | 13 => u32_at(data + 4 * k),
                _ => None,
            };
            match tag {
                0x0201 => jpeg = value(0),
                0x0202 => jpeg_len = value(0),
                // A preview stored as strips is one strip in every raw seen so far.
                0x0111 => strip = value(0),
                0x0117 => strip_len = value(0),
                0x014A => queue.extend((0..n as usize).filter_map(value)),
                _ => {}
            }
        }
        for (start, len) in [(jpeg, jpeg_len), (strip, strip_len)] {
            if let (Some(start), Some(len)) = (start, len)
                && (MIN_JPEG..MAX_JPEG).contains(&len)
            {
                found.push((start, len));
            }
        }
        if let Some(next) = u32_at(ifd + 2 + usize::from(count) * 12) {
            queue.push(next);
        }
    }
    found.sort_by_key(|&(start, len)| (Reverse(len), start));
    found.dedup();
    found
}

/// True for lossless JPEG (SOF3 and kin): how DNG and CR2 store raw sensor data, which
/// starts like a JPEG but is not a picture.
fn lossless(jpeg: &[u8]) -> bool {
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF {
        let marker = jpeg[at + 1];
        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            return matches!(marker, 0xC3 | 0xC7 | 0xCB | 0xCF);
        }
        if marker == 0xDA {
            break;
        }
        at += 2 + usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real JPEG of a few kilobytes: noise does not compress.
    fn sample_jpeg() -> Vec<u8> {
        let mut state = 1u32;
        let image = image::RgbImage::from_fn(96, 96, |_, _| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let b = (state >> 16).to_le_bytes();
            image::Rgb([b[0], b[1], b[0] ^ b[1]])
        });
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95).encode_image(&image).unwrap();
        assert!(out.len() as u32 > MIN_JPEG);
        out
    }

    fn write(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, bytes).unwrap();
        file
    }

    #[test]
    fn finds_the_jpeg_in_a_raf() {
        let jpeg = sample_jpeg();
        let mut raf = b"FUJIFILMCCD-RAW 0201FF383501".to_vec();
        raf.resize(160, 0);
        raf[84..88].copy_from_slice(&160u32.to_be_bytes());
        raf[88..92].copy_from_slice(&(jpeg.len() as u32).to_be_bytes());
        raf.extend_from_slice(&jpeg);
        assert_eq!(embedded_jpeg(write(&raf).path()).unwrap(), Some(jpeg));
    }

    #[test]
    fn finds_the_largest_jpeg_in_a_tiff_sub_ifd() {
        let jpeg = sample_jpeg();
        // Little-endian TIFF: IFD0 at 8 holds a SubIFDs pointer to IFD1 at 26, which
        // points at the JPEG placed at 56.
        let mut t = b"II*\x00".to_vec();
        t.extend_from_slice(&8u32.to_le_bytes());
        let entry = |tag: u16, kind: u16, value: u32| {
            let mut e = tag.to_le_bytes().to_vec();
            e.extend_from_slice(&kind.to_le_bytes());
            e.extend_from_slice(&1u32.to_le_bytes());
            e.extend_from_slice(&value.to_le_bytes());
            e
        };
        t.extend_from_slice(&1u16.to_le_bytes());
        t.extend(entry(0x014A, 4, 26));
        t.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(t.len(), 26);
        t.extend_from_slice(&2u16.to_le_bytes());
        t.extend(entry(0x0201, 4, 56));
        t.extend(entry(0x0202, 4, jpeg.len() as u32));
        t.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(t.len(), 56);
        t.extend_from_slice(&jpeg);
        assert_eq!(embedded_jpeg(write(&t).path()).unwrap(), Some(jpeg.clone()));

        // Big-endian files work the same way; anything else has no preview.
        assert_eq!(tiff(b"MM\x00*\x00\x00\x00\x00"), vec![]);
        assert_eq!(embedded_jpeg(write(b"not a raw file at all").path()).unwrap(), None);
    }

    #[test]
    fn lossless_jpeg_is_not_a_preview() {
        assert!(lossless(&[0xFF, 0xD8, 0xFF, 0xC3, 0, 2]));
        assert!(!lossless(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 2]));
        assert!(!lossless(&sample_jpeg()));
    }
}
