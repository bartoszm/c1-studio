//! Writing to the catalog, ported from `legacy/c1cat-py/c1cat/safety.py` and `cmd_meta.py`.
//!
//! Capture One keeps the whole catalog in memory while it runs and rewrites it on save,
//! so an edit made underneath it is silently lost. The only safe move is to refuse:
//! a write needs Capture One closed and nobody else holding the file. Before the first
//! write the catalog is backed up, and every write runs in one transaction that first
//! re-checks each change against what the catalog holds at that moment.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use c1model::change::{Check, compare, membership_problem, spot};
use c1model::source::Result;
use c1model::{
    ApplyPlan, Change, CollectionChange, ColorLabel, Conflict, Iptc, KeywordRemap, Op, Place, SourceError, WriteStatus,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::catalog::{self, EXPECTED_ZVERSION, db_err};
use crate::extract::blank;
use crate::{albums, config, keywords, process};

/// Backups kept per catalog; older ones are deleted after a new one is made.
pub(crate) const KEEP_BACKUPS: usize = 5;

/// Writes the app makes in a row share one backup for this long, as long as nothing
/// else touched the catalog in between (autosave writes often; each backup copies the
/// whole catalog). The backup holds the catalog from before the first of them.
const SHARED_BACKUP_MS: u64 = 3_600_000;

/// Whether a write could start now, cheapest checks first.
pub(crate) fn status(catalog: &Path) -> WriteStatus {
    if let Some(name) = process::running_capture_one() {
        return WriteStatus::Locked { by: name };
    }
    if journal_present(catalog) {
        return WriteStatus::Locked { by: "a program in the middle of a write (a rollback journal is present)".into() };
    }
    if held_elsewhere(catalog) {
        return WriteStatus::Locked { by: "another program that has the catalog open".into() };
    }
    match structure(catalog) {
        Ok(()) => WriteStatus::Ready,
        Err(_) => WriteStatus::ReadOnly,
    }
}

/// The checks that do not depend on who else is running: the journal mode Capture One
/// uses, and the schema version every column meaning here was verified against.
fn structure(catalog: &Path) -> Result<()> {
    let con = catalog::open_live(catalog)?;
    let mode: String = con.query_row("PRAGMA journal_mode", [], |r| r.get(0)).map_err(db_err)?;
    if !mode.eq_ignore_ascii_case("delete") {
        return Err(SourceError::Schema(format!("journal mode is {mode:?}, expected \"delete\"")));
    }
    let zversion = catalog::zversion(&con)?;
    if zversion != EXPECTED_ZVERSION {
        return Err(SourceError::Schema(format!("ZVERSION is {zversion}, expected {EXPECTED_ZVERSION}")));
    }
    Ok(())
}

/// Refuses unless [`status`] says ready, with the reason as the error.
pub(crate) fn preflight(catalog: &Path) -> Result<()> {
    match status(catalog) {
        WriteStatus::Ready => Ok(()),
        WriteStatus::Locked { by } => Err(SourceError::Locked { by }),
        WriteStatus::ReadOnly => structure(catalog),
    }
}

fn journal_present(catalog: &Path) -> bool {
    let mut journal = catalog.as_os_str().to_owned();
    journal.push("-journal");
    fs::metadata(journal).is_ok_and(|m| m.len() > 0)
}

/// Can we open the file with no sharing at all? Capture One holds it open while it
/// runs, and so does any other program reading it, even read-only.
#[cfg(windows)]
fn held_elsewhere(catalog: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new().read(true).write(true).share_mode(0).open(catalog).is_err()
}

#[cfg(not(windows))]
fn held_elsewhere(_catalog: &Path) -> bool {
    // POSIX locks do not interlock with Windows ones; the process check is the gate.
    false
}

// ---------------------------------------------------------------------------------------
// Reading and writing fields
// ---------------------------------------------------------------------------------------

/// The writable fields as the catalog holds them now, read the way `extract` reads them.
#[derive(Debug, PartialEq)]
struct Current {
    rating: Option<u8>,
    color: Option<ColorLabel>,
    iptc: Iptc,
    place: Place,
    /// GPS is on the image, shared by its variants.
    geo: Option<(f64, f64)>,
}

/// The text fields' columns on ZVARIANTMETADATA, in the order of `iptc_fields` and
/// `place_fields`.
const IPTC_COLUMNS: [&str; 8] = [
    "ZCONTENT_DESCRIPTION",
    "ZCONTENT_HEADLINE",
    "ZSTATUS_TITLE",
    "ZCONTACT_CREATOR",
    "ZSTATUS_COPYRIGHTNOTICE",
    "ZSTATUS_JOBIDENTIFIER",
    "ZSTATUS_INSTRUCTIONS",
    "ZSTATUS_SOURCE",
];
const PLACE_COLUMNS: [&str; 5] =
    ["ZIMAGE_CITY", "ZIMAGE_STATE", "ZIMAGE_COUNTRY", "ZIMAGE_ISOCOUNTRYCODE", "ZIMAGE_LOCATION"];

fn iptc_fields(i: &Iptc) -> [&Option<String>; 8] {
    [&i.description, &i.headline, &i.title, &i.creator, &i.copyright, &i.job_id, &i.instructions, &i.source]
}
fn place_fields(p: &Place) -> [&Option<String>; 5] {
    [&p.city, &p.state, &p.country, &p.country_code, &p.location]
}

fn current(con: &Connection, variant_pk: i64) -> Result<Option<Current>> {
    con.prepare_cached(
        "SELECT m.ZBASIC_RATING, m.ZCOLOR_TAG_INDEX,
                m.ZCONTENT_DESCRIPTION, m.ZCONTENT_HEADLINE, m.ZSTATUS_TITLE, m.ZCONTACT_CREATOR,
                m.ZSTATUS_COPYRIGHTNOTICE,
                m.ZIMAGE_CITY, m.ZIMAGE_STATE, m.ZIMAGE_COUNTRY, m.ZIMAGE_ISOCOUNTRYCODE, m.ZIMAGE_LOCATION,
                i.ZGPSLATITUDE, i.ZGPSLONGITUDE,
                m.ZSTATUS_JOBIDENTIFIER, m.ZSTATUS_INSTRUCTIONS, m.ZSTATUS_SOURCE
           FROM ZVARIANT v
           JOIN ZVARIANTMETADATA m ON m.ZLAYER = v.ZADJUSTMENTLAYER
           JOIN ZIMAGE i ON i.Z_PK = v.ZIMAGE
          WHERE v.Z_PK = ?1",
    )
    .and_then(|mut s| {
        s.query_row([variant_pk], |r| {
            let rating: Option<i64> = r.get(0)?;
            let color: Option<i64> = r.get(1)?;
            let text = |i: usize| r.get::<_, Option<String>>(i).map(blank);
            let lat = text(12)?;
            let lon = text(13)?;
            Ok(Current {
                rating: rating.filter(|&v| v > 0).map(|v| v.clamp(1, 5) as u8),
                color: color.and_then(|v| u8::try_from(v).ok()).and_then(ColorLabel::from_index),
                iptc: Iptc {
                    description: text(2)?,
                    headline: text(3)?,
                    title: text(4)?,
                    creator: text(5)?,
                    copyright: text(6)?,
                    job_id: text(14)?,
                    instructions: text(15)?,
                    source: text(16)?,
                },
                place: Place {
                    city: text(7)?,
                    state: text(8)?,
                    country: text(9)?,
                    country_code: text(10)?,
                    location: text(11)?,
                },
                geo: match (config::parse_gps(lat.as_deref()), config::parse_gps(lon.as_deref())) {
                    (Some(lat), Some(lon)) => Some((lat, lon)),
                    _ => None,
                },
            })
        })
        .optional()
    })
    .map_err(db_err)
}

/// Sorts changes into apply / conflict / no-op against the catalog behind `con`, as it
/// will be once `remaps` (queued keyword edits name the new paths) and `collections`
/// (photos may go into an album made there) have been applied.
pub(crate) fn plan(
    con: &Connection,
    remaps: &[KeywordRemap],
    collections: &[CollectionChange],
    changes: &[Change],
    variant_pk: impl Fn(&Change) -> Result<i64>,
) -> Result<ApplyPlan> {
    let mut plan = ApplyPlan::default();
    let mut albums: Option<Vec<c1model::Collection>> = None;
    for change in changes {
        let check = match current(con, variant_pk(change)?)? {
            None => Check::Conflict("deleted".into()),
            Some(now) => match &change.op {
                Op::SetRating { base, value } => compare(base, value, &now.rating),
                Op::SetColor { base, value } => compare(base, value, &now.color),
                Op::SetIptc { base, value } => compare(base, value, &now.iptc),
                Op::SetPlace { base, value } => compare(base, value, &now.place),
                // Only the position: Capture One keeps no altitude or origin.
                Op::SetGeo { base, value } => compare(&spot(base), &spot(value), &now.geo),
                Op::AddKeywords { paths } | Op::RemoveKeywords { paths } => {
                    let stored: Vec<c1model::KeywordPath> = keywords::stored(con, variant_pk(change)?)?
                        .into_iter()
                        .map(|(p, _)| remaps.iter().fold(c1model::KeywordPath::from_pipe(&p), |p, r| r.path(&p)))
                        .collect();
                    let has = |p: &c1model::KeywordPath| stored.contains(p);
                    let adding = matches!(change.op, Op::AddKeywords { .. });
                    for path in paths.iter().filter(|_| adding) {
                        keywords::validate(&path.to_pipe())?;
                    }
                    // Sets of keywords do not conflict: adding one that is there, or
                    // removing one that is not, is simply already done.
                    match (adding, paths.iter().all(has), paths.iter().any(has)) {
                        (true, true, _) | (false, _, false) => Check::NoOp,
                        _ => Check::Apply,
                    }
                }
                Op::AddToCollection { collection } | Op::RemoveFromCollection { collection } => {
                    let albums = match &mut albums {
                        Some(a) => a,
                        None => albums.insert(albums::after(con, collections)?),
                    };
                    match membership_problem(albums, collection) {
                        Some(why) => Check::Conflict(why),
                        None => {
                            // An album made in this write has no members yet.
                            let member = match albums::resolve(con, collection)? {
                                Some(pk) => albums::is_member(con, pk, variant_pk(change)?)?,
                                None => false,
                            };
                            let adding = matches!(change.op, Op::AddToCollection { .. });
                            if adding == member { Check::NoOp } else { Check::Apply }
                        }
                    }
                }
            },
        };
        match check {
            Check::Apply => plan.apply.push(change.clone()),
            Check::NoOp => plan.no_ops.push(change.clone()),
            Check::Conflict(current) => plan.conflicts.push(Conflict { change: change.clone(), current }),
        }
    }
    Ok(plan)
}

/// The two metadata rows a write must touch. User metadata lives on the adjustment
/// layer and is mirrored onto the combined layer; the default layer holds what was
/// embedded in the file and is never written.
fn metadata_rows(con: &Connection, variant_pk: i64) -> Result<Vec<i64>> {
    let mut stmt = con
        .prepare_cached(
            "SELECT ZMETADATA FROM ZVARIANTLAYER WHERE Z_PK IN (
               SELECT ZADJUSTMENTLAYER FROM ZVARIANT WHERE Z_PK = ?1
               UNION ALL
               SELECT ZCOMBINEDSETTINGS FROM ZVARIANT WHERE Z_PK = ?1)",
        )
        .map_err(db_err)?;
    let rows = stmt.query_map([variant_pk], |r| r.get::<_, Option<i64>>(0)).map_err(db_err)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_err)?.into_iter().flatten().collect())
}

