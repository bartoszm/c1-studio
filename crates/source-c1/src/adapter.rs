//! One adapter per family of Capture One catalog schemas.
//!
//! Capture One changes its catalog between releases: 16.8 added `ZRETOUCHINGLAYER`, 16.9 a
//! table of faces and columns nothing here uses. What differs in reading between the
//! schemas the app knows is kept here, behind [`Adapter`], and the adapter is chosen by the
//! catalog's schema number ([`choose`]). The reader asks the adapter and does not look at
//! the number.
//!
//! - An adapter lists every schema it was checked against ([`Adapter::versions`]): one
//!   adapter serves several Capture One versions while they differ in nothing it touches.
//! - Each method of the trait has the newest schema's way as its default. An adapter for
//!   an older or a newer schema overrides only what is different there.
//! - A schema no adapter lists is read with the adapter of the nearest older one, as
//!   untested, and is not written. Where the catalog itself can say what it has, it is
//!   asked instead of the adapter ([`Chosen::retouching_layers`]).
//!
//! Writes are only switched on or off per adapter today ([`Adapter::write`]): `write.rs`
//! reads the schema number and refuses a catalog whose adapter does not write
//! ([`crate::compat::check_writes`]). The write SQL itself is not behind the adapter, and
//! neither are the entity numbers (`Z_ENT`): they are constants and literals in
//! `keywords.rs`, `albums.rs` and `extract.rs`. They are the same in every schema written
//! (16.8 and 16.9), and become methods of the trait when a schema changes them.
//!
//! To support another Capture One version: when it differs in nothing used here, add its
//! schema to the adapter that fits and run `tools/difftest` on such a catalog. When it
//! differs, add an adapter that overrides the methods concerned (and a method to the
//! trait for a difference that has none yet, with today's SQL as its default), then list
//! it in [`ADAPTERS`].

use std::collections::HashMap;
use std::sync::LazyLock;

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
    /// The schemas it was checked against, newest first.
    fn versions(&self) -> &'static [Version];

    /// Whether writes were checked against these schemas.
    fn write(&self) -> Support;

    /// What it is called in messages and logs: the Capture One versions it serves, oldest
    /// to newest ("16.8 to 16.9", or "16.5" for one).
    fn name(&self) -> String {
        match self.versions() {
            [] => String::new(),
            [one] => one.capture_one.to_owned(),
            [newest, .., oldest] => format!("{} to {}", oldest.capture_one, newest.capture_one),
        }
    }

    /// How many retouching layers each variant has, by variant.
    fn retouching_layers(&self, con: &Connection) -> Result<HashMap<i64, u32>> {
        count_retouching_layers(con)
    }
}

/// The rows of `ZRETOUCHINGLAYER`, counted by variant.
fn count_retouching_layers(con: &Connection) -> Result<HashMap<i64, u32>> {
    let mut layers = HashMap::new();
    let mut stmt = con.prepare("SELECT ZVARIANT FROM ZRETOUCHINGLAYER").map_err(db_err)?;
    let mut rows = stmt.query([]).map_err(db_err)?;
    while let Some(r) = rows.next().map_err(db_err)? {
        *layers.entry(r.get(0).map_err(db_err)?).or_default() += 1;
    }
    Ok(layers)
}

fn has_table(con: &Connection, name: &str) -> Result<bool> {
    con.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)", [name], |r| {
        r.get(0)
    })
    .map_err(db_err)
}

/// Capture One 16.8 and 16.9: the catalog the reader and the differential write tests were
/// built on, and the same catalog after 16.9 upgraded it (a `ZFACE` table and columns on
/// `ZIMAGE`, `ZVARIANTLAYER` and `ZRETOUCHINGLAYER` were added; none this code reads or
/// writes was changed, and the differential read and write tests pass on it).
struct Current;

impl Adapter for Current {
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

/// Every schema known with its adapter, newest first. Built once: the adapters do not
/// change while the app runs.
static KNOWN: LazyLock<Vec<(&'static dyn Adapter, &'static Version)>> = LazyLock::new(|| {
    let mut all: Vec<_> = ADAPTERS.iter().flat_map(|&a| a.versions().iter().map(move |v| (a, v))).collect();
    all.sort_by_key(|(_, v)| std::cmp::Reverse(v.schema));
    all
});

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

