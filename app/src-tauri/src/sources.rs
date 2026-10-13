//! Which source the app opens: the providers it was built with, the command line, and
//! the recent list (`%APPDATA%\c1studio\sources.json`).

use std::path::PathBuf;

use c1model::provider::config_root;
use c1model::{Provider, SourceSpec};
use serde::{Deserialize, Serialize};
use source_c1::CaptureOneProvider;

/// Every kind of source the app can open. A new provider is one more line here.
pub fn providers() -> Vec<Box<dyn Provider>> {
    vec![Box::new(CaptureOneProvider)]
}

pub fn provider(id: &str) -> Result<Box<dyn Provider>, String> {
    providers().into_iter().find(|p| p.id() == id).ok_or_else(|| format!("the app cannot open {id:?} sources"))
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
    Err(format!("{given}: not something the app can open (it opens: {})", known.join(", ")))
}

/// What to open at start, and whether to put it on the recent list:
/// 1. the command line, `c1studio <path> [--provider <id>]`;
/// 2. `$C1_CATALOG` (scripts and test instances: not remembered);
/// 3. the source opened last.
///
/// Nothing is looked for beyond these: a first start opens nothing and asks.
pub fn startup(args: &[String]) -> Result<(SourceSpec, bool), String> {
    let mut provider_id = None;
    let mut given = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--provider" => provider_id = it.next().cloned(),
            flag if flag.starts_with('-') => {}
            path => given = given.or_else(|| Some(path.to_owned())),
        }
    }
    if let Some(given) = given {
        return spec_for(&given, provider_id.as_deref()).map(|s| (s, true));
    }
    if let Some(catalog) = std::env::var_os("C1_CATALOG") {
        return spec_for(&catalog.to_string_lossy(), Some("capture-one")).map(|s| (s, false));
    }
    if let Some(last) = Recent::load().list.into_iter().next() {
        return Ok((last, false));
    }
    Err("no catalog open yet: choose one".into())
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
    fn a_path_on_the_command_line_picks_its_provider() {
        let dir = std::env::temp_dir().join(format!("c1studio-sources-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let catalog = dir.join("Trip.cocatalogdb");
        std::fs::write(&catalog, b"").unwrap();
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (spec, remember) = startup(&args(&[catalog.to_str().unwrap()])).unwrap();
        assert_eq!((spec.provider.as_str(), remember), ("capture-one", true));
        // The folder holding it works too; flags are skipped; a named provider is used.
        let (by_folder, _) = startup(&args(&["--flag", dir.to_str().unwrap(), "--provider", "capture-one"])).unwrap();
        assert_eq!(by_folder, spec);
        assert!(startup(&args(&[dir.join("nope.jpg").to_str().unwrap()])).is_err());
        assert!(startup(&args(&[catalog.to_str().unwrap(), "--provider", "lightroom"])).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
