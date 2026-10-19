//! Which source the app opens at start: the command line, `$C1_CATALOG`, else the source
//! opened last. The providers, the ones offered and the recent list are `c1sources`'.

use c1model::SourceSpec;
pub use c1sources::{Offered, Recent, offered_providers, provider, providers, spec_among, spec_for};

/// What to open at start, and whether to put it on the recent list:
/// 1. the command line, `c1studio <path> [--provider <id>]`;
/// 2. `$C1_CATALOG` (scripts and test instances: not remembered);
/// 3. the source opened last of a kind still offered.
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
    let offered = Offered::load();
    if let Some(last) = Recent::load().list.into_iter().find(|s| offered.allows(&s.provider)) {
        return Ok((last, false));
    }
    Err("no catalog open yet: choose one".into())
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
