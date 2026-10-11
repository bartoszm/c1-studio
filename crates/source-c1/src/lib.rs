//! Capture One's catalog as a [`PhotoSource`].
//!
//! A catalog is a SQLite database with Core Data–style `Z*` tables; Capture One for
//! Windows has no scripting API, so this database is the only programmable surface.
//! `legacy/c1cat-py/skills/capture-one-catalog/references/schema.md` documents every
//! schema fact relied on here; each one was verified against a real catalog.

mod catalog;
mod config;
mod extract;
mod keywords;
mod process;
mod provider;
mod smart;
mod write;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use c1model::source::Result;
use c1model::{
    ApplyPlan, ApplyReport, Capabilities, Change, Cursor, Field, ImageFormat, ImageRef, ImageTier, KeywordRemap,
    PhotoId, PhotoSource, SourceError, SourceEvent, SourceId, SourceInfo, WriteStatus,
};

pub use catalog::EXPECTED_ZVERSION;
pub use config::{catalog_id, data_dir, find_catalog};
pub use extract::{Extract, extract};
pub use provider::CaptureOneProvider;
pub use write::Backup;

pub struct CaptureOne {
    catalog: PathBuf,
    id: SourceId,
    work_dir: PathBuf,
    /// Catalog schema version, read once at open.
    zversion: i64,
    /// Serialises snapshots: each one copies the catalog into `work_dir`.
    snapshot_lock: Mutex<()>,
}

impl CaptureOne {
    /// Opens a catalog. `work_dir` holds the temporary copy made for each snapshot.
    pub fn open(catalog: impl AsRef<Path>, work_dir: impl Into<PathBuf>) -> Result<Self> {
        let catalog = catalog.as_ref();
        if !catalog.is_file() {
            return Err(SourceError::NotFound(catalog.display().to_string()));
        }
        let catalog = std::path::absolute(catalog)?;
        let zversion = catalog::open_live(&catalog).and_then(|con| catalog::zversion(&con))?;
        Ok(Self {
            id: SourceId(format!("c1:{}", catalog_id(&catalog))),
            catalog,
            work_dir: work_dir.into(),
            zversion,
            snapshot_lock: Mutex::new(()),
        })
    }

    pub fn catalog_path(&self) -> &Path {
        &self.catalog
    }

    fn catalog_dir(&self) -> &Path {
        self.catalog.parent().expect("a catalog file has a parent directory")
    }

    /// Where backups made before each write go.
    pub fn backup_dir(&self) -> PathBuf {
        self.work_dir.join("backups")
    }

    /// Backups of this catalog, newest first.
    pub fn backups(&self) -> Vec<Backup> {
        write::backups(&self.catalog, &self.backup_dir())
    }

    fn variant_pk(&self, id: &PhotoId) -> Result<i64> {
        if id.source != self.id {
            return Err(SourceError::NotFound(format!("{id} belongs to another source")));
        }
        id.ext.parse().map_err(|_| SourceError::NotFound(id.to_string()))
    }
}

