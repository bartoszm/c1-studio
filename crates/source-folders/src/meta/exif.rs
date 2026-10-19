//! EXIF (and TIFF IFD0) through `kamadak-exif`: date, camera, lens, exposure, size,
//! orientation and GPS. RAF keeps its EXIF in the JPEG it embeds.

use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::Path;

use ::exif::{Exif as ExifData, In, Reader, Tag, Value};
use c1model::{Exif, GeoOrigin, GeoPoint};
use chrono::{FixedOffset, Local, NaiveDateTime, TimeZone};

use super::{FileKind, FileMeta};

/// How much of a TIFF-based raw is read for its EXIF; the IFDs sit near the start.
const RAW_HEAD: u64 = 2 << 20;

/// The EXIF fields of `path` (a file of `kind`), in a [`FileMeta`] with everything else empty.
/// Unreadable files give an empty one.
pub fn read(path: &Path, kind: FileKind) -> FileMeta {
    match kind {
        FileKind::Raw => read_raw(path),
        _ => file_exif(path).map(|e| from_exif(&e)).unwrap_or_default(),
    }
}

/// The EXIF of an image file the `exif` crate can open as a container.
fn file_exif(path: &Path) -> Option<ExifData> {
    let file = File::open(path).ok()?;
    Reader::new().read_from_container(&mut BufReader::new(file)).ok()
}

/// A raw: the EXIF in its own IFDs, else (always for a RAF) in the JPEG it embeds.
fn read_raw(path: &Path) -> FileMeta {
    let is_raf = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("raf"));
    let own = if is_raf { None } else { tiff_head_exif(path).map(|e| from_exif(&e)) };
    if own.as_ref().is_some_and(is_useful) {
        return own.unwrap_or_default();
    }
    let embedded = c1image::embedded_jpeg(path)
        .ok()
        .flatten()
        .and_then(|jpeg| Reader::new().read_from_container(&mut Cursor::new(jpeg)).ok().map(|e| from_exif(&e)));
    embedded.filter(is_useful).or(own).unwrap_or_default()
}

/// Whether anything worth having was found.
fn is_useful(meta: &FileMeta) -> bool {
    meta.captured_at.is_some() || meta.exif.camera_model.is_some() || meta.exif.camera_make.is_some()
}

/// The EXIF of a TIFF-based file, from its first megabytes; fields past them are lost.
fn tiff_head_exif(path: &Path) -> Option<ExifData> {
    let mut head = Vec::new();
    File::open(path).ok()?.take(RAW_HEAD).read_to_end(&mut head).ok()?;
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    match reader.read_raw(head) {
        Ok(exif) => Some(exif),
        Err(err) => err.distill_partial_result(|_| {}).ok(),
    }
}

/// The metadata `exif` holds.
fn from_exif(exif: &ExifData) -> FileMeta {
    FileMeta {
        captured_at: captured_at(exif),
        exif: Exif {
            camera_make: ascii(exif, Tag::Make),
            camera_model: ascii(exif, Tag::Model),
            lens: ascii(exif, Tag::LensModel),
            iso: uint(exif, Tag::PhotographicSensitivity).or_else(|| uint(exif, Tag::ISOSpeed)),
            aperture: rational(exif, Tag::FNumber),
            shutter: rational(exif, Tag::ExposureTime),
            focal: rational(exif, Tag::FocalLength),
            width: uint(exif, Tag::PixelXDimension).or_else(|| uint(exif, Tag::ImageWidth)),
            height: uint(exif, Tag::PixelYDimension).or_else(|| uint(exif, Tag::ImageLength)),
        },
        orientation: orientation(uint(exif, Tag::Orientation).unwrap_or(1)),
        geo: gps(exif),
        ..FileMeta::default()
    }
}

/// The text of an ASCII field, trimmed; `None` if absent or blank.
fn ascii(exif: &ExifData, tag: Tag) -> Option<String> {
    let Value::Ascii(values) = &exif.get_field(tag, In::PRIMARY)?.value else { return None };
    let text = String::from_utf8_lossy(values.first()?);
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    (!text.is_empty()).then(|| text.to_owned())
}

/// The first integer of a field.
fn uint(exif: &ExifData, tag: Tag) -> Option<u32> {
    exif.get_field(tag, In::PRIMARY)?.value.get_uint(0)
}

/// The first rational of a field as a number; `None` for a zero denominator.
fn rational(exif: &ExifData, tag: Tag) -> Option<f64> {
    rationals(exif, tag)?.first().copied()
}

/// All rationals of a field, as numbers; `None` if any has a zero denominator.
fn rationals(exif: &ExifData, tag: Tag) -> Option<Vec<f64>> {
    let Value::Rational(values) = &exif.get_field(tag, In::PRIMARY)?.value else { return None };
    if values.iter().any(|r| r.denom == 0) {
        return None;
    }
    Some(values.iter().map(|r| r.to_f64()).collect())
}

