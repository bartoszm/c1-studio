//! Finding the catalog and the paths Capture One derives from it.

use std::env;
use std::path::{Path, PathBuf};

use c1model::{Crop, SourceError};

/// Resolves the catalog: explicit path, then `$C1_CATALOG`, then the config file, then
/// a search of `C:\c1_catalogs\*\*.cocatalogdb`.
pub fn find_catalog(explicit: Option<&Path>) -> Result<PathBuf, SourceError> {
    let from_config = read_config_catalog();
    let candidates = [explicit.map(Path::to_path_buf), env::var_os("C1_CATALOG").map(PathBuf::from), from_config];
    if let Some(path) = candidates.into_iter().flatten().next() {
        return if path.is_file() { Ok(path) } else { Err(SourceError::NotFound(path.display().to_string())) };
    }

    let root = Path::new(r"C:\c1_catalogs");
    let mut found: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|dir| std::fs::read_dir(dir.path()).ok())
        .flatten()
        .flatten()
        .map(|e| e.path())
        // Capture One's rolling backups sit alongside as "name.cocatalogdb.backup N".
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cocatalogdb")))
        .collect();
    found.sort();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(SourceError::NotFound(format!(
            "no catalog found; pass one explicitly, set C1_CATALOG, or write {{\"catalog\": \"...\"}} to {}",
            config_file().display()
        ))),
        _ => Err(SourceError::Other(format!(
            "several catalogs found, choose one: {}",
            found.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// `%APPDATA%\c1studio\config.json`.
pub fn config_file() -> PathBuf {
    c1model::provider::config_root().join("config.json")
}

fn read_config_catalog() -> Option<PathBuf> {
    let text = std::fs::read_to_string(config_file()).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("catalog")?.as_str().map(PathBuf::from)
}

/// Per-catalog working data: `%LOCALAPPDATA%\c1studio\<catalog id>`.
pub fn data_dir(catalog: &Path) -> PathBuf {
    c1model::provider::data_root().join(catalog_id(catalog))
}

/// A readable, stable key per catalog: the stem plus a hash of the full path.
pub fn catalog_id(catalog: &Path) -> String {
    let full = catalog.to_string_lossy().to_lowercase();
    let hash = xxhash_rust::xxh3::xxh3_64(full.as_bytes()) as u32;
    let stem: String = catalog
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let stem = stem.trim_matches('-');
    format!("{}-{hash:08x}", if stem.is_empty() { "catalog" } else { stem })
}

/// Capture One's browser thumbnail: a plain 450 px JPEG keyed by variant, with the
/// crop and styles already applied.
pub(crate) fn thumbnail_path(catalog_dir: &Path, variant_pk: i64) -> PathBuf {
    catalog_dir.join("Cache").join("Thumbnails").join(format!("{variant_pk:08}.cot"))
}

/// Capture One's screen preview: `Cache\Previews\<ZIMAGE.ZSIDECARPATH>.cop`, a JPEG-XL
/// of linear camera RGB before white balance. The sidecar path is sometimes lower-cased
/// relative to the file on disk, so a miss falls back to a case-insensitive look through
/// that one folder.
pub(crate) fn preview_path(catalog_dir: &Path, sidecar: &str) -> Option<PathBuf> {
    if sidecar.is_empty() {
        return None;
    }
    let mut path = catalog_dir.join("Cache").join("Previews");
    path.extend(sidecar.split(['\\', '/']).filter(|s| !s.is_empty()));
    let mut name = path.file_name()?.to_os_string();
    name.push(".cop");
    path.set_file_name(&name);
    if path.is_file() {
        return Some(path);
    }
    let wanted = name.to_string_lossy().to_lowercase();
    std::fs::read_dir(path.parent()?)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().to_lowercase() == wanted)
        .map(|e| e.path())
}

/// `ZWHITEBALANCE`: `R;G;B` multipliers with G normalised to 1.
pub(crate) fn parse_white_balance(text: &str) -> Option<[f32; 3]> {
    let values: Vec<f32> = text.split(';').map(|v| v.trim().parse::<f32>()).collect::<Result<_, _>>().ok()?;
    match values.as_slice() {
        [r, g, b, ..] if [r, g, b].iter().all(|v| v.is_finite() && **v > 0.0) => Some([*r, *g, *b]),
        _ => None,
    }
}

/// What is left of `degrees` after the nearest quarter turn: the straightening, under 45
/// either way.
pub(crate) fn straightening(degrees: f64) -> f32 {
    if !degrees.is_finite() {
        return 0.0;
    }
    (degrees - (degrees / 90.0).round() * 90.0) as f32
}

/// `ZCROP` (`centreX;centreY;width;height`) in the displayed orientation. The centre is in
/// the sensor's orientation, the size already in the displayed one, both in pixels of the
/// `width` × `height` sensor image; `degrees` is `ZROTATION`. `None` when it is missing,
/// malformed or leaves the whole picture.
pub(crate) fn display_crop(text: &str, width: f64, height: f64, degrees: f64) -> Option<Crop> {
    let values: Vec<f64> = text.split(';').map(|v| v.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
    let [cx, cy, cw, ch] = values.as_slice() else { return None };
    if ![cx, cy, cw, ch].iter().all(|v| v.is_finite()) || *cw <= 0.0 || *ch <= 0.0 || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let (x, y) = (cx - width / 2.0, cy - height / 2.0);
    // Turned clockwise with the picture (y grows downwards).
    let ((x, y), (w, h)) = match quarter_turn(degrees) {
        90 => ((-y, x), (height, width)),
        180 => ((-x, -y), (width, height)),
        270 => ((y, -x), (height, width)),
        _ => ((x, y), (width, height)),
    };
    let crop = Crop { dx: (x / w) as f32, dy: (y / h) as f32, width: (cw / w) as f32, height: (ch / h) as f32 };
    let whole = crop.dx.abs() < 1e-3 && crop.dy.abs() < 1e-3 && crop.width > 0.999 && crop.height > 0.999;
    (!whole).then_some(crop)
}

/// Without a crop of its own, a straightened picture is cut to the largest frame of its
/// own shape that the turned picture fills, as Capture One shows it. `None` when it is not
/// straightened.
pub(crate) fn straightened_crop(width: f64, height: f64, degrees: f64) -> Option<Crop> {
    let theta = f64::from(straightening(degrees)).to_radians().abs();
    if theta < 1e-4 || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let long = width.max(height) / width.min(height);
    let k = (1.0 / (long * theta.sin() + theta.cos())) as f32;
    Some(Crop { dx: 0.0, dy: 0.0, width: k, height: k })
}

/// `crop` measured against the canvas straightening grew instead of the picture: for
/// renders that are straightened already, such as Capture One's thumbnails.
pub(crate) fn on_canvas(crop: Crop, width: f64, height: f64, degrees: f64) -> Crop {
    let (w, h) = if quarter_turn(degrees) % 180 == 90 { (height, width) } else { (width, height) };
    let (sin, cos) = f64::from(straightening(degrees)).to_radians().sin_cos();
    let (sin, cos) = (sin.abs(), cos.abs());
    let (kx, ky) = ((w / (w * cos + h * sin)) as f32, (h / (w * sin + h * cos)) as f32);
    Crop { dx: crop.dx * kx, dy: crop.dy * ky, width: crop.width * kx, height: crop.height * ky }
}

/// Degrees to the nearest quarter turn, in 0..360.
pub(crate) fn quarter_turn(degrees: f64) -> u16 {
    if !degrees.is_finite() {
        return 0;
    }
    ((degrees / 90.0).round().rem_euclid(4.0) * 90.0) as u16
}

/// The original's Windows path. `is_relative` images live inside the catalog folder
/// (imported into the catalog); the rest hang off a drive root such as `H:\`.
pub(crate) fn original_path(
    win_root: &str,
    relative: &str,
    filename: &str,
    is_relative: bool,
    catalog_dir: &Path,
) -> String {
    let rel = relative.trim_matches(['\\', '/']);
    if is_relative {
        let mut path = catalog_dir.to_path_buf();
        path.extend(rel.split(['\\', '/']).filter(|s| !s.is_empty()));
        return path.join(filename).to_string_lossy().into_owned();
    }
    if rel.is_empty() { format!("{win_root}{filename}") } else { format!("{win_root}{rel}\\{filename}") }
}

/// Signed decimal degrees as Capture One writes them: `44.803944444445N`, `20.37835W`.
/// The number is the shortest that reads back as the same value (as Python's `repr`,
/// which the oracle uses: a whole number keeps its `.0`).
pub(crate) fn format_gps(value: f64, latitude: bool) -> String {
    let letter = match (latitude, value >= 0.0) {
        (true, true) => 'N',
        (true, false) => 'S',
        (false, true) => 'E',
        (false, false) => 'W',
    };
    let abs = value.abs();
    if abs.fract() == 0.0 { format!("{abs:.1}{letter}") } else { format!("{abs}{letter}") }
}

/// Signed decimal degrees from either form Capture One stores:
/// decimal degrees `44.803944444445N`, or the XMP/EXIF form `50,2.2566833N`
/// (degrees, decimal minutes) and `50,2,15.4N` (degrees, minutes, seconds).
/// The comma forms come from GPS written by the camera; ~800 photos in the real
/// catalog use them, and the Python `parse_gps` dropped every one.
pub(crate) fn parse_gps(value: Option<&str>) -> Option<f64> {
    let text = value?.trim();
    let (number, sign) = match text.chars().last()? {
        'N' | 'n' | 'E' | 'e' => (&text[..text.len() - 1], 1.0),
        'S' | 's' | 'W' | 'w' => (&text[..text.len() - 1], -1.0),
        _ => (text, 1.0),
    };
    let parts: Vec<f64> = number.split(',').map(|p| p.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
    let degrees = match parts.as_slice() {
        [d] => *d,
        [d, m] => d + m / 60.0,
        [d, m, s] => d + m / 60.0 + s / 3600.0,
        _ => return None,
    };
    degrees.is_finite().then_some(degrees * sign)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gps_written_reads_back_the_same() {
        assert_eq!(format_gps(44.803944444445, true), "44.803944444445N");
        assert_eq!(format_gps(-20.37835, false), "20.37835W");
        assert_eq!(format_gps(-33.0, true), "33.0S");
        assert_eq!(format_gps(0.0, false), "0.0E");
        for v in [50.0 + 2.2566833 / 60.0, -(19.0 + 56.0 / 60.0 + 30.0 / 3600.0), 1e-7, 179.99999999999997] {
            assert_eq!(parse_gps(Some(&format_gps(v, true))), Some(v));
        }
    }

    #[test]
    fn gps_hemispheres() {
        assert_eq!(parse_gps(Some("44.803944444445N")), Some(44.803944444445));
        assert_eq!(parse_gps(Some("20.37835W")), Some(-20.37835));
        let lat = parse_gps(Some("50,2.2566833N")).unwrap();
        assert!((lat - (50.0 + 2.2566833 / 60.0)).abs() < 1e-12);
        let lon = parse_gps(Some("19,56,30S")).unwrap();
        assert!((lon + (19.0 + 56.0 / 60.0 + 30.0 / 3600.0)).abs() < 1e-12);
        assert_eq!(parse_gps(Some("")), None);
        assert_eq!(parse_gps(Some("garbage")), None);
        assert_eq!(parse_gps(None), None);
    }

    #[test]
    fn original_paths() {
        let dir = Path::new(r"C:\c1_catalogs\fuji");
        assert_eq!(original_path(r"H:\", r"fotos\2026", "A.RAF", false, dir), r"H:\fotos\2026\A.RAF");
        assert_eq!(original_path(r"H:\", "", "A.RAF", false, dir), r"H:\A.RAF");
        assert_eq!(original_path("", r"Originals\x", "A.RAF", true, dir), r"C:\c1_catalogs\fuji\Originals\x\A.RAF");
    }

    #[test]
    fn white_balance_parses_or_is_absent() {
        assert_eq!(parse_white_balance("1.887000;1.000000;1.844000"), Some([1.887, 1.0, 1.844]));
        assert_eq!(parse_white_balance(""), None);
        assert_eq!(parse_white_balance("1;0;1"), None);
        assert_eq!(parse_white_balance("a;b;c"), None);
    }

    #[test]
    fn rotation_snaps_to_quarter_turns() {
        assert_eq!(quarter_turn(0.0), 0);
        assert_eq!(quarter_turn(270.0), 270);
        assert_eq!(quarter_turn(269.59), 270);
        assert_eq!(quarter_turn(358.42), 0);
        assert_eq!(quarter_turn(-90.0), 270);
        assert_eq!(quarter_turn(f64::NAN), 0);
    }

    #[test]
    fn preview_path_tolerates_case() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Cache").join("Previews").join("2026").join("9");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("DSCF8638.RAF.cop"), b"").unwrap();
        // Windows finds it directly; case-sensitive filesystems through the folder scan.
        let found = preview_path(dir.path(), "2026/9/dscf8638.raf").unwrap();
        assert!(found.is_file());
        assert!(found.file_name().unwrap().eq_ignore_ascii_case("DSCF8638.RAF.cop"));
        assert_eq!(preview_path(dir.path(), "2026/9/missing.raf"), None);
        assert_eq!(preview_path(dir.path(), ""), None);
    }

    #[test]
    fn straightening_is_what_the_quarter_turn_leaves() {
        assert!((straightening(91.27) - 1.27).abs() < 1e-4);
        assert!((straightening(-92.17) + 2.17).abs() < 1e-4);
        assert!((straightening(0.47) - 0.47).abs() < 1e-4);
        assert_eq!(straightening(270.0), 0.0);
    }

    #[test]
    fn crop_in_the_displayed_orientation() {
        // DSCF5264: 82% of the frame, off to the right and down.
        let c = display_crop("3843.28;2803.90;6360.57;4240.38", 7728.0, 5152.0, 0.0).unwrap();
        assert!((c.width - 0.823).abs() < 1e-3 && (c.height - 0.823).abs() < 1e-3);
        assert!((c.dx - (3843.28 - 3864.0) / 7728.0).abs() < 1e-5);
        assert!((c.dy - (2803.90 - 2576.0) / 5152.0).abs() < 1e-5);
        // A portrait crop of a turned photo: its size only fits the turned frame.
        let c = display_crop("3151.89;2101.26;3797.43;5696.15", 6000.0, 4000.0, 270.0).unwrap();
        assert!(c.width <= 1.0 && c.height <= 1.0, "{c:?}");
        // Right of the sensor's centre is above the centre once turned anticlockwise.
        assert!(c.dy < 0.0 && c.dx.abs() < 0.04, "{c:?}");
        assert!(display_crop("3864;2576;7728;5152", 7728.0, 5152.0, 0.0).is_none());
        assert!(display_crop("", 7728.0, 5152.0, 0.0).is_none());
    }

    #[test]
    fn straightening_alone_crops_to_what_the_turn_fills() {
        assert!(straightened_crop(6000.0, 4000.0, 0.0).is_none());
        assert!(straightened_crop(6000.0, 4000.0, 90.0).is_none());
        // untitled-1506: −1.43° keeps 96% of the frame.
        let c = straightened_crop(6000.0, 4000.0, -1.43).unwrap();
        assert!((c.width - 0.964).abs() < 1e-3 && c.width == c.height, "{c:?}");
        assert_eq!(straightened_crop(4000.0, 6000.0, 91.43), Some(c));
    }

    #[test]
    fn a_crop_on_the_grown_canvas_is_smaller() {
        let c = Crop { dx: 0.1, dy: 0.0, width: 0.5, height: 0.5 };
        assert_eq!(on_canvas(c, 6000.0, 4000.0, 90.0), c);
        let turned = on_canvas(c, 6000.0, 4000.0, 1.67);
        // 6000 × 4000 turned 1.67° needs a 6114-wide canvas.
        assert!((turned.width - 0.5 * 6000.0 / 6114.4).abs() < 1e-3, "{turned:?}");
        assert!(turned.dx < c.dx && turned.height < c.height);
    }

    #[test]
    fn thumbnail_is_zero_padded() {
        let p = thumbnail_path(Path::new(r"C:\cat"), 42);
        assert!(p.ends_with(r"Cache\Thumbnails\00000042.cot"));
    }

    #[test]
    fn catalog_id_is_readable_and_stable() {
        let id = catalog_id(Path::new(r"C:\c1_catalogs\fuji\fiuji.cocatalogdb"));
        assert!(id.starts_with("fiuji-"));
        assert_eq!(id, catalog_id(Path::new(r"C:\C1_CATALOGS\fuji\fiuji.cocatalogdb")));
    }
}
