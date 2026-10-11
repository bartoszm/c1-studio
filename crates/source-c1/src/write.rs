//! Writing to the catalog, ported from `legacy/c1cat-py/c1cat/safety.py` and `cmd_meta.py`.
//!
//! Capture One keeps the whole catalog in memory while it runs and rewrites it on save,
//! so an edit made underneath it is silently lost. The only safe move is to refuse:
//! a write needs Capture One closed and nobody else holding the file. Before the first
//! write the catalog is backed up, and every write runs in one transaction that first
//! re-checks each change against what the catalog holds at that moment.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use c1model::change::{Check, compare};
use c1model::source::Result;
use c1model::{ApplyPlan, Change, ColorLabel, Conflict, Op, SourceError, WriteStatus};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::catalog::{self, EXPECTED_ZVERSION, db_err};
use crate::process;

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
}

fn current(con: &Connection, variant_pk: i64) -> Result<Option<Current>> {
    con.prepare_cached(
        "SELECT m.ZBASIC_RATING, m.ZCOLOR_TAG_INDEX FROM ZVARIANT v
           JOIN ZVARIANTMETADATA m ON m.ZLAYER = v.ZADJUSTMENTLAYER
          WHERE v.Z_PK = ?1",
    )
    .and_then(|mut s| {
        s.query_row([variant_pk], |r| {
            let rating: Option<i64> = r.get(0)?;
            let color: Option<i64> = r.get(1)?;
            Ok(Current {
                rating: rating.filter(|&v| v > 0).map(|v| v.clamp(1, 5) as u8),
                color: color.and_then(|v| u8::try_from(v).ok()).and_then(ColorLabel::from_index),
            })
        })
        .optional()
    })
    .map_err(db_err)
}

/// Sorts changes into apply / conflict / no-op against the catalog behind `con`.
pub(crate) fn plan(
    con: &Connection,
    changes: &[Change],
    variant_pk: impl Fn(&Change) -> Result<i64>,
) -> Result<ApplyPlan> {
    let mut plan = ApplyPlan::default();
    for change in changes {
        let check = match current(con, variant_pk(change)?)? {
            None => Check::Conflict("deleted".into()),
            Some(now) => match &change.op {
                Op::SetRating { base, value } => compare(base, value, &now.rating),
                Op::SetColor { base, value } => compare(base, value, &now.color),
                _ => return Err(SourceError::Unsupported("only ratings and colour tags are written so far")),
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
        _ => return Err(SourceError::Unsupported("only ratings and colour tags are written so far")),
    };
    let rows = metadata_rows(con, variant_pk)?;
    let sql = format!("UPDATE ZVARIANTMETADATA SET {column} = ?1 WHERE Z_PK = ?2");
    let mut stmt = con.prepare_cached(&sql).map_err(db_err)?;
    for row in &rows {
        stmt.execute(params![value, row]).map_err(db_err)?;
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
    let again = self::plan(&tx, &plan.apply, &variant_pk)?;
    if again.apply != plan.apply {
        return Err(SourceError::StalePlan);
    }
    for change in &plan.apply {
        write_change(&tx, variant_pk(change)?, &change.op)?;
    }
    // Capture One's own record of when the document last changed.
    tx.execute(
        "UPDATE ZDOCUMENTCONTENT SET ZDATEMODIFIED = ?1 WHERE Z_PK = (SELECT MIN(Z_PK) FROM ZDOCUMENTCONTENT)",
        [unix_now()],
    )
    .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(plan.apply.len())
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
        let plan = plan(&con, &[change(1, Op::SetRating { base: None, value: Some(5) })], pk).unwrap();
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
        let p = plan(&con, &set, pk).unwrap();
        write(&mut con, &p, pk).unwrap();
        assert_eq!(by_layer(&con, 1, "ZCOLOR_TAG_INDEX"), [(17, Some(4)), (18, None), (20, Some(4))]);

        let clear = [
            change(1, Op::SetRating { base: Some(3), value: None }),
            change(1, Op::SetColor { base: Some(ColorLabel::Green), value: None }),
        ];
        let p = plan(&con, &clear, pk).unwrap();
        assert_eq!(p.apply.len(), 2);
        write(&mut con, &p, pk).unwrap();
        assert_eq!(by_layer(&con, 1, "ZBASIC_RATING"), [(17, None), (18, None), (20, None)]);
        assert_eq!(by_layer(&con, 1, "ZCOLOR_TAG_INDEX"), [(17, None), (18, None), (20, None)]);
    }

    #[test]
    fn plan_sorts_applies_no_ops_and_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let Some(con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        con.execute("UPDATE ZVARIANTMETADATA SET ZBASIC_RATING = 2 WHERE Z_PK IN (5, 6)", []).unwrap(); // variant 2
        let p = plan(
            &con,
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
        let p = plan(&con, &[change(1, Op::SetRating { base: None, value: Some(4) })], pk).unwrap();
        // Between the plan and the write, someone else rates it.
        con.execute("UPDATE ZVARIANTMETADATA SET ZBASIC_RATING = 1 WHERE Z_PK IN (2, 3)", []).unwrap();
        assert!(matches!(write(&mut con, &p, pk), Err(SourceError::StalePlan)));
        assert_eq!(by_layer(&con, 1, "ZBASIC_RATING"), [(17, Some(1)), (18, None), (20, Some(1))]);
    }

    #[test]
    fn writing_stamps_the_document() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        let p = plan(&con, &[change(1, Op::SetRating { base: None, value: Some(4) })], pk).unwrap();
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
            let p = plan(con, &[change(pk, Op::SetRating { base: None, value: Some(stars) })], self::pk).unwrap();
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