/// Writes one change to both metadata rows. Returns the rows touched.
fn write_change(con: &Connection, variant_pk: i64, op: &Op) -> Result<usize> {
    // Capture One stores "unrated" and "no colour" as NULL, not 0.
    let (column, value): (&str, Option<i64>) = match op {
        Op::SetRating { value, .. } => match value {
            Some(v @ 1..=5) => ("ZBASIC_RATING", Some(i64::from(*v))),
            Some(0) | None => ("ZBASIC_RATING", None),
            Some(v) => return Err(SourceError::Other(format!("rating {v} is outside 0..=5"))),
        },
        Op::SetColor { value, .. } => ("ZCOLOR_TAG_INDEX", value.map(|c| i64::from(c.index()))),
        Op::AddKeywords { paths } | Op::RemoveKeywords { paths } => {
            let paths: Vec<String> = paths.iter().map(|p| p.to_pipe()).collect();
            let before = keywords::stored(con, variant_pk)?;
            let after = if matches!(op, Op::AddKeywords { .. }) {
                for path in paths.iter().filter(|p| !before.iter().any(|(s, _)| s == *p)) {
                    keywords::ensure_path(con, path)?;
                }
                keywords::add_paths(&before, &paths)
            } else {
                keywords::remove_paths(&before, &paths)
            };
            return keywords::write_variant(con, variant_pk, &after);
        }
        // Text fields: only the columns that change, so a blank stored as '' stays as it
        // is; a cleared field is NULL.
        Op::SetIptc { base, value } => {
            let changed = changed_columns(&IPTC_COLUMNS, iptc_fields(base), iptc_fields(value));
            return write_text(con, variant_pk, &changed);
        }
        Op::SetPlace { base, value } => {
            let changed = changed_columns(&PLACE_COLUMNS, place_fields(base), place_fields(value));
            return write_text(con, variant_pk, &changed);
        }
        Op::SetGeo { value, .. } => {
            let (lat, lon) = match value {
                Some(g) => (Some(config::format_gps(g.lat, true)), Some(config::format_gps(g.lon, false))),
                None => (None, None),
            };
            return con
                .prepare_cached(
                    "UPDATE ZIMAGE SET ZGPSLATITUDE = ?1, ZGPSLONGITUDE = ?2
                      WHERE Z_PK = (SELECT ZIMAGE FROM ZVARIANT WHERE Z_PK = ?3)",
                )
                .and_then(|mut s| s.execute(params![lat, lon, variant_pk]))
                .map_err(db_err);
        }
        Op::AddToCollection { .. } | Op::RemoveFromCollection { .. } => {
            return Err(SourceError::Other("album membership is written per album, not per photo".into()));
        }
    };
    let rows = metadata_rows(con, variant_pk)?;
    let sql = format!("UPDATE ZVARIANTMETADATA SET {column} = ?1 WHERE Z_PK = ?2");
    let mut stmt = con.prepare_cached(&sql).map_err(db_err)?;
    for row in &rows {
        stmt.execute(params![value, row]).map_err(db_err)?;
    }
    Ok(rows.len())
}

