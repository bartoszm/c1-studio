//! Which Capture One made a catalog, and what the app can do with it.
//!
//! `ZVERSIONINFO` keeps a row for every Capture One version that opened the catalog:
//! `ZAUTHOR` names it (`16.5.5.2701 Pro Win`) and `ZVERSION` is the catalog schema it
//! left (1650). The newest row is the catalog as it is now. Reading is tried for any
//! version and said to be untested when no adapter lists it ([`crate::adapter`]); writing
//! is only for a schema the writes were checked against.

use c1model::source::Result;
use c1model::{Compatibility, SourceError, Support};
use rusqlite::Connection;

use crate::{adapter, catalog};

/// A catalog's version history, as `ZVERSIONINFO` keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CatalogVersion {
    /// The schema now (the newest row).
    pub schema: i64,
    /// The Capture One that last opened it, as stored: `16.5.5.2701 Pro Win`.
    pub saved_by: Option<String>,
    /// The Capture One that made it (the oldest row).
    pub created_by: Option<String>,
}

pub(crate) fn read_version(con: &Connection) -> Result<CatalogVersion> {
    let row = |order: &str| {
        con.query_row(&format!("SELECT ZVERSION, ZAUTHOR FROM ZVERSIONINFO ORDER BY Z_PK {order} LIMIT 1"), [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })
    };
    let not_a_catalog =
        |e: rusqlite::Error| SourceError::Schema(format!("not a Capture One catalog (it has no version record: {e})"));
    let (schema, saved_by) = row("DESC").map_err(not_a_catalog)?;
    let (_, created_by) = row("ASC").map_err(not_a_catalog)?;
    Ok(CatalogVersion { schema, saved_by, created_by })
}

/// `16.5.5.2701 Pro Win` → `Capture One 16.5.5 Pro (Windows)`. Unknown parts are kept
/// as they are.
pub(crate) fn app_name(author: &str) -> String {
    let mut parts = author.split_whitespace();
    let version = parts.next().unwrap_or_default();
    let short: Vec<&str> = version.split('.').take(3).collect();
    let mut name = format!("Capture One {}", short.join("."));
    for part in parts {
        match part {
            "Win" => name.push_str(" (Windows)"),
            "Mac" => name.push_str(" (macOS)"),
            "ProF" => name.push_str(" Pro for Fujifilm"),
            other => {
                name.push(' ');
                name.push_str(other);
            }
        }
    }
    name
}

/// `items` as a list in a sentence: "a", "a and b", "a, b and c".
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    }
}

/// The versions the app knows, for messages: "16.9 (schema 160906), 16.8 (schema 160800)
/// and 16.5 (schema 1650)".
fn known_list() -> String {
    listed(&adapter::versions().map(|v| format!("{} (schema {})", v.capture_one, v.schema)).collect::<Vec<_>>())
}

/// The schemas writes were checked against, for messages: "160906 or 160800".
fn written_schemas() -> String {
    let items: Vec<String> = adapter::written().map(|v| v.schema.to_string()).collect();
    items.join(" or ")
}

/// What a write is about to rely on, for [`check_writes`].
#[derive(Clone, Copy, Debug)]
pub(crate) enum Checked {
    /// The catalog that is written.
    Catalog,
    /// A backup that is restored over it.
    Backup,
}

/// Refuses a catalog or a backup of a schema writes were not checked against, with its
/// schema and those that are written as the error.
pub(crate) fn check_writes(con: &Connection, what: Checked) -> Result<()> {
    let zversion = catalog::zversion(con)?;
    if adapter::choose(zversion).write() == Support::Tested {
        return Ok(());
    }
    let whose = match what {
        Checked::Catalog => "ZVERSION",
        Checked::Backup => "the backup's ZVERSION",
    };
    Err(SourceError::Schema(format!("{whose} is {zversion}, expected {}", written_schemas())))
}

pub(crate) fn assess(version: &CatalogVersion) -> Compatibility {
    let made = version.saved_by.as_deref().map(app_name).unwrap_or_else(|| "Capture One".to_owned());
    let mut format = format!("{made}, catalog schema {}", version.schema);
    if let Some(first) = version.created_by.as_deref().filter(|f| Some(*f) != version.saved_by.as_deref()) {
        format.push_str(&format!(" (created with {})", app_name(first)));
    }
    let writes = listed(&adapter::written().map(|v| v.capture_one.to_owned()).collect::<Vec<_>>());
    let chosen = adapter::choose(version.schema);
    let note = match chosen.exact {
        Some(_) if chosen.write() == Support::Tested => None,
        Some(v) => Some(format!(
            "Read and checked for Capture One {} catalogs. Writing is off: it is checked only for {writes} catalogs so far.",
            v.capture_one
        )),
        None => {
            let newest = adapter::versions().next().map_or(0, |v| v.schema);
            let when = if version.schema > newest { "newer than" } else { "not one of" };
            Some(format!(
                "This catalog's version is {when} those the app was checked against ({}). It is read the way \
                 Capture One {} catalogs are, and shows what it can; writing is off.",
                known_list(),
                chosen.adapter.name()
            ))
        }
    };
    let (read, write) = (chosen.read(), chosen.write());
    Compatibility { format, read, write, note }
}