/// Clockwise degrees for an EXIF orientation value; mirrored ones count as their turn.
fn orientation(value: u32) -> u16 {
    match value {
        3 | 4 => 180,
        5 | 6 => 90,
        7 | 8 => 270,
        _ => 0,
    }
}

/// Capture time in epoch milliseconds: the original date, else digitized, else modified.
fn captured_at(exif: &ExifData) -> Option<i64> {
    let sub = |tag| ascii(exif, tag);
    [
        (Tag::DateTimeOriginal, Tag::SubSecTimeOriginal, Some(Tag::OffsetTimeOriginal)),
        (Tag::DateTimeDigitized, Tag::SubSecTimeDigitized, Some(Tag::OffsetTimeDigitized)),
        (Tag::DateTime, Tag::SubSecTime, Some(Tag::OffsetTime)),
    ]
    .into_iter()
    .find_map(|(date, subsec, offset)| {
        let date = ascii(exif, date)?;
        parse_time(&date, sub(subsec).as_deref(), offset.and_then(sub).as_deref())
    })
}

/// `2024:05:01 13:02:03`, its fraction digits (`"45"` is 450 ms) and its `+02:00` offset
/// (this machine's local time without one) as epoch milliseconds.
pub fn parse_time(date: &str, subsec: Option<&str>, offset: Option<&str>) -> Option<i64> {
    let naive = NaiveDateTime::parse_from_str(date.trim(), "%Y:%m:%d %H:%M:%S").ok()?;
    let fixed = offset.and_then(parse_offset);
    let seconds = match fixed {
        Some(offset) => offset.from_local_datetime(&naive).single()?.timestamp(),
        None => Local.from_local_datetime(&naive).earliest()?.timestamp(),
    };
    let millis = subsec.map(fraction_millis).unwrap_or(0);
    Some(seconds * 1000 + millis)
}

/// The first three digits of a fraction of a second, as milliseconds.
fn fraction_millis(digits: &str) -> i64 {
    let digits: String = digits.trim().chars().take_while(char::is_ascii_digit).take(3).collect();
    if digits.is_empty() {
        return 0;
    }
    let value: i64 = digits.parse().unwrap_or(0);
    value * 10_i64.pow(3 - digits.len() as u32)
}

/// `+02:00` or `-0530`.
fn parse_offset(text: &str) -> Option<FixedOffset> {
    let text = text.trim();
    let sign = match text.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let digits: String = text[1..].chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 4 {
        return None;
    }
    let hours: i32 = digits[..2].parse().ok()?;
    let minutes: i32 = digits[2..].parse().ok()?;
    FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))
}

/// The position the GPS fields give; nothing for no fix (0, 0) or out-of-range values.
fn gps(exif: &ExifData) -> Option<GeoPoint> {
    let lat = coordinate(exif, Tag::GPSLatitude, Tag::GPSLatitudeRef, b'S')?;
    let lon = coordinate(exif, Tag::GPSLongitude, Tag::GPSLongitudeRef, b'W')?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) || (lat == 0.0 && lon == 0.0) {
        return None;
    }
    Some(GeoPoint { lat, lon, alt: altitude(exif), origin: GeoOrigin::Exif })
}

/// Degrees, minutes and seconds with their reference letter as signed decimal degrees.
fn coordinate(exif: &ExifData, tag: Tag, reference: Tag, negative: u8) -> Option<f64> {
    let parts = rationals(exif, tag)?;
    let [degrees, minutes, seconds] = parts[..] else { return None };
    let value = degrees + minutes / 60.0 + seconds / 3600.0;
    let south_or_west =
        ascii(exif, reference).is_some_and(|r| r.bytes().next().map(|b| b.to_ascii_uppercase()) == Some(negative));
    Some(if south_or_west { -value } else { value })
}