fn changed_columns<'a, const N: usize>(
    columns: &[&'static str; N],
    base: [&Option<String>; N],
    value: [&'a Option<String>; N],
) -> Vec<(&'static str, Option<&'a str>)> {
    (0..N).filter(|&i| base[i] != value[i]).map(|i| (columns[i], value[i].as_deref())).collect()
}

/// Sets text columns on both metadata rows. Returns the rows touched.
fn write_text(con: &Connection, variant_pk: i64, columns: &[(&str, Option<&str>)]) -> Result<usize> {
    let rows = metadata_rows(con, variant_pk)?;
    for (column, value) in columns {
        let sql = format!("UPDATE ZVARIANTMETADATA SET {column} = ?1 WHERE Z_PK = ?2");
        let mut stmt = con.prepare_cached(&sql).map_err(db_err)?;
        for row in &rows {
            stmt.execute(params![value, row]).map_err(db_err)?;
        }
    }
    Ok(rows.len())
}

/// Applies `plan` inside one transaction on an open read-write connection. The plan is
/// made again first; if the catalog no longer agrees with it, nothing is written.
pub(crate) fn write(
    con: &mut Connection,
    plan: &ApplyPlan,
    variant_pk: impl Fn(&Change) -> Result<i64>,
) -> Result<usize> {
    let tx = con.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_err)?;
    let again = self::plan(&tx, &plan.remaps, &plan.collections, &plan.apply, &variant_pk)?;
    if again.apply != plan.apply {
        return Err(SourceError::StalePlan);
    }
    // Keyword tree changes first: queued photo edits already name keywords by their new
    // paths. (The check above ran before them, against the state the plan was made on.)
    for remap in &plan.remaps {
        keywords::remap(&tx, remap)?;
    }
    // Then albums: photo edits may put photos into one made here.
    for (i, change) in plan.collections.iter().enumerate() {
        if let Some(why) = albums::check(&tx, &plan.collections[..i], change)? {
            return Err(SourceError::Other(format!("{why}: {change:?}")));
        }
        albums::write(&tx, change)?;
    }
    // Album membership goes album by album, as the Python `album add` does.
    let mut members: BTreeMap<(String, bool), Vec<i64>> = BTreeMap::new();
    for change in &plan.apply {
        match &change.op {
            Op::AddToCollection { collection } => {
                members.entry((collection.0.clone(), true)).or_default().push(variant_pk(change)?)
            }
            Op::RemoveFromCollection { collection } => {
                members.entry((collection.0.clone(), false)).or_default().push(variant_pk(change)?)
            }
            op => {
                write_change(&tx, variant_pk(change)?, op)?;
            }
        }
    }
    for ((collection, adding), variants) in members {
        let id = c1model::CollectionId(collection);
        let pk = albums::resolve(&tx, &id)?.ok_or_else(|| SourceError::Other(format!("album {} is gone", id.0)))?;
        let before = albums::dangling(&tx, pk)?;
        if adding {
            albums::add_members(&tx, pk, &variants)?;
        } else {
            albums::remove_members(&tx, pk, &variants)?;
        }
        let after = albums::dangling(&tx, pk)?;
        if after.0 > before.0 || after.1 > before.1 {
            return Err(SourceError::Other(format!(
                "album {} would be left with links that do not match ({before:?} → {after:?})",
                id.0
            )));
        }
    }
    if !plan.remaps.is_empty() || plan.apply.iter().any(|c| matches!(c.op, Op::AddKeywords { .. })) {
        let problems = keywords::check_tree(&tx)?;
        if !problems.is_empty() {
            return Err(SourceError::Other(format!("the keyword tree would be left broken: {}", problems.join("; "))));
        }
    }
    // Capture One's own record of when the document last changed.
    tx.execute(
        "UPDATE ZDOCUMENTCONTENT SET ZDATEMODIFIED = ?1 WHERE Z_PK = (SELECT MIN(Z_PK) FROM ZDOCUMENTCONTENT)",
        [unix_now()],
    )
    .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(plan.remaps.len() + plan.collections.len() + plan.apply.len())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

// ---------------------------------------------------------------------------------------
// Backups
// ---------------------------------------------------------------------------------------

/// Written next to each backup as `<backup>.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Backup {
    pub backup: PathBuf,
    pub source: PathBuf,
    /// The catalog's size and modification time when it was copied: a backup is reused
    /// while the catalog still matches them.
    pub source_size: u64,
    /// Nanoseconds since the epoch: whole numbers, so they compare exactly after a trip
    /// through JSON.
    pub source_mtime: u64,
    /// Local time, `2026-10-11T14:03:22`.
    pub created: String,
    pub size: u64,
    pub millis: u128,
    /// Milliseconds since the epoch, when it was made.
    #[serde(default)]
    pub created_ms: u64,
    /// The catalog's size and modification time right after the app's last write that
    /// this backup covers. While the catalog still matches, the next write reuses it.
    #[serde(default)]
    pub after_write: Option<(u64, u64)>,
}

/// Size and modification time (ns).
fn file_stamp(path: &Path) -> Result<(u64, u64)> {
    let meta = fs::metadata(path)?;
    let mtime = meta.modified()?.duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    Ok((meta.len(), mtime))
}

/// Backups of `catalog` in `dir`, newest first.
pub fn backups(catalog: &Path, dir: &Path) -> Vec<Backup> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Backup> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice::<Backup>(&fs::read(e.path()).ok()?).ok())
        .filter(|b| b.source == catalog && b.backup.is_file())
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.backup.cmp(&a.backup)));
    out
}