impl PhotoSource for CaptureOne {
    fn info(&self) -> SourceInfo {
        let stem = self.catalog.file_stem().unwrap_or_default().to_string_lossy();
        SourceInfo { id: self.id.clone(), name: format!("Capture One · {stem}"), version: self.zversion.to_string() }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            writable: vec![Field::Rating, Field::Color, Field::Keywords, Field::Iptc, Field::Place, Field::Geo],
            tiers: vec![ImageTier::Thumb, ImageTier::Preview, ImageTier::Original],
            geo_per_asset: true,
            collections: true,
        }
    }

    fn snapshot(&self, since: Option<&Cursor>, sink: &mut dyn FnMut(SourceEvent) -> Result<()>) -> Result<()> {
        let _guard = self.snapshot_lock.lock().unwrap();
        // Capture One records no per-row modification dates (the columns hold a .NET
        // MinValue sentinel), so the only cheap change signal is the file itself.
        // When it changed we send everything; the consumer diffs by fingerprint.
        let cursor = catalog::file_cursor(&self.catalog)?;
        if since == Some(&cursor) {
            sink(SourceEvent::BeginIncremental)?;
            return sink(SourceEvent::End(cursor));
        }

        let copy = catalog::copy_for_reading(&self.catalog, &self.work_dir)?;
        let extracted = {
            let con = catalog::open_copy(copy.path())?;
            extract(&con, self.catalog_dir(), &self.id)?
        };
        drop(copy);

        sink(SourceEvent::BeginFull)?;
        sink(SourceEvent::Collections(extracted.collections))?;
        sink(SourceEvent::Keywords(extracted.keywords))?;
        for photo in extracted.photos {
            sink(SourceEvent::Upsert(Box::new(photo)))?;
        }
        sink(SourceEvent::End(cursor))
    }

    fn image(&self, id: &PhotoId, tier: ImageTier) -> Result<Option<ImageRef>> {
        let vpk = self.variant_pk(id)?;
        let path = match tier {
            ImageTier::Thumb => Some(config::thumbnail_path(self.catalog_dir(), vpk)),
            ImageTier::Original => {
                let con = catalog::open_live(&self.catalog)?;
                catalog::original_path(&con, self.catalog_dir(), vpk)?.map(PathBuf::from)
            }
            ImageTier::Preview => {
                let con = catalog::open_live(&self.catalog)?;
                let Some(preview) = catalog::preview_source(&con, vpk)? else { return Ok(None) };
                let path = config::preview_path(self.catalog_dir(), &preview.sidecar);
                // Capture One's own render of the edits, for the look the preview lacks.
                let thumb = config::thumbnail_path(self.catalog_dir(), vpk);
                return Ok(path.map(|path| ImageRef {
                    path,
                    format: ImageFormat::JpegXl,
                    white_balance: preview.white_balance,
                    rotation: preview.rotation,
                    monochrome: preview.monochrome,
                    look: thumb.is_file().then_some(thumb),
                }));
            }
        };
        let ext = path.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_ascii_lowercase());
        let format = match (tier, ext.as_deref()) {
            (ImageTier::Thumb, _) | (_, Some("jpg" | "jpeg")) => ImageFormat::Jpeg,
            (_, Some("jxl")) => ImageFormat::JpegXl,
            _ => ImageFormat::Raw,
        };
        Ok(path.filter(|p| p.is_file()).map(|path| ImageRef {
            path,
            format,
            white_balance: None,
            rotation: 0,
            monochrome: false,
            look: None,
        }))
    }

    fn write_status(&self) -> WriteStatus {
        write::status(&self.catalog)
    }

    /// Reads the live catalog, so it works while Capture One is open.
    fn plan(&self, changes: &[Change]) -> Result<ApplyPlan> {
        self.plan_after(&[], changes)
    }

    fn plan_after(&self, remaps: &[KeywordRemap], changes: &[Change]) -> Result<ApplyPlan> {
        let con = catalog::open_live(&self.catalog)?;
        let mut plan = write::plan(&con, remaps, changes, |c| self.variant_pk(&c.photo))?;
        plan.remaps = remaps.to_vec();
        Ok(plan)
    }

    /// Refuses while Capture One or anything else has the catalog open, backs it up,
    /// then writes the plan in one transaction after checking it still holds.
    fn check_remap(&self, earlier: &[KeywordRemap], remap: &KeywordRemap) -> Result<Option<String>> {
        let con = catalog::open_live(&self.catalog)?;
        keywords::check_remap(&con, earlier, remap)
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplyReport> {
        if plan.apply.is_empty() && plan.remaps.is_empty() {
            return Ok(ApplyReport::default());
        }
        let _guard = self.snapshot_lock.lock().unwrap();
        write::preflight(&self.catalog)?;
        let backup = write::backup(&self.catalog, &self.backup_dir(), write::KEEP_BACKUPS)?;
        let mut con = write::open_write(&self.catalog)?;
        let applied = write::write(&mut con, plan, |c| self.variant_pk(&c.photo))?;
        drop(con);
        write::wrote(&self.catalog, &backup)?;
        Ok(ApplyReport { applied, backup: Some(backup.backup.display().to_string()) })
    }
}

