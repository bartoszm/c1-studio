//! Capture One as a [`Provider`]: a catalog is its `.cocatalogdb` file, or the folder
//! that holds one (Capture One's own catalog folder).

use std::path::{Path, PathBuf};

use c1model::source::Result;
use c1model::{PhotoSource, Pick, Provider};

use crate::CaptureOne;
use crate::config::{catalog_id, find_catalog};

pub struct CaptureOneProvider;

impl Provider for CaptureOneProvider {
    fn id(&self) -> &'static str {
        "capture-one"
    }

    fn name(&self) -> &'static str {
        "Capture One catalog"
    }

    fn pick(&self) -> Pick {
        Pick::File { label: "Capture One catalog", extensions: &["cocatalogdb"] }
    }

    fn recognise(&self, given: &str) -> Option<String> {
        resolve(Path::new(given)).map(|p| p.display().to_string())
    }

    /// The same key the app has always used, so a catalog keeps its library however its
    /// path is spelled (`C:/x/a.cocatalogdb` and `c:\x\A.cocatalogdb` are one catalog).
    fn data_key(&self, location: &str) -> String {
        catalog_id(&absolute(Path::new(location)))
    }

    /// `$C1_CATALOG`, the config file, or the only catalog under `C:\c1_catalogs`.
    fn find_default(&self) -> Option<String> {
        find_catalog(None).ok().map(|p| p.display().to_string())
    }

    fn open(&self, location: &str, work_dir: &Path) -> Result<Box<dyn PhotoSource>> {
        let path = resolve(Path::new(location)).unwrap_or_else(|| PathBuf::from(location));
        Ok(Box::new(CaptureOne::open(path, work_dir)?))
    }
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn is_catalog(path: &Path) -> bool {
    path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("cocatalogdb"))
}

/// The catalog file for a file or a folder holding exactly one (its rolling backups,
/// `name.cocatalogdb.backup 1`, do not count).
fn resolve(given: &Path) -> Option<PathBuf> {
    if is_catalog(given) {
        return Some(absolute(given));
    }
    let mut found = std::fs::read_dir(given).ok()?.flatten().map(|e| e.path()).filter(|p| is_catalog(p));
    let only = found.next()?;
    found.next().is_none().then(|| absolute(&only))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catalog_is_its_file_or_the_folder_holding_it() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = dir.path().join("Fuji.cocatalogdb");
        std::fs::write(&catalog, b"").unwrap();
        std::fs::write(dir.path().join("Fuji.cocatalogdb.backup 1"), b"").unwrap();
        let p = CaptureOneProvider;
        let file = p.recognise(catalog.to_str().unwrap()).unwrap();
        assert_eq!(p.recognise(dir.path().to_str().unwrap()), Some(file.clone()));
        assert_eq!(p.recognise(dir.path().join("nothing.jpg").to_str().unwrap()), None);
        // A second catalog in the folder: which one is meant is not clear.
        std::fs::write(dir.path().join("Other.cocatalogdb"), b"").unwrap();
        assert_eq!(p.recognise(dir.path().to_str().unwrap()), None);
        assert_eq!(p.recognise(&file), Some(file));
    }

    #[test]
    fn the_data_key_ignores_how_the_path_is_spelled() {
        let p = CaptureOneProvider;
        let key = p.data_key(r"C:\c1_catalogs\fuji\fiuji.cocatalogdb");
        assert!(key.starts_with("fiuji-"));
        if cfg!(windows) {
            // What the app keyed this catalog's library by before providers: unchanged.
            assert_eq!(key, "fiuji-ba047789");
            assert_eq!(p.data_key("c:/C1_catalogs/fuji/FIUJI.cocatalogdb"), key);
        }
    }
}
