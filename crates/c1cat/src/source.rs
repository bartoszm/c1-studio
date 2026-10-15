//! Which source a command works on, and opening it the way the app does.

use std::path::Path;

use anyhow::Context;
use c1index::Library;
use c1index::thumbs::Thumbs;
use c1model::provider::data_root;
use c1model::{PhotoSource, SourceSpec};
use c1sources::{Recent, provider, spec_for};

/// The provider of Capture One catalogs, which `$C1_CATALOG` and the old catalog search are
/// about.
const CAPTURE_ONE: &str = "capture-one";

/// The global options that choose the source.
#[derive(clap::Args, Clone, Default)]
pub struct SourceArgs {
    /// The source to open: a catalog file, a folder holding one, or a `.c1folders`
    /// definition. Defaults to C1_CATALOG, then the source the app opened last, then the
    /// Capture One catalog named in config.json or found under C:\c1_catalogs.
    #[arg(long, global = true, visible_alias = "catalog", value_name = "PATH")]
    pub source: Option<String>,
    /// Open it with this provider (capture-one, folders) instead of the first that
    /// recognises it.
    #[arg(long, global = true, value_name = "ID")]
    pub provider: Option<String>,
}

/// What [`choose`] looks at besides the options, so a test can supply its own.
pub struct Places {
    /// `$C1_CATALOG`.
    pub env_catalog: Option<String>,
    /// The sources the app opened, newest first.
    pub recent: Vec<SourceSpec>,
    /// The Capture One catalog the old search finds (`config.json`, then the glob).
    pub legacy: Box<dyn FnOnce() -> Result<String, String>>,
}

impl Places {
    /// The places on this machine.
    fn here() -> Self {
        Self {
            env_catalog: std::env::var_os("C1_CATALOG").map(|v| v.to_string_lossy().into_owned()),
            recent: Recent::load().list,
            legacy: Box::new(|| {
                source_c1::find_catalog(None).map(|p| p.display().to_string()).map_err(|e| e.to_string())
            }),
        }
    }
}

/// The source to open: `--source`, else `$C1_CATALOG`, else the app's most recent source,
/// else the Capture One catalog the old search finds. A source that was asked for and is
/// not usable is an error, not a reason to open another. `--provider` limits every step to
/// that provider.
pub fn choose(args: &SourceArgs, places: Places) -> anyhow::Result<SourceSpec> {
    let only = args.provider.as_deref();
    if let Some(id) = only {
        provider(id).map_err(anyhow::Error::msg)?;
    }
    if let Some(given) = &args.source {
        return spec_for(given, only)
            .map_err(anyhow::Error::msg)
            .with_context(|| "cannot open the source given with --source");
    }
    let mut tried = vec!["--source (not given)".to_owned()];
    let capture_one = only.is_none_or(|id| id == CAPTURE_ONE);
    if capture_one {
        match places.env_catalog.filter(|v| !v.is_empty()) {
            Some(given) => {
                return spec_for(&given, Some(CAPTURE_ONE))
                    .map_err(anyhow::Error::msg)
                    .with_context(|| "cannot open the catalog named by $C1_CATALOG");
            }
            None => tried.push("$C1_CATALOG (not set)".into()),
        }
    }
    match places.recent.into_iter().find(|s| only.is_none_or(|id| s.provider == id)) {
        Some(last) => return Ok(last),
        None => tried.push("the app's recent sources (none)".into()),
    }
    if capture_one {
        match (places.legacy)() {
            Ok(path) => return spec_for(&path, Some(CAPTURE_ONE)).map_err(anyhow::Error::msg),
            Err(e) => tried.push(format!("the Capture One catalog search ({e})")),
        }
    }
    anyhow::bail!("no source to open; pass --source PATH. Tried: {}", tried.join("; "))
}

/// The source the options and this machine's settings point at.
pub fn choose_here(args: &SourceArgs) -> anyhow::Result<SourceSpec> {
    choose(args, Places::here())
}

/// A source opened, with the library and thumbnails the app keeps about it.
pub struct Session {
    pub spec: SourceSpec,
    pub source: Box<dyn PhotoSource>,
    pub library: Library,
    /// Thumbnails made for photos the source has none for (shared with the app).
    pub thumbs: Thumbs,
}

/// Opens a source through its provider, as the app does: its data in `<data root>/<key>`.
pub fn open(args: &SourceArgs) -> anyhow::Result<Session> {
    open_spec(choose_here(args)?)
}

/// As [`open`], for a source already chosen.
pub fn open_spec(spec: SourceSpec) -> anyhow::Result<Session> {
    let provider = provider(&spec.provider).map_err(anyhow::Error::msg)?;
    let dir = data_root().join(provider.data_key(&spec.location));
    let source =
        provider.open(&spec.location, &dir.join("work")).with_context(|| format!("opening {}", spec.location))?;
    let library = Library::open(dir.join("library")).context("opening the library")?;
    Ok(Session { thumbs: Thumbs::new(dir.join("thumbs")), spec, source, library })
}

