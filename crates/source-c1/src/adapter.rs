//! One adapter per family of Capture One catalog schemas.
//!
//! Capture One changes its catalog between releases: 16.8 added `ZRETOUCHINGLAYER`, 16.9 a
//! table of faces and columns nothing here uses. What differs between the schemas the app
//! knows is kept here, behind [`Adapter`], and the adapter is chosen by the catalog's
//! schema number ([`choose`]). The rest of the crate asks the adapter and does not look at
//! the number.
//!
//! - An adapter lists every schema it was checked against ([`Adapter::versions`]): one
//!   adapter serves several Capture One versions while they differ in nothing it touches.
//! - Each method of the trait has the newest schema's way as its default. An adapter for
//!   an older or a newer schema overrides only what is different there.
//! - A schema no adapter lists is read with the adapter of the nearest older one, as
//!   untested, and is not written.
//!
//! To support another Capture One version: when it differs in nothing used here, add its
//! schema to the adapter that fits and run `tools/difftest` on such a catalog. When it
//! differs, add an adapter that overrides the methods concerned (and a method to the
//! trait for a difference that has none yet, with today's SQL as its default), then list
//! it in [`ADAPTERS`].

use std::collections::HashMap;

use c1model::Support;
use c1model::source::Result;
use rusqlite::Connection;

use crate::catalog::db_err;

/// A catalog schema an adapter was checked against.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Version {
    /// `ZVERSIONINFO.ZVERSION`.
    pub schema: i64,
    /// The Capture One that writes it, for messages: "16.8".
    pub capture_one: &'static str,
}

/// How catalogs of some schemas are read and written, where the schemas differ.
pub(crate) trait Adapter: Sync {
    /// What it is called in messages and logs.
    fn name(&self) -> &'static str;

    /// The schemas it was checked against, newest first.
    fn versions(&self) -> &'static [Version];

    /// Whether writes were checked against these schemas.
    fn write(&self) -> Support;

    /// How many retouching layers each variant has, by variant.
    fn retouching_layers(&self, con: &Connection) -> Result<HashMap<i64, u32>> {
        let mut layers = HashMap::new();
        let mut stmt = con.prepare("SELECT ZVARIANT FROM ZRETOUCHINGLAYER").map_err(db_err)?;
        let mut rows = stmt.query([]).map_err(db_err)?;
        while let Some(r) = rows.next().map_err(db_err)? {
            *layers.entry(r.get(0).map_err(db_err)?).or_default() += 1;
        }
        Ok(layers)
    }
}

/// Capture One 16.8 and 16.9: the catalog the reader and the differential write tests were
/// built on, and the same catalog after 16.9 upgraded it (a `ZFACE` table and columns on
/// `ZIMAGE`, `ZVARIANTLAYER` and `ZRETOUCHINGLAYER` were added; none this code reads or
/// writes was changed, and the differential read and write tests pass on it).
struct Current;

impl Adapter for Current {
    fn name(&self) -> &'static str {
        "16.8 to 16.9"
    }

    fn versions(&self) -> &'static [Version] {
        &[Version { schema: 160906, capture_one: "16.9" }, Version { schema: 160800, capture_one: "16.8" }]
    }

    fn write(&self) -> Support {
        Support::Tested
    }
}

/// Capture One 16.5: a real catalog, read and checked; not written. It has no
/// `ZRETOUCHINGLAYER` table.
struct Before168;

impl Adapter for Before168 {
    fn name(&self) -> &'static str {
        "16.5"
    }

    fn versions(&self) -> &'static [Version] {
        &[Version { schema: 1650, capture_one: "16.5" }]
    }

    fn write(&self) -> Support {
        Support::Off
    }

    fn retouching_layers(&self, _con: &Connection) -> Result<HashMap<i64, u32>> {
        Ok(HashMap::new())
    }
}

/// Every adapter, the one for the newest schemas first.
static ADAPTERS: &[&dyn Adapter] = &[&Current, &Before168];

/// The adapter a catalog is read with.
pub(crate) struct Chosen {
    pub adapter: &'static dyn Adapter,
    /// The version of the adapter's the catalog is, when it is one of them: else the
    /// adapter is the nearest there is, and the catalog untested.
    pub exact: Option<&'static Version>,
}