#[cfg(test)]
mod tests {
    use c1model::{Op, conformance};

    use super::*;

    fn source() -> Option<(tempfile::TempDir, CaptureOne)> {
        let dir = tempfile::tempdir().unwrap();
        let catalog = dir.path().join("fixture.cocatalogdb");
        drop(write::tests::fixture(&catalog)?);
        let source = CaptureOne::open(&catalog, dir.path().join("work")).unwrap();
        Some((dir, source))
    }

    fn ready(source: &CaptureOne) -> bool {
        let status = source.write_status();
        if status != WriteStatus::Ready {
            eprintln!("write checks skipped: {status:?}");
        }
        status == WriteStatus::Ready
    }

    #[test]
    fn a_catalog_from_before_retouching_layers_reads() {
        // Schema 1650 (Capture One before 16.x) has no ZRETOUCHINGLAYER table.
        let dir = tempfile::tempdir().unwrap();
        let catalog = dir.path().join("old.cocatalogdb");
        let Some(con) = write::tests::fixture(&catalog) else { return };
        con.execute_batch("DROP TABLE ZRETOUCHINGLAYER; UPDATE ZVERSIONINFO SET ZVERSION = 1650").unwrap();
        drop(con);
        let source = CaptureOne::open(&catalog, dir.path().join("work")).unwrap();
        let mut photos = 0;
        source
            .snapshot(None, &mut |e| {
                photos += matches!(e, SourceEvent::Upsert(_)) as usize;
                Ok(())
            })
            .unwrap();
        assert_eq!(photos, 5);
        // Read, but never written: the app was not checked against this format. (Locked
        // instead while Capture One runs.)
        assert_ne!(source.write_status(), WriteStatus::Ready);
    }

    #[test]
    fn fixture_conforms_for_reads_and_writes() {
        let Some((_dir, source)) = source() else { return };
        assert_eq!(conformance::check_read(&source), Vec::<String>::new());
        if ready(&source) {
            let probe = PhotoId::new(&source.info().id, "3");
            assert_eq!(conformance::check_write(&source, &probe), Vec::<String>::new());
        }
    }

    #[test]
    fn apply_backs_up_first_and_the_next_snapshot_sees_the_change() {
        let Some((_dir, source)) = source() else { return };
        if !ready(&source) {
            return;
        }
        let id = PhotoId::new(&source.info().id, "2");
        let plan = source
            .plan(&[Change {
                photo: id.clone(),
                op: Op::SetColor { base: None, value: Some(c1model::ColorLabel::Red) },
            }])
            .unwrap();
        let report = source.apply(&plan).unwrap();
        assert_eq!(report.applied, 1);
        let backup = source.backups();
        assert_eq!(backup.len(), 1);
        assert_eq!(report.backup.as_deref(), Some(backup[0].backup.display().to_string().as_str()));

        let mut color = None;
        source
            .snapshot(None, &mut |e| {
                if let SourceEvent::Upsert(p) = e
                    && p.id == id
                {
                    color = p.color;
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(color, Some(c1model::ColorLabel::Red));

        // The backup still holds the catalog as it was.
        let old = rusqlite::Connection::open(&backup[0].backup).unwrap();
        let tag: Option<i64> = old
            .query_row("SELECT m.ZCOLOR_TAG_INDEX FROM ZVARIANT v JOIN ZVARIANTMETADATA m ON m.ZLAYER = v.ZADJUSTMENTLAYER WHERE v.Z_PK = 2", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tag, None);
    }

    #[test]
    fn edits_not_ported_yet_are_refused() {
        let Some((_dir, source)) = source() else { return };
        let id = PhotoId::new(&source.info().id, "1");
        let add = Change { photo: id, op: Op::AddToCollection { collection: c1model::CollectionId("1".into()) } };
        assert!(matches!(source.plan(&[add]), Err(SourceError::Unsupported(_))));
    }
}
