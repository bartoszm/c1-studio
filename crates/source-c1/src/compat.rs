//! Which Capture One made a catalog, and what the app can do with it.
//!
//! `ZVERSIONINFO` keeps a row for every Capture One version that opened the catalog:
//! `ZAUTHOR` names it (`16.5.5.2701 Pro Win`) and `ZVERSION` is the catalog schema it
//! left (1650). The newest row is the catalog as it is now. Reading is tried for any
//! version and said to be untested when it is not one below; writing is only for a
//! schema the writes were checked against.

use c1model::source::Result;
use c1model::{Compatibility, SourceError, Support};
use rusqlite::Connection;

use crate::catalog::EXPECTED_ZVERSION;

/// Schemas the app has been checked against, newest first.
struct Known {
    schema: i64,
    capture_one: &'static str,
    read: Support,
    write: Support,
}

const KNOWN: &[Known] = &[
    // The real catalog the reader and the differential write tests were built on.
    Known { schema: EXPECTED_ZVERSION, capture_one: "16.8", read: Support::Tested, write: Support::Tested },
    // A real 16.5 catalog: read and checked (it lacks ZRETOUCHINGLAYER); not written.
    Known { schema: 1650, capture_one: "16.5", read: Support::Tested, write: Support::Off },
];

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

/// The versions the app knows, for messages: "16.8 (schema 160800) and 16.5 (schema 1650)".
fn known_list() -> String {
    let items: Vec<String> = KNOWN.iter().map(|k| format!("{} (schema {})", k.capture_one, k.schema)).collect();
    items.join(" and ")
}

pub(crate) fn assess(version: &CatalogVersion) -> Compatibility {
    let made = version.saved_by.as_deref().map(app_name).unwrap_or_else(|| "Capture One".to_owned());
    let mut format = format!("{made}, catalog schema {}", version.schema);
    if let Some(first) = version.created_by.as_deref().filter(|f| Some(*f) != version.saved_by.as_deref()) {
        format.push_str(&format!(" (created with {})", app_name(first)));
    }
    let writes = KNOWN.iter().find(|k| k.write == Support::Tested).map_or("", |k| k.capture_one);
    let (read, write, note) = match KNOWN.iter().find(|k| k.schema == version.schema) {
        Some(k) if k.write == Support::Tested => (k.read, k.write, None),
        Some(k) => (
            k.read,
            k.write,
            Some(format!(
                "Read and checked for Capture One {} catalogs. Writing is off: it is checked only for {writes} catalogs so far.",
                k.capture_one
            )),
        ),
        None => {
            let newest = KNOWN.iter().map(|k| k.schema).max().unwrap_or_default();
            let when = if version.schema > newest { "newer than" } else { "not one of" };
            (
                Support::Untested,
                Support::Off,
                Some(format!(
                    "This catalog's version is {when} those the app was checked against ({}). It shows what it can \
                     read; writing is off.",
                    known_list()
                )),
            )
        }
    };
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
    fn known_versions_and_others() {
        let now = assess(&version(160800, "16.8.6.3784 Pro Win", "12.0.2.13 ProF Win"));
        assert_eq!((now.read, now.write, now.note), (Support::Tested, Support::Tested, None));
        assert_eq!(
            now.format,
            "Capture One 16.8.6 Pro (Windows), catalog schema 160800 (created with Capture One 12.0.2 Pro for Fujifilm (Windows))"
        );
        let older = assess(&version(1650, "16.5.5.2701 Pro Win", "16.5.5.2701 Pro Win"));
        assert_eq!((older.read, older.write), (Support::Tested, Support::Off));
        assert_eq!(older.format, "Capture One 16.5.5 Pro (Windows), catalog schema 1650");
        assert!(older.note.unwrap().contains("Writing is off"));
        let unknown = assess(&version(1531, "15.3.1.17 Pro Win", "15.3.1.17 Pro Win"));
        assert_eq!((unknown.read, unknown.write), (Support::Untested, Support::Off));
        assert!(unknown.note.as_deref().unwrap().contains("not one of"));
        assert!(assess(&version(170000, "17.0.0.1 Pro Win", "x")).note.unwrap().contains("newer than"));
        let failed =
            read_failed(&version(1531, "15.3.1.17 Pro Win", "15.3.1.17 Pro Win"), &SourceError::Schema("x".into()));
        assert!(failed.to_string().contains("Capture One 15.3.1 Pro (Windows), catalog schema 1531"), "{failed}");
    }
}
