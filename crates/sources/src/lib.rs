//! The kinds of source the app and `c1cat` can open, and the sources opened before
//! (`%APPDATA%\c1studio\sources.json`).

use std::path::PathBuf;

use c1model::provider::config_root;
use c1model::{Provider, SourceSpec};
use serde::{Deserialize, Serialize};
use source_c1::CaptureOneProvider;
use source_folders::FoldersProvider;

/// Every kind of source that can be opened. A new provider is one more line here.
pub fn providers() -> Vec<Box<dyn Provider>> {
    vec![Box::new(CaptureOneProvider), Box::new(FoldersProvider)]
}

pub fn provider(id: &str) -> Result<Box<dyn Provider>, String> {
    providers().into_iter().find(|p| p.id() == id).ok_or_else(|| format!("cannot open {id:?} sources"))
}

/// What the user pointed at, with the provider to read it: the one named, else the first
/// that recognises it. The location comes back in the provider's own form.
pub fn spec_for(given: &str, provider_id: Option<&str>) -> Result<SourceSpec, String> {
    for p in providers().into_iter().filter(|p| provider_id.is_none_or(|id| p.id() == id)) {
        if let Some(location) = p.recognise(given) {
            return Ok(SourceSpec { provider: p.id().to_owned(), location });
        }
    }
    let known: Vec<&str> = providers().iter().map(|p| p.name()).collect();
    Err(format!("{given}: not something that can be opened here (it opens: {})", known.join(", ")))
}

/// Sources opened before, newest first.
#[derive(Default, Serialize, Deserialize)]
pub struct Recent {
    #[serde(default)]
    pub list: Vec<SourceSpec>,
}

const RECENT_KEPT: usize = 10;

impl Recent {
    fn path() -> PathBuf {
        config_root().join("sources.json")
    }

    pub fn load() -> Self {
        std::fs::read(Self::path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }

    pub fn remember(spec: &SourceSpec) -> Result<(), String> {
        let mut recent = Self::load();
        recent.list.retain(|s| s != spec);
        recent.list.insert(0, spec.clone());
        recent.list.truncate(RECENT_KEPT);
        recent.save()
    }

    pub fn forget(spec: &SourceSpec) -> Result<(), String> {
        let mut recent = Self::load();
        recent.list.retain(|s| s != spec);
        recent.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_provider_recognises_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = dir.path().join("Trip.cocatalogdb");
        std::fs::write(&catalog, b"").unwrap();
        let spec = spec_for(catalog.to_str().unwrap(), None).unwrap();
        assert_eq!(spec.provider, "capture-one");
        let def = source_folders::Definition::new("Trips", &[dir.path().to_owned()]);
        let file = dir.path().join("trips.c1folders");
        def.save(&file).unwrap();
        let spec = spec_for(file.to_str().unwrap(), None).unwrap();
        assert_eq!(spec.provider, "folders");
        assert_eq!(spec_for(&spec.location, Some("folders")).unwrap(), spec, "a location is recognised again");
        assert!(spec_for(file.to_str().unwrap(), Some("capture-one")).is_err());
        assert!(provider("lightroom").is_err());
    }
}