/// Copies the catalog into `dir` through SQLite's backup API, so the copy is consistent
/// even if something touched the file meanwhile. Reuses the newest backup when the
/// catalog has not changed since it was made, and keeps only the newest `keep`.
pub fn backup(catalog: &Path, dir: &Path, keep: usize) -> Result<Backup> {
    let stamp = file_stamp(catalog)?;
    let (source_size, source_mtime) = stamp;
    let all = backups(catalog, dir);
    if let Some(same) = all.iter().find(|b| b.source_size == source_size && b.source_mtime == source_mtime) {
        return Ok(same.clone());
    }
    // Only the app wrote since the newest backup, and not long ago: it still covers this.
    if let Some(newest) = all.first()
        && newest.after_write == Some(stamp)
        && now_ms().saturating_sub(newest.created_ms) < SHARED_BACKUP_MS
    {
        return Ok(newest.clone());
    }
    fs::create_dir_all(dir)?;
    let started = Instant::now();
    let stem = catalog.file_stem().unwrap_or_default().to_string_lossy();
    let ext = catalog.extension().unwrap_or_default().to_string_lossy();
    let now = chrono::Local::now();
    let target = dir.join(format!("{stem}.{}.{ext}", now.format("%Y%m%dT%H%M%S%.3f")));
    let result =
        catalog::open_live(catalog).and_then(|con| con.backup(rusqlite::MAIN_DB, &target, None).map_err(db_err));
    if let Err(e) = result {
        let _ = fs::remove_file(&target);
        return Err(e);
    }
    let record = Backup {
        size: fs::metadata(&target)?.len(),
        backup: target.clone(),
        source: catalog.to_owned(),
        source_size,
        source_mtime,
        created: now.format("%Y-%m-%dT%H:%M:%S%.3f").to_string(),
        millis: started.elapsed().as_millis(),
        created_ms: now_ms(),
        after_write: None,
    };
    let manifest = manifest_path(&target);
    fs::write(&manifest, serde_json::to_vec_pretty(&record).map_err(|e| SourceError::Other(e.to_string()))?)?;
    for stale in backups(catalog, dir).into_iter().skip(keep.max(1)) {
        let _ = fs::remove_file(&stale.backup);
        let _ = fs::remove_file(manifest_path(&stale.backup));
    }
    Ok(record)
}

/// Records that the app just wrote to the catalog under `backup`'s cover.
pub(crate) fn wrote(catalog: &Path, backup: &Backup) -> Result<()> {
    let record = Backup { after_write: Some(file_stamp(catalog)?), ..backup.clone() };
    let bytes = serde_json::to_vec_pretty(&record).map_err(|e| SourceError::Other(e.to_string()))?;
    fs::write(manifest_path(&record.backup), bytes)?;
    Ok(())
}

fn manifest_path(backup: &Path) -> PathBuf {
    let mut name = backup.as_os_str().to_owned();
    name.push(".json");
    PathBuf::from(name)
}