/// The definition file a command works on: `path`, else the source chosen, which must be a
/// folders source.
pub fn definition_file(path: Option<&Path>, args: &SourceArgs) -> anyhow::Result<std::path::PathBuf> {
    if let Some(path) = path {
        let spec = spec_for(&path.to_string_lossy(), Some("folders")).map_err(anyhow::Error::msg)?;
        return Ok(spec.location.into());
    }
    let spec = choose_here(args)?;
    if spec.provider != "folders" {
        anyhow::bail!("{} is a {} source, not a photo folders definition", spec.location, spec.provider);
    }
    Ok(spec.location.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Places {
        Places { env_catalog: None, recent: Vec::new(), legacy: Box::new(|| Err("no such place".into())) }
    }

    fn args(source: Option<&Path>, provider: Option<&str>) -> SourceArgs {
        SourceArgs { source: source.map(|p| p.display().to_string()), provider: provider.map(str::to_owned) }
    }

    fn catalog(dir: &Path) -> std::path::PathBuf {
        let file = dir.join("Trip.cocatalogdb");
        std::fs::write(&file, b"").unwrap();
        file
    }

    fn definition(dir: &Path) -> std::path::PathBuf {
        let file = dir.join("Trips.c1folders");
        source_folders::Definition::new("Trips", &[dir.to_owned()]).save(&file).unwrap();
        file
    }

    #[test]
    fn a_definition_given_as_source_is_the_folders_provider() {
        let dir = tempfile::tempdir().unwrap();
        let file = definition(dir.path());
        let spec = choose(&args(Some(&file), None), none()).unwrap();
        assert_eq!(spec.provider, "folders");
        // The folder holding it works too.
        assert_eq!(choose(&args(Some(dir.path()), None), none()).unwrap(), spec);
        assert!(choose(&args(Some(&file), Some("capture-one")), none()).is_err(), "the other provider will not do");
    }

    #[test]
    fn a_catalog_given_as_source_is_capture_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = catalog(dir.path());
        let spec = choose(&args(Some(&file), None), none()).unwrap();
        assert_eq!(spec.provider, "capture-one");
        assert!(choose(&args(Some(&dir.path().join("nothing.jpg")), None), none()).is_err());
        assert!(choose(&args(Some(&file), Some("lightroom")), none()).is_err(), "unknown provider");
    }

    #[test]
    fn the_places_are_tried_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let file = catalog(dir.path());
        let def = definition(dir.path());
        let recent = vec![spec_for(def.to_str().unwrap(), None).unwrap()];
        let places = |env: Option<&Path>, legacy: Option<&Path>| {
            let legacy = legacy.map(|p| p.display().to_string());
            Places {
                env_catalog: env.map(|p| p.display().to_string()),
                recent: recent.clone(),
                legacy: Box::new(move || legacy.ok_or_else(|| "none".to_owned())),
            }
        };
        let other = tempfile::tempdir().unwrap();
        let other_catalog = catalog(other.path());
        // $C1_CATALOG beats the recent list, which beats the old search.
        let spec = choose(&args(None, None), places(Some(&file), Some(&other_catalog))).unwrap();
        assert_eq!((spec.provider.as_str(), spec.location.ends_with("Trip.cocatalogdb")), ("capture-one", true));
        assert_eq!(choose(&args(None, None), places(None, Some(&other_catalog))).unwrap(), recent[0]);
        // --provider capture-one skips the folders source in the recent list.
        let spec = choose(&args(None, Some("capture-one")), places(None, Some(&other_catalog))).unwrap();
        assert_eq!(spec.provider, "capture-one");
        // --provider folders ignores $C1_CATALOG and the old search.
        assert_eq!(choose(&args(None, Some("folders")), places(Some(&file), Some(&other_catalog))).unwrap(), recent[0]);
    }

    #[test]
    fn the_old_search_is_the_last_resort_and_the_error_lists_what_was_tried() {
        let dir = tempfile::tempdir().unwrap();
        let file = catalog(dir.path());
        let found = Places {
            legacy: Box::new({
                let f = file.display().to_string();
                move || Ok(f)
            }),
            ..none()
        };
        assert_eq!(choose(&args(None, None), found).unwrap().provider, "capture-one");
        let error = choose(&args(None, None), none()).unwrap_err().to_string();
        for place in ["--source", "$C1_CATALOG", "recent", "no such place"] {
            assert!(error.contains(place), "{error:?} names {place}");
        }
        let error = choose(&args(None, Some("folders")), none()).unwrap_err().to_string();
        assert!(!error.contains("$C1_CATALOG"), "{error:?}");
    }

    #[test]
    fn an_unusable_catalog_variable_is_an_error_not_a_reason_to_open_another() {
        let dir = tempfile::tempdir().unwrap();
        let recent = vec![spec_for(definition(dir.path()).to_str().unwrap(), None).unwrap()];
        let places = Places { env_catalog: Some("/no/such.cocatalogdb".into()), recent, ..none() };
        let error = format!("{:#}", choose(&args(None, None), places).unwrap_err());
        assert!(error.contains("C1_CATALOG") && error.contains("/no/such.cocatalogdb"), "{error}");
    }

    #[test]
    fn a_capture_one_catalog_keeps_the_data_folder_it_always_had() {
        let dir = tempfile::tempdir().unwrap();
        let file = catalog(dir.path());
        let spec = choose(&args(Some(&file), None), none()).unwrap();
        let key = provider(&spec.provider).unwrap().data_key(&spec.location);
        assert_eq!(data_root().join(key), source_c1::data_dir(Path::new(&spec.location)));
    }
}