impl Chosen {
    pub(crate) fn read(&self) -> Support {
        if self.exact.is_some() { Support::Tested } else { Support::Untested }
    }

    pub(crate) fn write(&self) -> Support {
        if self.exact.is_some() { self.adapter.write() } else { Support::Off }
    }
}

/// Every schema known with its adapter, newest first.
fn known() -> Vec<(&'static dyn Adapter, &'static Version)> {
    let mut all: Vec<_> = ADAPTERS.iter().flat_map(|&a| a.versions().iter().map(move |v| (a, v))).collect();
    all.sort_by_key(|(_, v)| std::cmp::Reverse(v.schema));
    all
}

/// The adapter for a catalog of `schema`: the one that lists it, else the one of the
/// nearest older schema (a newer Capture One mostly adds to a catalog), else the oldest.
pub(crate) fn choose(schema: i64) -> Chosen {
    let known = known();
    let (adapter, version) = known
        .iter()
        .find(|(_, v)| v.schema <= schema)
        .or(known.last())
        .copied()
        .expect("there is an adapter with a version");
    Chosen { adapter, exact: (version.schema == schema).then_some(version) }
}

/// Every version known, newest first.
pub(crate) fn versions() -> Vec<&'static Version> {
    known().into_iter().map(|(_, v)| v).collect()
}

/// The versions writes were checked against, newest first.
pub(crate) fn written() -> Vec<&'static Version> {
    known().into_iter().filter(|(a, _)| a.write() == Support::Tested).map(|(_, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_adapter_serves_several_versions() {
        let (new, old) = (choose(160906), choose(160800));
        assert_eq!(new.adapter.name(), old.adapter.name());
        assert_eq!(new.exact.map(|v| v.capture_one), Some("16.9"));
        assert_eq!(old.exact.map(|v| v.capture_one), Some("16.8"));
        assert_eq!((new.read(), new.write()), (Support::Tested, Support::Tested));
        let older = choose(1650);
        assert_ne!(older.adapter.name(), new.adapter.name());
        assert_eq!((older.adapter.name(), older.read(), older.write()), ("16.5", Support::Tested, Support::Off));
    }

    #[test]
    fn an_unknown_schema_takes_the_nearest_older_adapter_untested() {
        // Newer than all: the newest adapter.
        let newer = choose(170000);
        assert_eq!((newer.adapter.name(), newer.exact), (choose(160906).adapter.name(), None));
        assert_eq!((newer.read(), newer.write()), (Support::Untested, Support::Off));
        // Between two: the older one's, never one written for a schema that came later.
        assert_eq!(choose(160850).adapter.name(), choose(160800).adapter.name());
        assert_eq!(choose(160850).write(), Support::Off);
        assert_eq!(choose(2000).adapter.name(), "16.5");
        // Older than all: the oldest.
        assert_eq!((choose(1531).adapter.name(), choose(1531).exact), ("16.5", None));
    }

    #[test]
    fn every_schema_is_listed_once_and_the_newest_is_the_expected_one() {
        let all: Vec<i64> = versions().iter().map(|v| v.schema).collect();
        let mut unique = all.clone();
        unique.dedup();
        assert_eq!(all, unique, "a schema belongs to one adapter");
        assert!(all.is_sorted_by(|a, b| a > b));
        assert_eq!(all[0], crate::catalog::EXPECTED_ZVERSION);
        assert_eq!(written().iter().map(|v| v.schema).collect::<Vec<_>>(), [160906, 160800]);
    }

    #[test]
    fn the_adapter_for_16_5_reads_no_retouching_layers_and_the_current_one_counts_them() {
        let con = Connection::open_in_memory().unwrap();
        // No table at all: 16.5 does not ask for it.
        assert!(choose(1650).adapter.retouching_layers(&con).unwrap().is_empty());
        assert!(choose(160906).adapter.retouching_layers(&con).is_err());
        con.execute_batch(
            "CREATE TABLE ZRETOUCHINGLAYER (Z_PK INTEGER PRIMARY KEY, ZVARIANT INTEGER);
             INSERT INTO ZRETOUCHINGLAYER (ZVARIANT) VALUES (7), (7), (9);",
        )
        .unwrap();
        let layers = choose(160800).adapter.retouching_layers(&con).unwrap();
        assert_eq!((layers.get(&7), layers.get(&9), layers.len()), (Some(&2), Some(&1), 2));
    }
}