/// Opens the catalog for writing. Only after [`preflight`].
pub(crate) fn open_write(catalog: &Path) -> Result<Connection> {
    let con = Connection::open(catalog).map_err(db_err)?;
    con.busy_timeout(std::time::Duration::from_secs(10)).map_err(db_err)?;
    Ok(con)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Capture One's schema, dumped once from the live catalog into `tests/fixtures`
    /// (ignored by git: it is their schema, not ours). `None` when there is neither the
    /// dump nor a catalog, as on CI; the tests that need it then pass without running.
    fn schema() -> Option<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("schema.sql");
        if let Ok(sql) = fs::read_to_string(&path) {
            return Some(sql);
        }
        let con = catalog::open_live(&crate::find_catalog(None).ok()?).ok()?;
        let mut stmt = con
            .prepare(
                "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'
                  ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END, name",
            )
            .ok()?;
        let sql: String = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .ok()?
            .filter_map(|r| r.ok())
            .map(|s| {
                s + ";
"
            })
            .collect();
        fs::create_dir_all(path.parent()?).ok()?;
        fs::write(&path, &sql).ok()?;
        Some(sql)
    }

    /// A small catalog with the real schema: five images, each with one variant whose
    /// default (18), adjustment (17) and combined (20) layers have their own metadata
    /// row, as Capture One lays them out. Mirrors `legacy/c1cat-py/tests/conftest.py`.
    pub(crate) fn fixture(path: &Path) -> Option<Connection> {
        let Some(schema) = schema() else {
            eprintln!("skipped: no schema fixture and no catalog to dump one from");
            return None;
        };
        let con = Connection::open(path).unwrap();
        con.execute_batch(&schema).unwrap();
        con.execute_batch(
            "INSERT INTO ZVERSIONINFO (Z_ENT, Z_PK, ZVERSION, ZCOMPATIBLEVERSION, ZAUTHOR) VALUES (32, 1, 160800, 160800, 'test');
             INSERT INTO ZDOCUMENTCONTENT (Z_PK, ZDATEMODIFIED, ZDOCUMENTTYPE) VALUES (1, 0, 1);
             INSERT INTO ZCOLLECTION (Z_ENT, Z_PK, ZUUID, ZNAME, ZPARENT, ZPROJECT, ZCOLLECTIONINDEX, ZDOCUMENT)
               VALUES (7, 1, 'root', '[Collection]', -1, -1, 0, 1);
             INSERT INTO ZPATHLOCATION (Z_ENT, Z_PK, ZWINROOT, ZRELATIVEPATH, ZISRELATIVE) VALUES (38, 1, 'H:\\', 'fotos', 0);",
        )
        .unwrap();
        let mut layer = 0;
        for pk in 1..=5i64 {
            con.execute(
                "INSERT INTO ZIMAGE (Z_ENT, Z_PK, ZIMAGEUUID, ZIMAGEFILENAME, ZDISPLAYNAME, ZIMAGELOCATION, ZISTRASHED,
                   ZISINSIDECATALOG, ZEXP_DATE, ZIMPORTDATE, ZCAMERA_MODEL, ZCAMERA_LENS, ZISO, ZWIDTH, ZHEIGHT, ZSIDECARPATH)
                 VALUES (12, ?1, ?2, ?3, ?4, 1, 0, 0, ?5, 1700100000, 'X-Pro2', '23mm', 400, 6000, 4000, ?6)",
                params![
                    pk,
                    format!("uuid-img-{pk}"),
                    format!("DSCF{pk:04}.RAF"),
                    format!("DSCF{pk:04}"),
                    1_700_000_000 + pk * 86_400,
                    format!("2023/1/1/1/DSCF{pk:04}.RAF")
                ],
            )
            .unwrap();
            let mut layers = [0i64; 3];
            for (i, ent) in [18, 17, 20].into_iter().enumerate() {
                layer += 1;
                con.execute("INSERT INTO ZVARIANTMETADATA (Z_ENT, Z_PK, ZLAYER) VALUES (13, ?1, ?1)", [layer]).unwrap();
                con.execute(
                    "INSERT INTO ZVARIANTLAYER (Z_ENT, Z_PK, ZVARIANT, ZINDEX, ZMETADATA) VALUES (?1, ?2, ?3, ?4, ?2)",
                    params![ent, layer, pk, i as i64],
                )
                .unwrap();
                layers[i] = layer;
            }
            con.execute(
                "INSERT INTO ZVARIANT (Z_ENT, Z_PK, ZVARIANTUUID, ZINDEX, ZIMAGE, ZISMODIFIED, ZADJUSTMENTLAYER,
                   ZDEFAULTLAYER, ZCOMBINEDSETTINGS) VALUES (15, ?1, ?2, 0, ?1, 0, ?3, ?4, ?5)",
                params![pk, format!("uuid-var-{pk}"), layers[1], layers[0], layers[2]],
            )
            .unwrap();
        }
        Some(con)
    }

    fn by_layer(con: &Connection, variant: i64, column: &str) -> Vec<(i64, Option<i64>)> {
        let sql = format!(
            "SELECT l.Z_ENT, m.{column} FROM ZVARIANTLAYER l JOIN ZVARIANTMETADATA m ON m.Z_PK = l.ZMETADATA
              WHERE l.ZVARIANT = ?1 ORDER BY l.Z_ENT"
        );
        let mut stmt = con.prepare(&sql).unwrap();
        stmt.query_map([variant], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
    }

    fn change(pk: i64, op: Op) -> Change {
        Change { photo: c1model::PhotoId::new(&c1model::SourceId("t".into()), pk.to_string()), op }
    }
    fn pk(c: &Change) -> Result<i64> {
        Ok(c.photo.ext.parse().unwrap())
    }

    #[test]
    fn rating_goes_to_adjustment_and_combined_never_default() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        let plan = plan(&con, &[], &[], &[change(1, Op::SetRating { base: None, value: Some(5) })], pk).unwrap();
        assert_eq!(write(&mut con, &plan, pk).unwrap(), 1);
        assert_eq!(by_layer(&con, 1, "ZBASIC_RATING"), [(17, Some(5)), (18, None), (20, Some(5))]);
        assert_eq!(by_layer(&con, 2, "ZBASIC_RATING"), [(17, None), (18, None), (20, None)]);
    }

    #[test]
    fn unrated_and_no_colour_are_null() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        let set = [
            change(1, Op::SetRating { base: None, value: Some(3) }),
            change(1, Op::SetColor { base: None, value: Some(ColorLabel::Green) }),
        ];
        let p = plan(&con, &[], &[], &set, pk).unwrap();
        write(&mut con, &p, pk).unwrap();
        assert_eq!(by_layer(&con, 1, "ZCOLOR_TAG_INDEX"), [(17, Some(4)), (18, None), (20, Some(4))]);

        let clear = [
            change(1, Op::SetRating { base: Some(3), value: None }),
            change(1, Op::SetColor { base: Some(ColorLabel::Green), value: None }),
        ];
        let p = plan(&con, &[], &[], &clear, pk).unwrap();
        assert_eq!(p.apply.len(), 2);
        write(&mut con, &p, pk).unwrap();
        assert_eq!(by_layer(&con, 1, "ZBASIC_RATING"), [(17, None), (18, None), (20, None)]);
        assert_eq!(by_layer(&con, 1, "ZCOLOR_TAG_INDEX"), [(17, None), (18, None), (20, None)]);
    }

    fn text_by_layer(con: &Connection, variant: i64, column: &str) -> Vec<(i64, Option<String>)> {
        let sql = format!(
            "SELECT l.Z_ENT, m.{column} FROM ZVARIANTLAYER l JOIN ZVARIANTMETADATA m ON m.Z_PK = l.ZMETADATA
              WHERE l.ZVARIANT = ?1 ORDER BY l.Z_ENT"
        );
        let mut stmt = con.prepare(&sql).unwrap();
        stmt.query_map([variant], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
    }

    fn s(v: &str) -> Option<String> {
        Some(v.to_owned())
    }

    #[test]
    fn place_writes_only_the_fields_that_change_on_both_layers() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        // A blank stored as '' reads as nothing and is left alone.
        con.execute("UPDATE ZVARIANTMETADATA SET ZIMAGE_STATE = '', ZIMAGE_CITY = 'Krakow' WHERE Z_PK IN (2, 3)", [])
            .unwrap();
        let base = Place { city: s("Krakow"), ..Place::default() };
        let value = Place { city: s("Kraków"), country: s("Poland"), country_code: s("PL"), ..Place::default() };
        let p =
            plan(&con, &[], &[], &[change(1, Op::SetPlace { base: base.clone(), value: value.clone() })], pk).unwrap();
        assert_eq!(p.apply.len(), 1);
        write(&mut con, &p, pk).unwrap();
        let some = |v: &str| [(17, s(v)), (18, None), (20, s(v))];
        assert_eq!(text_by_layer(&con, 1, "ZIMAGE_CITY"), some("Kraków"));
        assert_eq!(text_by_layer(&con, 1, "ZIMAGE_COUNTRY"), some("Poland"));
        assert_eq!(text_by_layer(&con, 1, "ZIMAGE_ISOCOUNTRYCODE"), some("PL"));
        assert_eq!(text_by_layer(&con, 1, "ZIMAGE_STATE"), some(""));
        // Written: the same op is now done; one made on the old value conflicts.
        let again = plan(&con, &[], &[], &[change(1, Op::SetPlace { base: base.clone(), value })], pk).unwrap();
        assert_eq!(again.no_ops.len(), 1);
        let stale = Op::SetPlace { base, value: Place { city: s("Warsaw"), ..Place::default() } };
        assert_eq!(plan(&con, &[], &[], &[change(1, stale)], pk).unwrap().conflicts.len(), 1);
        // Cleared is NULL.
        let clear = Op::SetPlace {
            base: Place { city: s("Kraków"), country: s("Poland"), country_code: s("PL"), ..Place::default() },
            value: Place { country: s("Poland"), country_code: s("PL"), ..Place::default() },
        };
        let p = plan(&con, &[], &[], &[change(1, clear)], pk).unwrap();
        write(&mut con, &p, pk).unwrap();
        assert_eq!(text_by_layer(&con, 1, "ZIMAGE_CITY"), [(17, None), (18, None), (20, None)]);
    }

    #[test]
    fn caption_fields_go_to_both_layers() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        let value = Iptc { description: s("24hprojet"), copyright: s("B. M."), ..Iptc::default() };
        let p = plan(&con, &[], &[], &[change(2, Op::SetIptc { base: Iptc::default(), value })], pk).unwrap();
        write(&mut con, &p, pk).unwrap();
        assert_eq!(
            text_by_layer(&con, 2, "ZCONTENT_DESCRIPTION"),
            [(17, s("24hprojet")), (18, None), (20, s("24hprojet"))]
        );
        assert_eq!(text_by_layer(&con, 2, "ZSTATUS_COPYRIGHTNOTICE"), [(17, s("B. M.")), (18, None), (20, s("B. M."))]);
        assert_eq!(text_by_layer(&con, 2, "ZSTATUS_TITLE"), [(17, None), (18, None), (20, None)]);
    }

    #[test]
    fn title_and_status_fields_go_to_both_layers_and_clear_to_null() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        let value = Iptc {
            title: s("DSCF0743"),
            description: s("Line one\nLine two"),
            job_id: s("JOB-7"),
            instructions: s("Embargo until Monday"),
            source: s("Archiwum"),
            ..Iptc::default()
        };
        let p = plan(&con, &[], &[], &[change(3, Op::SetIptc { base: Iptc::default(), value: value.clone() })], pk)
            .unwrap();
        write(&mut con, &p, pk).unwrap();
        let both = |v: &str| [(17, s(v)), (18, None), (20, s(v))];
        assert_eq!(text_by_layer(&con, 3, "ZSTATUS_TITLE"), both("DSCF0743"));
        assert_eq!(text_by_layer(&con, 3, "ZCONTENT_DESCRIPTION"), both("Line one\nLine two"));
        assert_eq!(text_by_layer(&con, 3, "ZSTATUS_JOBIDENTIFIER"), both("JOB-7"));
        assert_eq!(text_by_layer(&con, 3, "ZSTATUS_INSTRUCTIONS"), both("Embargo until Monday"));
        assert_eq!(text_by_layer(&con, 3, "ZSTATUS_SOURCE"), both("Archiwum"));
        // Read back the way it was written: done.
        let again = plan(&con, &[], &[], &[change(3, Op::SetIptc { base: Iptc::default(), value: value.clone() })], pk);
        assert_eq!(again.unwrap().no_ops.len(), 1);
        let cleared = Iptc { job_id: None, ..value.clone() };
        let p = plan(&con, &[], &[], &[change(3, Op::SetIptc { base: value, value: cleared })], pk).unwrap();
        write(&mut con, &p, pk).unwrap();
        assert_eq!(text_by_layer(&con, 3, "ZSTATUS_JOBIDENTIFIER"), [(17, None), (18, None), (20, None)]);
        assert_eq!(text_by_layer(&con, 3, "ZSTATUS_TITLE"), both("DSCF0743"));
    }

    #[test]
    fn gps_goes_to_the_image_and_compares_by_position() {
        use c1model::{GeoOrigin, GeoPoint};
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        con.execute("UPDATE ZIMAGE SET ZGPSLATITUDE = '50,2.2566833N', ZGPSLONGITUDE = '19,56,30E' WHERE Z_PK = 3", [])
            .unwrap();
        let camera = GeoPoint {
            lat: 50.0 + 2.2566833 / 60.0,
            lon: 19.0 + 56.0 / 60.0 + 30.0 / 3600.0,
            alt: None,
            origin: GeoOrigin::Exif,
        };
        let manual = GeoPoint { lat: -33.8688, lon: 151.2093, alt: Some(5.0), origin: GeoOrigin::Manual };
        let p = plan(&con, &[], &[], &[change(3, Op::SetGeo { base: Some(camera), value: Some(manual) })], pk).unwrap();
        assert_eq!(p.apply.len(), 1);
        write(&mut con, &p, pk).unwrap();
        let gps: (String, String) = con
            .query_row("SELECT ZGPSLATITUDE, ZGPSLONGITUDE FROM ZIMAGE WHERE Z_PK = 3", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(gps, ("33.8688S".to_owned(), "151.2093E".to_owned()));
        // Reads back as stored data, at the same spot: done, not a conflict.
        let read_back = GeoPoint { alt: None, origin: GeoOrigin::Exif, ..manual };
        let again =
            plan(&con, &[], &[], &[change(3, Op::SetGeo { base: Some(camera), value: Some(read_back) })], pk).unwrap();
        assert_eq!(again.no_ops.len(), 1);
        let p = plan(&con, &[], &[], &[change(3, Op::SetGeo { base: Some(manual), value: None })], pk).unwrap();
        write(&mut con, &p, pk).unwrap();
        let cleared: Option<String> =
            con.query_row("SELECT ZGPSLATITUDE FROM ZIMAGE WHERE Z_PK = 3", [], |r| r.get(0)).unwrap();
        assert_eq!(cleared, None);
    }

    #[test]
    fn plan_sorts_applies_no_ops_and_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let Some(con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        con.execute("UPDATE ZVARIANTMETADATA SET ZBASIC_RATING = 2 WHERE Z_PK IN (5, 6)", []).unwrap(); // variant 2
        let p = plan(
            &con,
            &[],
            &[],
            &[
                change(1, Op::SetRating { base: None, value: Some(4) }), // apply
                change(2, Op::SetRating { base: None, value: Some(4) }), // someone set 2: conflict
                change(2, Op::SetRating { base: Some(1), value: Some(2) }), // already 2: no-op
                change(99, Op::SetRating { base: None, value: Some(1) }), // gone
            ],
            pk,
        )
        .unwrap();
        assert_eq!((p.apply.len(), p.conflicts.len(), p.no_ops.len()), (1, 2, 1));
        assert_eq!(p.conflicts[0].current, "Some(2)");
        assert_eq!(p.conflicts[1].current, "deleted");
    }

    #[test]
    fn a_plan_made_stale_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        let p = plan(&con, &[], &[], &[change(1, Op::SetRating { base: None, value: Some(4) })], pk).unwrap();
        // Between the plan and the write, someone else rates it.
        con.execute("UPDATE ZVARIANTMETADATA SET ZBASIC_RATING = 1 WHERE Z_PK IN (2, 3)", []).unwrap();
        assert!(matches!(write(&mut con, &p, pk), Err(SourceError::StalePlan)));
        assert_eq!(by_layer(&con, 1, "ZBASIC_RATING"), [(17, Some(1)), (18, None), (20, Some(1))]);
    }

    #[test]
    fn writing_stamps_the_document() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        let p = plan(&con, &[], &[], &[change(1, Op::SetRating { base: None, value: Some(4) })], pk).unwrap();
        write(&mut con, &p, pk).unwrap();
        let stamp: f64 = con.query_row("SELECT ZDATEMODIFIED FROM ZDOCUMENTCONTENT", [], |r| r.get(0)).unwrap();
        assert!((stamp - unix_now()).abs() < 60.0);
    }

    #[test]
    fn backups_are_reused_until_the_catalog_changes_and_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = dir.path().join("c.cocatalogdb");
        let Some(con) = fixture(&catalog) else { return };
        drop(con);
        let store = dir.path().join("backups");

        let first = backup(&catalog, &store, 2).unwrap();
        assert!(first.backup.is_file());
        assert_eq!(backup(&catalog, &store, 2).unwrap(), first, "unchanged catalog reuses the backup");

        let mut made = vec![first];
        for n in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let con = Connection::open(&catalog).unwrap();
            con.execute("UPDATE ZDOCUMENTCONTENT SET ZDATEMODIFIED = ?1", [n]).unwrap();
            drop(con);
            made.push(backup(&catalog, &store, 2).unwrap());
        }
        let kept = backups(&catalog, &store);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0], made[3]);
        assert_eq!(kept[1], made[2]);
        assert!(!made[0].backup.exists(), "the oldest is pruned");

        // The copy is a working catalog.
        let copy = Connection::open(&kept[0].backup).unwrap();
        let n: i64 = copy.query_row("SELECT COUNT(*) FROM ZVARIANT", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 5);
    }

    #[test]
    fn writes_in_a_row_share_a_backup_until_someone_else_writes() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = dir.path().join("c.cocatalogdb");
        let Some(mut con) = fixture(&catalog) else { return };
        let store = dir.path().join("backups");
        let write_one = |con: &mut Connection, pk: i64, stars: u8| {
            let p =
                plan(con, &[], &[], &[change(pk, Op::SetRating { base: None, value: Some(stars) })], self::pk).unwrap();
            write(con, &p, self::pk).unwrap();
        };

        let first = backup(&catalog, &store, 5).unwrap();
        write_one(&mut con, 1, 4);
        wrote(&catalog, &first).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = backup(&catalog, &store, 5).unwrap();
        assert_eq!(second.backup, first.backup, "only the app wrote: the same backup covers it");
        write_one(&mut con, 2, 4);
        wrote(&catalog, &second).unwrap();

        // Someone else writes (Capture One): the next write gets a fresh backup.
        std::thread::sleep(std::time::Duration::from_millis(20));
        con.execute("UPDATE ZVARIANTMETADATA SET ZBASIC_RATING = 1 WHERE Z_PK IN (8, 9)", []).unwrap();
        let third = backup(&catalog, &store, 5).unwrap();
        assert_ne!(third.backup, first.backup);
        assert_eq!(backups(&catalog, &store).len(), 2);
    }

    fn keywords_by_layer(con: &Connection, variant: i64) -> Vec<(i64, Option<String>)> {
        let mut stmt = con
            .prepare(
                "SELECT l.Z_ENT, m.ZCONTENT_KEYWORDS FROM ZVARIANTLAYER l JOIN ZVARIANTMETADATA m ON m.Z_PK = l.ZMETADATA
                  WHERE l.ZVARIANT = ?1 ORDER BY l.Z_ENT",
            )
            .unwrap();
        stmt.query_map([variant], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
    }

    fn tree(con: &Connection) -> Vec<(String, Option<String>, i64, i64)> {
        let mut stmt = con
            .prepare(
                "SELECT k.ZNAME, p.ZNAME, k.ZLEFT, k.ZRIGHT FROM ZKEYWORD k LEFT JOIN ZKEYWORD p ON p.Z_PK = k.ZPARENT
                  ORDER BY k.ZLEFT",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap().map(|r| r.unwrap()).collect()
    }

    fn kw(paths: &[&str]) -> Vec<c1model::KeywordPath> {
        paths.iter().map(|p| c1model::KeywordPath::from_pipe(p)).collect()
    }

    #[test]
    fn keywords_go_to_both_layers_in_their_own_order_and_grow_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        let add = change(1, Op::AddKeywords { paths: kw(&["street", "b&w", "best|best2018"]) });
        let p = plan(&con, &[], &[], std::slice::from_ref(&add), pk).unwrap();
        write(&mut con, &p, pk).unwrap();
        assert_eq!(
            keywords_by_layer(&con, 1),
            [
                (17, Some("b&w||1,best|best2018||2,street||0".into())),
                (18, None),
                (20, Some("street||0,b&w||1,best|best2018||2".into())),
            ]
        );
        assert_eq!(
            tree(&con),
            [
                ("street".into(), None, 1, 2),
                ("b&w".into(), None, 3, 4),
                ("best".into(), None, 5, 8),
                ("best2018".into(), Some("best".into()), 6, 7),
            ]
        );
        // Already there: nothing to do.
        assert_eq!(plan(&con, &[], &[], &[add], pk).unwrap().no_ops.len(), 1);
    }

    #[test]
    fn a_child_under_an_early_parent_shifts_the_rest_of_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        for (pk_, path) in [(1, "a"), (2, "b"), (3, "a|x"), (3, "a|x|deep"), (4, "b|y")] {
            let p = plan(&con, &[], &[], &[change(pk_, Op::AddKeywords { paths: kw(&[path]) })], pk).unwrap();
            write(&mut con, &p, pk).unwrap();
        }
        assert_eq!(keywords::check_tree(&con).unwrap(), Vec::<String>::new());
        assert_eq!(
            tree(&con),
            [
                ("a".into(), None, 1, 6),
                ("x".into(), Some("a".into()), 2, 5),
                ("deep".into(), Some("x".into()), 3, 4),
                ("b".into(), None, 7, 10),
                ("y".into(), Some("b".into()), 8, 9),
            ]
        );
        assert_eq!(keywords_by_layer(&con, 3)[0].1.as_deref(), Some("a|x||0,a|x|deep||1"));
    }

    #[test]
    fn removing_renumbers_and_leaves_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        let p =
            plan(&con, &[], &[], &[change(2, Op::AddKeywords { paths: kw(&["street", "b&w", "night"]) })], pk).unwrap();
        write(&mut con, &p, pk).unwrap();
        let remove = change(2, Op::RemoveKeywords { paths: kw(&["street"]) });
        let p = plan(&con, &[], &[], std::slice::from_ref(&remove), pk).unwrap();
        write(&mut con, &p, pk).unwrap();
        assert_eq!(
            keywords_by_layer(&con, 2),
            [(17, Some("b&w||0,night||1".into())), (18, None), (20, Some("b&w||0,night||1".into()))]
        );
        assert_eq!(tree(&con).len(), 3, "the keyword stays in the library");
        assert_eq!(plan(&con, &[], &[], &[remove], pk).unwrap().no_ops.len(), 1, "removing it again does nothing");
        // Removing the last one leaves no string at all.
        let p = plan(&con, &[], &[], &[change(2, Op::RemoveKeywords { paths: kw(&["b&w", "night"]) })], pk).unwrap();
        write(&mut con, &p, pk).unwrap();
        assert_eq!(keywords_by_layer(&con, 2), [(17, None), (18, None), (20, None)]);
    }

    #[test]
    fn a_keyword_name_with_a_comma_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let Some(con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        assert!(plan(&con, &[], &[], &[change(1, Op::AddKeywords { paths: kw(&["a,b"]) })], pk).is_err());
    }

    fn remap_of(map: &[(&str, &str)], prune: &[&str]) -> c1model::KeywordRemap {
        c1model::KeywordRemap {
            map: map
                .iter()
                .map(|(a, b)| (c1model::KeywordPath::from_pipe(a), c1model::KeywordPath::from_pipe(b)))
                .collect(),
            prune: prune.iter().map(|p| c1model::KeywordPath::from_pipe(p)).collect(),
            label: String::new(),
        }
    }

    /// Photos 1-3 with keywords; the tree has street, streets|night, b&w, a|x.
    fn tagged(path: &Path) -> Option<Connection> {
        let mut con = fixture(path)?;
        for (pk_, paths) in [(1, vec!["street", "streets", "b&w"]), (2, vec!["streets|night", "a"]), (3, vec!["a|x"])] {
            let p = plan(&con, &[], &[], &[change(pk_, Op::AddKeywords { paths: kw(&paths) })], pk).unwrap();
            write(&mut con, &p, pk).unwrap();
        }
        Some(con)
    }

    fn remap_all(con: &mut Connection, remap: c1model::KeywordRemap) {
        let p = ApplyPlan { remaps: vec![remap], ..ApplyPlan::default() };
        write(con, &p, pk).unwrap();
        assert_eq!(keywords::check_tree(con).unwrap(), Vec::<String>::new());
    }

    fn adjusted(con: &Connection, variant: i64) -> Option<String> {
        keywords_by_layer(con, variant)[0].1.clone()
    }

    #[test]
    fn renaming_keeps_the_tree_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = tagged(&dir.path().join("c.cocatalogdb")) else { return };
        let before = tree(&con);
        remap_all(&mut con, remap_of(&[("streets", "avenues")], &[]));
        assert_eq!(adjusted(&con, 1).as_deref(), Some("avenues||1,b&w||2,street||0"));
        assert_eq!(adjusted(&con, 2).as_deref(), Some("a||1,avenues|night||0"));
        let after = tree(&con);
        let renamed: Vec<_> = before
            .into_iter()
            .map(|(n, p, l, r)| {
                let f = |s: String| if s == "streets" { "avenues".to_owned() } else { s };
                (f(n), p.map(f), l, r)
            })
            .collect();
        assert_eq!(after, renamed, "only the name changes; intervals stay");
    }

    #[test]
    fn joining_folds_a_keyword_and_its_children_into_another() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = tagged(&dir.path().join("c.cocatalogdb")) else { return };
        let street_pk: i64 =
            con.query_row("SELECT Z_PK FROM ZKEYWORD WHERE ZNAME = 'street'", [], |r| r.get(0)).unwrap();
        remap_all(&mut con, remap_of(&[("streets", "street")], &[]));
        assert_eq!(adjusted(&con, 1).as_deref(), Some("b&w||1,street||0"), "the duplicate is dropped");
        assert_eq!(adjusted(&con, 2).as_deref(), Some("a||1,street|night||0"));
        let names: Vec<_> = tree(&con).into_iter().map(|(n, p, ..)| (n, p)).collect();
        assert!(names.contains(&("night".into(), Some("street".into()))));
        assert!(!names.iter().any(|(n, _)| n == "streets"));
        let kept: i64 = con.query_row("SELECT Z_PK FROM ZKEYWORD WHERE ZNAME = 'street'", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, street_pk, "the target keeps its node");
    }

    #[test]
    fn grouping_under_a_new_keyword_then_ungrouping() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = tagged(&dir.path().join("c.cocatalogdb")) else { return };
        remap_all(&mut con, remap_of(&[("a", "things|a"), ("b&w", "things|b&w")], &[]));
        assert_eq!(adjusted(&con, 3).as_deref(), Some("things|a|x||0"));
        let names: Vec<_> = tree(&con).into_iter().map(|(n, p, ..)| (n, p)).collect();
        assert!(names.contains(&("things".into(), None)));
        assert!(names.contains(&("a".into(), Some("things".into()))));
        assert!(names.contains(&("x".into(), Some("a".into()))));
        assert!(names.contains(&("b&w".into(), Some("things".into()))));

        // Ungroup: the children move up, and the emptied group goes (no photo has it).
        remap_all(&mut con, remap_of(&[("things|a", "a"), ("things|b&w", "b&w")], &["things"]));
        assert_eq!(adjusted(&con, 3).as_deref(), Some("a|x||0"));
        let names: Vec<_> = tree(&con).into_iter().map(|(n, p, ..)| (n, p)).collect();
        assert!(!names.iter().any(|(n, _)| n == "things"));
        assert!(names.contains(&("a".into(), None)));
    }

    #[test]
    fn a_group_some_photo_carries_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = tagged(&dir.path().join("c.cocatalogdb")) else { return };
        remap_all(&mut con, remap_of(&[("a|x", "x")], &["a"]));
        assert!(tree(&con).iter().any(|(n, p, ..)| n == "a" && p.is_none()), "photo 2 still has a");
    }

    #[test]
    fn swapping_two_names_does_not_trip_the_unique_index() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = tagged(&dir.path().join("c.cocatalogdb")) else { return };
        remap_all(&mut con, remap_of(&[("street", "b&w"), ("b&w", "street")], &[]));
        assert_eq!(adjusted(&con, 1).as_deref(), Some("b&w||0,street||2,streets||1"));
    }

    #[test]
    fn a_remap_of_keywords_gone_from_the_catalog_is_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let Some(con) = tagged(&dir.path().join("c.cocatalogdb")) else { return };
        assert!(keywords::check_remap(&con, &[], &remap_of(&[("nope", "x")], &[])).unwrap().is_some());
        assert!(keywords::check_remap(&con, &[], &remap_of(&[("streets|night", "night")], &[])).unwrap().is_none());
        // A change to a keyword an earlier queued change makes.
        let rename = remap_of(&[("street", "avenue")], &[]);
        let group = remap_of(&[("avenue", "g|avenue")], &[]);
        assert!(keywords::check_remap(&con, &[], &group).unwrap().is_some());
        assert!(keywords::check_remap(&con, &[rename], &group).unwrap().is_none());
    }

    #[test]
    fn structure_wants_delete_journal_and_the_known_version() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = dir.path().join("c.cocatalogdb");
        let Some(con) = fixture(&catalog) else { return };
        assert!(structure(&catalog).is_ok());
        con.execute("UPDATE ZVERSIONINFO SET ZVERSION = 170000", []).unwrap();
        assert!(matches!(structure(&catalog), Err(SourceError::Schema(_))));
    }
}
