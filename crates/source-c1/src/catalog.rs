//! Opening the catalog safely.
//!
//! Capture One caches the catalog in memory and rewrites it on save, so we never hold it
//! open for long: a snapshot reads from a private copy, and point lookups open the live
//! file read-only for a moment.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use c1model::source::Result;
use c1model::{Cursor, SourceError};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::config;

/// The schema this code was verified against (`ZVERSIONINFO.ZVERSION`, Capture One 16.8).
pub const EXPECTED_ZVERSION: i64 = 160800;

pub(crate) fn db_err(e: rusqlite::Error) -> SourceError {
    SourceError::Other(format!("catalog: {e}"))
}

fn uri(path: &Path, extra: &str) -> String {
    // SQLite URIs want forward slashes; a Windows drive path becomes file:/C:/...
    let p = path.to_string_lossy().replace('\\', "/");
    let p = if p.starts_with('/') { p } else { format!("/{p}") };
    format!("file:{p}?mode=ro{extra}")
}

/// The live catalog, read-only, for short lookups. Waits briefly if Capture One is
/// mid-write rather than failing.
pub(crate) fn open_live(catalog: &Path) -> Result<Connection> {
    let con = Connection::open_with_flags(
        uri(catalog, ""),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db_err)?;
    con.busy_timeout(Duration::from_secs(2)).map_err(db_err)?;
    Ok(con)
}

/// Our private copy. `immutable` tells SQLite nobody else touches it, so it skips locking.
pub(crate) fn open_copy(copy: &Path) -> Result<Connection> {
    Connection::open_with_flags(
        uri(copy, "&immutable=1"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db_err)
}

/// `ZVERSIONINFO` keeps one row per Capture One version that ever opened the catalog;
/// the current schema is the newest row.
pub(crate) fn zversion(con: &Connection) -> Result<i64> {
    con.query_row("SELECT ZVERSION FROM ZVERSIONINFO ORDER BY Z_PK DESC LIMIT 1", [], |r| r.get(0))
        .map_err(|e| SourceError::Schema(format!("no ZVERSIONINFO: {e}")))
}

/// Modification time and size: changes whenever Capture One saves.
pub(crate) fn file_cursor(catalog: &Path) -> Result<Cursor> {
    let meta = fs::metadata(catalog)?;
    let mtime = meta.modified()?.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    Ok(Cursor(format!("{mtime}:{}", meta.len())))
}

/// A copy deleted when dropped.
pub(crate) struct TempCopy(PathBuf);

impl TempCopy {
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Copies the catalog, retrying if it changed while we copied or if a hot journal
/// shows Capture One is mid-transaction. A torn copy would be silently wrong data.
pub(crate) fn copy_for_reading(catalog: &Path, work_dir: &Path) -> Result<TempCopy> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fs::create_dir_all(work_dir)?;
    // Unique per copy: several sources (or tests) in one process may share a work dir.
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let target = TempCopy(work_dir.join(format!("snapshot-{}-{n}.cocatalogdb", std::process::id())));
    let journal = catalog.with_extension("cocatalogdb-journal");
    for attempt in 0..5 {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(500 * attempt));
        }
        if fs::metadata(&journal).map(|m| m.len() > 0).unwrap_or(false) {
            continue;
        }
        let before = file_cursor(catalog)?;
        fs::copy(catalog, target.path())?;
        if file_cursor(catalog)? == before {
            return Ok(target);
        }
    }
    Err(SourceError::Other("the catalog kept changing while it was copied; try again".into()))
}

/// Where the original file lives, in Windows form.
pub(crate) struct PreviewSource {
    pub sidecar: String,
    pub white_balance: Option<[f32; 3]>,
    /// Clockwise degrees, a multiple of 90.
    pub rotation: u16,
    /// Black and white is switched on, or the film curve is a monochrome simulation.
    pub monochrome: bool,
    /// Clockwise degrees beyond `rotation`.
    pub straighten: f32,
    pub crop: Option<c1model::Crop>,
}

