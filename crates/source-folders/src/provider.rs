//! The `folders` provider: finds definition files and opens them.

use std::path::{Path, PathBuf};

use c1model::{PhotoSource, Pick, Provider, Result};

use crate::FolderSource;
use crate::definition::{Definition, EXTENSION, slug};

pub struct FoldersProvider;

impl Provider for FoldersProvider {
    fn id(&self) -> &'static str {
        crate::PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Photo folders"
    }

    fn pick(&self) -> Pick {
        Pick::File { label: "Photo folders", extensions: &[EXTENSION] }
    }

    /// A definition file, or a folder holding exactly one. Never a plain photo folder: that
    /// needs a definition made for it first (the app's Folders dialog, `c1cat folders init`).
    fn recognise(&self, given: &str) -> Option<String> {
        let path = resolve(Path::new(given))?;
        Some(std::path::absolute(&path).unwrap_or(path).to_string_lossy().into_owned())
    }

    fn data_key(&self, location: &str) -> String {
        match Definition::load(Path::new(location)) {
            Ok(def) => def.data_key(),
            // Unreadable: still a stable key, from where it is.
            Err(_) => {
                let stem = Path::new(location).file_stem().map(|s| s.to_string_lossy().into_owned());
                let hash = xxhash_rust::xxh3::xxh3_64(location.to_lowercase().as_bytes()) as u32;
                format!("{}-{hash:08x}", slug(&stem.unwrap_or_default()))
            }
        }
    }

    fn open(&self, location: &str, work_dir: &Path) -> Result<Box<dyn PhotoSource>> {
        Ok(Box::new(FolderSource::open(Path::new(location), work_dir)?))
    }
}

fn is_definition(path: &Path) -> bool {
    path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

fn resolve(given: &Path) -> Option<PathBuf> {
    if is_definition(given) {
        return Some(given.to_owned());
    }
    if !given.is_dir() {
        return None;
    }
    let mut found =
        std::fs::read_dir(given).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| is_definition(p));
    let one = found.next()?;
    found.next().is_none().then_some(one)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_definition_or_a_folder_with_one() {
        let dir = tempfile::tempdir().unwrap();
        let p = FoldersProvider;
        let photos = dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        assert_eq!(p.recognise(photos.to_str().unwrap()), None, "a plain folder is not a source");
        let def = Definition::new("Trips", std::slice::from_ref(&photos));
        let file = dir.path().join("trips.c1folders");
        def.save(&file).unwrap();
        let location = p.recognise(file.to_str().unwrap()).unwrap();
        assert_eq!(p.recognise(dir.path().to_str().unwrap()), Some(location.clone()));
        assert_eq!(p.recognise(&location), Some(location.clone()), "its own location");
        assert_eq!(p.data_key(&location), def.data_key());
        Definition::new("Other", &[]).save(&dir.path().join("other.c1folders")).unwrap();
        assert_eq!(p.recognise(dir.path().to_str().unwrap()), None, "two: ambiguous");
        assert!(p.data_key("/nowhere/x.c1folders").starts_with("x-"));
    }
}