    /// How many retouching layers each variant has, by variant. A schema the adapter
    /// lists is read the adapter's way. For any other the adapter is a guess, so the
    /// catalog is asked: the layers are counted when it has the table, and there are none
    /// when it has not.
    pub(crate) fn retouching_layers(&self, con: &Connection) -> Result<HashMap<i64, u32>> {
        if self.exact.is_some() {
            self.adapter.retouching_layers(con)
        } else if has_table(con, "ZRETOUCHINGLAYER")? {
            count_retouching_layers(con)
        } else {
            Ok(HashMap::new())
        }
    }
}

/// The adapter for a catalog of `schema`: the one that lists it, else the one of the
/// nearest older schema (a newer Capture One mostly adds to a catalog), else the oldest.
pub(crate) fn choose(schema: i64) -> Chosen {
    let &(adapter, version) =
        KNOWN.iter().find(|(_, v)| v.schema <= schema).or(KNOWN.last()).expect("there is an adapter with a version");
    Chosen { adapter, exact: (version.schema == schema).then_some(version) }
}

/// Every version known, newest first.
pub(crate) fn versions() -> impl Iterator<Item = &'static Version> {
    KNOWN.iter().map(|&(_, v)| v)
}

/// The versions writes were checked against, newest first.
pub(crate) fn written() -> impl Iterator<Item = &'static Version> {
    KNOWN.iter().filter(|(a, _)| a.write() == Support::Tested).map(|&(_, v)| v)
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
        assert_eq!((old.read(), old.write()), (Support::Tested, Support::Tested));
        let older = choose(1650);
        assert_ne!(older.adapter.name(), new.adapter.name());
        assert_eq!(older.exact.map(|v| v.capture_one), Some("16.5"));
        assert_eq!((older.read(), older.write()), (Support::Tested, Support::Off));
    }

    #[test]
    fn an_adapter_is_named_by_its_versions_oldest_to_newest() {
        assert_eq!(choose(160906).adapter.name(), "16.8 to 16.9");
        assert_eq!(choose(1650).adapter.name(), "16.5");
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
        let oldest = choose(1531);
        assert_eq!((oldest.adapter.name().as_str(), oldest.exact), ("16.5", None));
        assert_eq!((oldest.read(), oldest.write()), (Support::Untested, Support::Off));
    }

    #[test]
    fn every_schema_is_listed_once_newest_first() {
        let all: Vec<i64> = versions().map(|v| v.schema).collect();
        assert_eq!(all, [160906, 160800, 1650]);
        assert!(all.is_sorted_by(|a, b| a > b), "newest first, and a schema belongs to one adapter");
        assert_eq!(written().map(|v| v.schema).collect::<Vec<_>>(), [160906, 160800]);
    }

    const RETOUCHED: &str = "CREATE TABLE ZRETOUCHINGLAYER (Z_PK INTEGER PRIMARY KEY, ZVARIANT INTEGER);
         INSERT INTO ZRETOUCHINGLAYER (ZVARIANT) VALUES (7), (7), (9);";

    #[test]
    fn a_listed_schema_reads_retouching_layers_the_way_its_adapter_does() {
        let con = Connection::open_in_memory().unwrap();
        // No table at all: 16.5 does not ask for it, and 16.8 and 16.9 must have it.
        assert!(choose(1650).retouching_layers(&con).unwrap().is_empty());
        assert!(choose(160906).retouching_layers(&con).is_err());
        assert!(choose(160800).retouching_layers(&con).is_err());
        con.execute_batch(RETOUCHED).unwrap();
        for schema in [160906, 160800] {
            let layers = choose(schema).retouching_layers(&con).unwrap();
            assert_eq!((layers.get(&7), layers.get(&9), layers.len()), (Some(&2), Some(&1), 2));
        }
        assert!(choose(1650).retouching_layers(&con).unwrap().is_empty());
    }

    #[test]
    fn an_unlisted_schema_reads_the_retouching_layers_its_catalog_has() {
        let con = Connection::open_in_memory().unwrap();
        // No table: none, also for a schema newer than all, whose adapter would ask for it.
        for schema in [1531, 160700, 160850, 170000] {
            assert!(choose(schema).retouching_layers(&con).unwrap().is_empty(), "{schema}");
        }
        con.execute_batch(RETOUCHED).unwrap();
        // With the table: counted, also between 16.5 and 16.8, where the adapter is the one
        // of 16.5, which reads none.
        for schema in [1531, 160700, 160850, 170000] {
            let layers = choose(schema).retouching_layers(&con).unwrap();
            assert_eq!((layers.get(&7), layers.get(&9), layers.len()), (Some(&2), Some(&1), 2), "{schema}");
        }
    }
}