/// Where the variant's `.cop` preview should be, and how to render it.
///
/// White balance comes from the combined layer when it has one: that is what Capture One
/// renders, styles included, and across 300 sampled variants it matched Capture One's own
/// thumbnails best (48% closest; the default layer, 25%).
///
/// The preview is the whole sensor image as shot: no crop, straightening or perspective
/// fix (600 sampled variants, 400 of them cropped; the thumbnail is straightened and
/// fixed but not cropped either). `ZROTATION` on the combined layer is the clockwise turn
/// Capture One shows it with, the user's own rotation included; what is left beyond the
/// quarter turn is the straightening. `ZCROP` is applied after both; without one, a
/// straightened picture is cut to what the turn fills.
///
/// Black and white is either `ZBWENABLED` or a monochrome film simulation (Fuji ACROS,
/// Monochrome, Sepia) as the film curve. Neither catches a raw shot in ACROS whose curve
/// is "Auto": the simulation is then read from the raw file, which the catalog does not
/// record. Capture One's thumbnail shows it, so callers should check that as well.
pub(crate) fn preview_source(con: &Connection, variant_pk: i64) -> Result<Option<PreviewSource>> {
    con.query_row(
        "SELECT i.ZSIDECARPATH, c.ZROTATION, c.ZBWENABLED, c.ZFILMCURVE,
                c.ZWHITEBALANCE, a.ZWHITEBALANCE, d.ZWHITEBALANCE,
                c.ZCROP, i.ZWIDTH, i.ZHEIGHT
           FROM ZVARIANT v
           JOIN ZIMAGE i             ON i.Z_PK = v.ZIMAGE
           LEFT JOIN ZVARIANTLAYER c ON c.Z_PK = v.ZCOMBINEDSETTINGS
           LEFT JOIN ZVARIANTLAYER a ON a.Z_PK = v.ZADJUSTMENTLAYER
           LEFT JOIN ZVARIANTLAYER d ON d.Z_PK = v.ZDEFAULTLAYER
          WHERE v.Z_PK = ?",
        [variant_pk],
        |r| {
            let sidecar = r.get::<_, Option<String>>(0)?.unwrap_or_default();
            let degrees = r.get::<_, Option<f64>>(1)?.unwrap_or(0.0);
            let rotation = config::quarter_turn(degrees);
            let curve = r.get::<_, Option<String>>(3)?.unwrap_or_default().to_lowercase();
            let monochrome = r.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0
                || ["acros", "monochrome", "sepia"].iter().any(|k| curve.contains(k));
            let mut white_balance = None;
            for column in 4..=6 {
                if let Some(wb) = r.get::<_, Option<String>>(column)?.as_deref().and_then(config::parse_white_balance) {
                    white_balance = Some(wb);
                    break;
                }
            }
            let (width, height) =
                (r.get::<_, Option<f64>>(8)?.unwrap_or(0.0), r.get::<_, Option<f64>>(9)?.unwrap_or(0.0));
            let crop = r
                .get::<_, Option<String>>(7)?
                .and_then(|c| config::display_crop(&c, width, height, degrees))
                .or_else(|| config::straightened_crop(width, height, degrees));
            let straighten = config::straightening(degrees);
            Ok(PreviewSource { sidecar, white_balance, rotation, monochrome, straighten, crop })
        },
    )
    .optional()
    .map_err(db_err)
}

/// The crop of every variant that has one (its own, or the one straightening makes),
/// measured for its `.cot` thumbnail, which is straightened onto a grown canvas already.
pub(crate) fn thumbnail_crops(con: &Connection) -> Result<std::collections::HashMap<i64, c1model::Crop>> {
    let mut statement = con
        .prepare(
            "SELECT v.Z_PK, c.ZCROP, c.ZROTATION, i.ZWIDTH, i.ZHEIGHT
               FROM ZVARIANT v
               JOIN ZIMAGE i             ON i.Z_PK = v.ZIMAGE
               JOIN ZVARIANTLAYER c ON c.Z_PK = v.ZCOMBINEDSETTINGS
              WHERE c.ZCROP IS NOT NULL OR (c.ZROTATION IS NOT NULL AND c.ZROTATION <> 0)",
        )
        .map_err(db_err)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
            ))
        })
        .map_err(db_err)?;
    let mut crops = std::collections::HashMap::new();
    for row in rows {
        let (vpk, crop, degrees, width, height) = row.map_err(db_err)?;
        let crop = crop
            .and_then(|c| config::display_crop(&c, width, height, degrees))
            .or_else(|| config::straightened_crop(width, height, degrees));
        if let Some(crop) = crop {
            crops.insert(vpk, config::on_canvas(crop, width, height, degrees));
        }
    }
    Ok(crops)
}

pub(crate) fn original_path(con: &Connection, catalog_dir: &Path, variant_pk: i64) -> Result<Option<String>> {
    con.query_row(
        "SELECT p.ZWINROOT, p.ZRELATIVEPATH, p.ZISRELATIVE, i.ZIMAGEFILENAME
           FROM ZVARIANT v
           JOIN ZIMAGE i        ON i.Z_PK = v.ZIMAGE
           JOIN ZPATHLOCATION p ON p.Z_PK = i.ZIMAGELOCATION
          WHERE v.Z_PK = ?",
        [variant_pk],
        |r| {
            Ok(config::original_path(
                &r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                &r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                &r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                r.get::<_, Option<bool>>(2)?.unwrap_or(false),
                catalog_dir,
            ))
        },
    )
    .optional()
    .map_err(db_err)
}
