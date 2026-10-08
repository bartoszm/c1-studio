//! Finding the catalog and the paths Capture One derives from it.

use std::env;
use std::path::{Path, PathBuf};

use c1model::SourceError;

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
    let base = env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home().join(".config"));
    base.join("c1studio").join("config.json")
}

fn read_config_catalog() -> Option<PathBuf> {
    let text = std::fs::read_to_string(config_file()).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("catalog")?.as_str().map(PathBuf::from)
}

fn home() -> PathBuf {
    env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

/// Per-catalog working data: `%LOCALAPPDATA%\c1studio\<catalog id>`.
pub fn data_dir(catalog: &Path) -> PathBuf {
    let base = env::var_os("C1_DATA_DIR")
        .or_else(|| env::var_os("LOCALAPPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local").join("share"));
    base.join("c1studio").join(catalog_id(catalog))
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

/// The original's Windows path. `is_relative` images live inside the catalog folder
/// (imported into the catalog); the rest hang off a drive root such as `H:\`.
pub(crate) fn original_path(win_root: &str, relative: &str, filename: &str, is_relative: bool, catalog_dir: &Path) -> String {
    let rel = relative.trim_matches(['\\', '/']);
    if is_relative {
        let mut path = catalog_dir.to_path_buf();
        path.extend(rel.split(['\\', '/']).filter(|s| !s.is_empty()));
        return path.join(filename).to_string_lossy().into_owned();
    }
    if rel.is_empty() { format!("{win_root}{filename}") } else { format!("{win_root}{rel}\\{filename}") }
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