/// A read that failed, said with the version, so it is clear what to report or do.
pub(crate) fn read_failed(version: &CatalogVersion, error: &SourceError) -> SourceError {
    let compat = assess(version);
    let tried = match compat.read {
        Support::Tested => "a version the app reads",
        _ => "a version the app was not checked against",
    };
    SourceError::Other(format!(
        "could not read this catalog ({}, {tried}): {error}. The app reads Capture One {}; opening the catalog in \
         one of those upgrades it.",
        compat.format,
        known_list()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(schema: i64, saved: &str, created: &str) -> CatalogVersion {
        CatalogVersion { schema, saved_by: Some(saved.into()), created_by: Some(created.into()) }
    }

    #[test]
    fn names_say_which_capture_one() {
        assert_eq!(app_name("16.5.5.2701 Pro Win"), "Capture One 16.5.5 Pro (Windows)");
        assert_eq!(app_name("12.0.2.13 ProF Win"), "Capture One 12.0.2 Pro for Fujifilm (Windows)");
        assert_eq!(app_name("23.1.0.1 Express Mac"), "Capture One 23.1.0 Express (macOS)");
    }

    #[test]
    fn what_is_said_of_known_versions_and_of_others() {
        // Which schemas are read and written is the adapters' to say (`adapter.rs`): here,
        // how it is worded.
        let newest = assess(&version(160906, "16.9.0.3894 Pro Win", "12.0.2.13 ProF Win"));
        assert_eq!(newest.note, None);
        assert!(
            newest.format.starts_with("Capture One 16.9.0 Pro (Windows), catalog schema 160906"),
            "{}",
            newest.format
        );
        let now = assess(&version(160800, "16.8.6.3784 Pro Win", "12.0.2.13 ProF Win"));
        assert_eq!(now.note, None);
        assert_eq!(
            now.format,
            "Capture One 16.8.6 Pro (Windows), catalog schema 160800 (created with Capture One 12.0.2 Pro for Fujifilm (Windows))"
        );
        let older = assess(&version(1650, "16.5.5.2701 Pro Win", "16.5.5.2701 Pro Win"));
        assert_eq!(older.format, "Capture One 16.5.5 Pro (Windows), catalog schema 1650");
        let note = older.note.unwrap();
        assert!(note.contains("Writing is off") && note.contains("only for 16.9 and 16.8 catalogs"), "{note}");
        let unknown = assess(&version(1531, "15.3.1.17 Pro Win", "15.3.1.17 Pro Win"));
        let note = unknown.note.unwrap();
        assert!(note.contains("not one of") && note.contains("the way Capture One 16.5 catalogs are"), "{note}");
        let note = assess(&version(170000, "17.0.0.1 Pro Win", "x")).note.unwrap();
        assert!(
            note.contains("newer than") && note.contains("the way Capture One 16.8 to 16.9 catalogs are"),
            "{note}"
        );
        let failed =
            read_failed(&version(1531, "15.3.1.17 Pro Win", "15.3.1.17 Pro Win"), &SourceError::Schema("x".into()));
        assert!(failed.to_string().contains("Capture One 15.3.1 Pro (Windows), catalog schema 1531"), "{failed}");
        assert!(
            failed.to_string().contains("16.9 (schema 160906), 16.8 (schema 160800) and 16.5 (schema 1650)"),
            "{failed}"
        );
    }

    #[test]
    fn assess_passes_on_what_the_adapter_says() {
        for schema in [160906, 160800, 1650, 1531, 160700, 170000] {
            let (compat, chosen) = (assess(&version(schema, "x", "x")), adapter::choose(schema));
            assert_eq!((compat.read, compat.write), (chosen.read(), chosen.write()), "{schema}");
        }
    }

    #[test]
    fn a_write_is_refused_with_the_schema_and_those_written() {
        let con = Connection::open_in_memory().unwrap();
        con.execute_batch(
            "CREATE TABLE ZVERSIONINFO (Z_PK INTEGER PRIMARY KEY, ZVERSION INTEGER);
             INSERT INTO ZVERSIONINFO (ZVERSION) VALUES (1650);",
        )
        .unwrap();
        let refusal = |what| check_writes(&con, what).unwrap_err();
        assert!(matches!(refusal(Checked::Catalog), SourceError::Schema(_)));
        assert!(refusal(Checked::Catalog).to_string().ends_with("ZVERSION is 1650, expected 160906 or 160800"));
        assert!(!refusal(Checked::Catalog).to_string().contains("backup"));
        assert!(
            refusal(Checked::Backup).to_string().ends_with("the backup's ZVERSION is 1650, expected 160906 or 160800")
        );
        // The newest row is the catalog now: a newer Capture One upgraded it.
        con.execute("INSERT INTO ZVERSIONINFO (ZVERSION) VALUES (160800)", []).unwrap();
        assert!(check_writes(&con, Checked::Catalog).is_ok() && check_writes(&con, Checked::Backup).is_ok());
        con.execute("INSERT INTO ZVERSIONINFO (ZVERSION) VALUES (170000)", []).unwrap();
        assert!(refusal(Checked::Catalog).to_string().ends_with("ZVERSION is 170000, expected 160906 or 160800"));
    }
}
