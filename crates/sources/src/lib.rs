//! The kinds of source the app and `c1cat` can open, the ones the app offers
//! (`%APPDATA%\c1studio\providers.json`), and the sources opened before
//! (`%APPDATA%\c1studio\sources.json`).

use std::path::{Path, PathBuf};

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

/// The providers the app offers: those [`Offered`] allows.
pub fn offered_providers() -> Vec<Box<dyn Provider>> {
    let offered = Offered::load();
    providers().into_iter().filter(|p| offered.allows(p.id())).collect()
}

/// What the user pointed at, with the provider to read it: the one named, else the first
/// that recognises it. The location comes back in the provider's own form.
pub fn spec_for(given: &str, provider_id: Option<&str>) -> Result<SourceSpec, String> {
    spec_among(&providers(), given, provider_id)
}

/// [`spec_for`] among the given providers only.
pub fn spec_among(among: &[Box<dyn Provider>], given: &str, provider_id: Option<&str>) -> Result<SourceSpec, String> {
    for p in among.iter().filter(|p| provider_id.is_none_or(|id| p.id() == id)) {
        if let Some(location) = p.recognise(given) {
            return Ok(SourceSpec { provider: p.id().to_owned(), location });
        }
    }
    if let Some(id) = provider_id.filter(|id| !among.iter().any(|p| p.id() == *id))
        && let Ok(p) = provider(id)
    {
        return Err(format!("{given}: {} is turned off; turn it on in the Source menu", p.name()));
    }
    let known: Vec<&str> = among.iter().map(|p| p.name()).collect();
    Err(format!("{given}: not something that can be opened here (it opens: {})", known.join(", ")))
}

/// The providers the app offers, as the installer or the Source menu left them. No file,
/// or a list naming no provider there is, offers them all; `c1cat` ignores it.
#[derive(Default, Serialize, Deserialize)]
pub struct Offered {
    #[serde(default)]
    pub providers: Vec<String>,
}

impl Offered {
    fn path() -> PathBuf {
        config_root().join("providers.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    fn load_from(path: &Path) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn allows(&self, id: &str) -> bool {
        let known = |p: &String| providers().iter().any(|k| k.id() == p);
        self.providers.iter().any(|p| p == id) || !self.providers.iter().any(known)
    }

    /// Offers these providers from now on: at least one, each one there is.
    pub fn save(ids: &[String]) -> Result<(), String> {
        Self::save_to(&Self::path(), ids)
    }

    fn save_to(path: &Path, ids: &[String]) -> Result<(), String> {
        if ids.is_empty() {
            return Err("at least one kind of source must stay on".into());
        }
        for id in ids {
            provider(id)?;
        }
        let offered = Offered { providers: ids.to_vec() };
        write_json(path, &offered)
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
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
        write_json(&Self::path(), self)
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
        let only_folders: Vec<_> = providers().into_iter().filter(|p| p.id() == "folders").collect();
        let off = spec_among(&only_folders, catalog.to_str().unwrap(), Some("capture-one")).unwrap_err();
        assert!(off.contains("turned off"), "{off}");
        assert!(spec_among(&only_folders, catalog.to_str().unwrap(), None).is_err());
    }

    #[test]
    fn offered_providers_default_to_all_and_keep_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c1studio").join("providers.json");
        let all = Offered::load_from(&path);
        assert!(all.allows("capture-one") && all.allows("folders"), "no file offers them all");
        Offered::save_to(&path, &["folders".into()]).unwrap();
        let some = Offered::load_from(&path);
        assert!(some.allows("folders") && !some.allows("capture-one"));
        assert!(Offered::save_to(&path, &[]).is_err());
        assert!(Offered::save_to(&path, &["lightroom".into()]).is_err());
        let unknown = Offered { providers: vec!["lightroom".into()] };
        assert!(unknown.allows("capture-one"), "a list naming none there is offers them all");
        std::fs::write(&path, b"not json").unwrap();
        assert!(Offered::load_from(&path).allows("capture-one"));
    }

    /// The installer's page writes the ids itself: each provider must be on it.
    #[test]
    fn the_installer_offers_every_provider() {
        let nsi = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/windows/hooks.nsh");
        let nsi = std::fs::read_to_string(&nsi).unwrap();
        for p in providers() {
            assert!(nsi.contains(&format!("\"{}\"", p.id())), "{} is not on the installer's page", p.id());
        }
    }
}