/// Metres above sea level; negative when the reference says below.
fn altitude(exif: &ExifData) -> Option<f64> {
    let metres = rational(exif, Tag::GPSAltitude)?;
    let below = uint(exif, Tag::GPSAltitudeRef) == Some(1);
    Some(if below { -metres } else { metres })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ::exif::experimental::Writer;
    use ::exif::{Field, Rational};

    fn field(tag: Tag, value: Value) -> Field {
        Field { tag, ifd_num: In::PRIMARY, value }
    }

    fn text(s: &str) -> Value {
        Value::Ascii(vec![s.as_bytes().to_vec()])
    }

    fn rat(num: u32, denom: u32) -> Rational {
        Rational { num, denom }
    }

    /// A TIFF-structured EXIF block with a typical set of fields, little endian when `little`.
    pub(crate) fn sample_exif(little: bool) -> Vec<u8> {
        let fields = vec![
            field(Tag::Make, text("FUJIFILM ")),
            field(Tag::Model, text("X-T5")),
            field(Tag::Orientation, Value::Short(vec![6])),
            field(Tag::DateTimeOriginal, text("2024:05:01 13:02:03")),
            field(Tag::SubSecTimeOriginal, text("45")),
            field(Tag::OffsetTimeOriginal, text("+02:00")),
            field(Tag::ExposureTime, Value::Rational(vec![rat(1, 250)])),
            field(Tag::FNumber, Value::Rational(vec![rat(28, 10)])),
            field(Tag::PhotographicSensitivity, Value::Short(vec![400])),
            field(Tag::FocalLength, Value::Rational(vec![rat(35, 1)])),
            field(Tag::LensModel, text("XF35mmF1.4 R")),
            field(Tag::PixelXDimension, Value::Long(vec![6240])),
            field(Tag::PixelYDimension, Value::Long(vec![4160])),
            field(Tag::GPSLatitudeRef, text("N")),
            field(Tag::GPSLatitude, Value::Rational(vec![rat(50, 1), rat(2, 1), rat(1560, 100)])),
            field(Tag::GPSLongitudeRef, text("W")),
            field(Tag::GPSLongitude, Value::Rational(vec![rat(17, 1), rat(1, 1), rat(0, 1)])),
            field(Tag::GPSAltitudeRef, Value::Byte(vec![1])),
            field(Tag::GPSAltitude, Value::Rational(vec![rat(123, 1)])),
        ];
        let mut writer = Writer::new();
        for f in &fields {
            writer.push_field(f);
        }
        let mut out = Cursor::new(Vec::new());
        writer.write(&mut out, little).unwrap();
        out.into_inner()
    }

    fn parse(tiff: Vec<u8>) -> FileMeta {
        from_exif(&Reader::new().read_raw(tiff).unwrap())
    }

    #[test]
    fn reads_the_fields() {
        for little in [false, true] {
            let meta = parse(sample_exif(little));
            assert_eq!(meta.exif.camera_make.as_deref(), Some("FUJIFILM"));
            assert_eq!(meta.exif.camera_model.as_deref(), Some("X-T5"));
            assert_eq!(meta.exif.lens.as_deref(), Some("XF35mmF1.4 R"));
            assert_eq!(meta.exif.iso, Some(400));
            assert_eq!(meta.exif.aperture, Some(2.8));
            assert_eq!(meta.exif.shutter, Some(0.004));
            assert_eq!(meta.exif.focal, Some(35.0));
            assert_eq!((meta.exif.width, meta.exif.height), (Some(6240), Some(4160)));
            assert_eq!(meta.orientation, 90);
            // 13:02:03.045 at +02:00 is 11:02:03.045 UTC
            assert_eq!(meta.captured_at, Some(1_714_561_323_450));
            let geo = meta.geo.unwrap();
            assert!((geo.lat - (50.0 + 2.0 / 60.0 + 15.6 / 3600.0)).abs() < 1e-9);
            assert!((geo.lon + 17.0166666667).abs() < 1e-6);
            assert_eq!(geo.alt, Some(-123.0));
            assert_eq!(geo.origin, GeoOrigin::Exif);
        }
    }

    #[test]
    fn dates_without_an_offset_are_local() {
        let local = Local.with_ymd_and_hms(2023, 7, 8, 9, 10, 11).earliest().unwrap();
        assert_eq!(parse_time("2023:07:08 09:10:11", None, None), Some(local.timestamp() * 1000));
        assert_eq!(parse_time("2023:07:08 09:10:11", Some("5"), Some("garbage")), Some(local.timestamp() * 1000 + 500));
        assert_eq!(parse_time("    :  :     ", None, None), None);
    }

    #[test]
    fn orientations() {
        let turns: Vec<u16> = (1..=8).map(orientation).collect();
        assert_eq!(turns, [0, 0, 180, 180, 90, 90, 270, 270]);
    }

    #[test]
    fn unreadable_files_are_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.jpg");
        std::fs::write(&path, b"not an image").unwrap();
        assert_eq!(read(&path, FileKind::Jpeg), FileMeta::default());
        assert_eq!(read(&dir.path().join("missing.dng"), FileKind::Raw), FileMeta::default());
    }

    #[test]
    fn a_tiff_based_raw_is_read_from_its_head() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.nef");
        std::fs::write(&path, sample_exif(true)).unwrap();
        let meta = read(&path, FileKind::Raw);
        assert_eq!(meta.exif.camera_model.as_deref(), Some("X-T5"));
    }
}
