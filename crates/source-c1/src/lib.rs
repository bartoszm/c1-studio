//! Capture One's catalog as a [`PhotoSource`].
//!
//! A catalog is a SQLite database with Core Data–style `Z*` tables; Capture One for
//! Windows has no scripting API, so this database is the only programmable surface.
//! `legacy/c1cat-py/skills/capture-one-catalog/references/schema.md` documents every
//! schema fact relied on here; each one was verified against a real catalog.

mod albums;
mod catalog;
mod compat;
mod config;
mod extract;
mod keywords;
mod process;
mod provider;
mod smart;
mod write;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use c1model::source::Result;
use c1model::{
    ApplyPlan, ApplyReport, BackupInfo, Capabilities, Change, CollectionChange, Crop, Cursor, Field, ImageFormat,
    ImageRef, ImageTier, KeywordRemap, PhotoId, PhotoSource, SourceError, SourceEvent, SourceId, SourceInfo,
    WriteStatus,
};

pub use catalog::EXPECTED_ZVERSION;
pub use config::{catalog_id, data_dir, find_catalog};
pub use extract::{Extract, extract};
pub use provider::CaptureOneProvider;
pub use write::Backup;

type CropsByVariant = HashMap<i64, Crop>;

pub struct CaptureOne {
    catalog: PathBuf,
    id: SourceId,
    work_dir: PathBuf,
    /// Which Capture One made the catalog, read at open and again with each full read
    /// (opening it in a newer Capture One upgrades it).
    version: Mutex<compat::CatalogVersion>,
    /// Serialises snapshots: each one copies the catalog into `work_dir`.
    snapshot_lock: Mutex<()>,
    /// Thumbnail crops by variant, read in one go and again when the catalog changes.
    thumbnail_crops: Mutex<Option<(Cursor, CropsByVariant)>>,
}

impl CaptureOne {
    /// Opens a catalog. `work_dir` holds the temporary copy made for each snapshot.
    pub fn open(catalog: impl AsRef<Path>, work_dir: impl Into<PathBuf>) -> Result<Self> {
        let catalog = catalog.as_ref();
        if !catalog.is_file() {
            return Err(SourceError::NotFound(catalog.display().to_string()));
        }
        let catalog = std::path::absolute(catalog)?;
        let version = catalog::open_live(&catalog).and_then(|con| compat::read_version(&con))?;
        Ok(Self {
            id: SourceId(format!("c1:{}", catalog_id(&catalog))),
            catalog,
            work_dir: work_dir.into(),
            version: Mutex::new(version),
            snapshot_lock: Mutex::new(()),
            thumbnail_crops: Mutex::new(None),
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

    /// The crop to show `variant_pk`'s thumbnail with.
    fn thumbnail_crop(&self, variant_pk: i64) -> Result<Option<Crop>> {
        let cursor = catalog::file_cursor(&self.catalog)?;
        let mut cached = self.thumbnail_crops.lock().unwrap();
        if cached.as_ref().is_none_or(|(at, _)| *at != cursor) {
            let crops = catalog::thumbnail_crops(&catalog::open_live(&self.catalog)?)?;
            *cached = Some((cursor, crops));
        }
        Ok(cached.as_ref().and_then(|(_, crops)| crops.get(&variant_pk).copied()))
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
        let version = self.version.lock().unwrap();
        SourceInfo {
            id: self.id.clone(),
            name: format!("Capture One · {stem}"),
            version: version.schema.to_string(),
            compatibility: compat::assess(&version),
        }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            writable: vec![
                Field::Rating,
                Field::Color,
                Field::Keywords,
                Field::Iptc,
                Field::Place,
                Field::Geo,
                Field::Collections,
            ],
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
            let version = compat::read_version(&con)?;
            *self.version.lock().unwrap() = version.clone();
            extract(&con, self.catalog_dir(), &self.id).map_err(|e| compat::read_failed(&version, &e))?
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
        match tier {
            ImageTier::Thumb => {
                let path = config::thumbnail_path(self.catalog_dir(), vpk);
                if !path.is_file() {
                    return Ok(None);
                }
                Ok(Some(ImageRef {
                    path,
                    format: ImageFormat::Jpeg,
                    white_balance: None,
                    rotation: 0,
                    as_stored: false,
                    monochrome: false,
                    look: None,
                    straighten: 0.0,
                    crop: self.thumbnail_crop(vpk)?,
                }))
            }
            ImageTier::Preview => {
                let con = catalog::open_live(&self.catalog)?;
                let Some(preview) = catalog::preview_source(&con, vpk)? else { return Ok(None) };
                let path = config::preview_path(self.catalog_dir(), &preview.sidecar);
                // Capture One's own render of the edits, for the look the preview lacks.
                let thumb = config::thumbnail_path(self.catalog_dir(), vpk);
                Ok(path.map(|path| ImageRef {
                    path,
                    format: ImageFormat::JpegXl,
                    white_balance: preview.white_balance,
                    rotation: preview.rotation,
                    as_stored: false,
                    monochrome: preview.monochrome,
                    look: thumb.is_file().then_some(thumb),
                    straighten: preview.straighten,
                    crop: preview.crop,
                }))
            }
            ImageTier::Original => {
                let con = catalog::open_live(&self.catalog)?;
                let Some(path) = catalog::original_path(&con, self.catalog_dir(), vpk)?.map(PathBuf::from) else {
                    return Ok(None);
                };
                if !path.is_file() {
                    return Ok(None);
                }
                let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
                // Framed as the preview is. Capture One turns the pixels as stored (a portrait
                // JPEG lies on its side with ZROTATION 270), so EXIF orientation is left out.
                // The crop is in fractions of the picture, which the camera's embedded JPEG
                // shows whole, as the sensor image does.
                let framing = catalog::preview_source(&con, vpk)?;
                Ok(Some(ImageRef {
                    path,
                    format: match ext.as_deref() {
                        Some("jpg" | "jpeg") => ImageFormat::Jpeg,
                        Some("jxl") => ImageFormat::JpegXl,
                        _ => ImageFormat::Raw,
                    },
                    white_balance: None,
                    rotation: framing.as_ref().map_or(0, |f| f.rotation),
                    as_stored: true,
                    monochrome: false,
                    look: None,
                    straighten: framing.as_ref().map_or(0.0, |f| f.straighten),
                    crop: framing.and_then(|f| f.crop),
                }))
            }
        }
    }

    fn write_status(&self) -> WriteStatus {
        write::status(&self.catalog)
    }

    /// Reads the live catalog, so it works while Capture One is open.
    fn plan(&self, changes: &[Change]) -> Result<ApplyPlan> {
        self.plan_after(&[], &[], changes)
    }

    fn plan_after(
        &self,
        remaps: &[KeywordRemap],
        collections: &[CollectionChange],
        changes: &[Change],
    ) -> Result<ApplyPlan> {
        let con = catalog::open_live(&self.catalog)?;
        let mut plan = write::plan(&con, remaps, collections, changes, |c| self.variant_pk(&c.photo))?;
        plan.remaps = remaps.to_vec();
        plan.collections = collections.to_vec();
        Ok(plan)
    }

    fn check_remap(&self, earlier: &[KeywordRemap], remap: &KeywordRemap) -> Result<Option<String>> {
        let con = catalog::open_live(&self.catalog)?;
        keywords::check_remap(&con, earlier, remap)
    }

    fn check_collection(&self, earlier: &[CollectionChange], change: &CollectionChange) -> Result<Option<String>> {
        let con = catalog::open_live(&self.catalog)?;
        albums::check(&con, earlier, change)
    }

    /// Refuses while Capture One or anything else has the catalog open, backs it up,
    /// then writes the plan in one transaction after checking it still holds.
    fn apply(&self, plan: &ApplyPlan) -> Result<ApplyReport> {
        if plan.apply.is_empty() && plan.remaps.is_empty() && plan.collections.is_empty() {
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

    fn backups(&self) -> Result<Vec<BackupInfo>> {
        Ok(CaptureOne::backups(self).iter().map(backup_info).collect())
    }

    /// Refuses while Capture One or anything else has the catalog open, as a write does.
    fn restore(&self, id: &str) -> Result<BackupInfo> {
        let _guard = self.snapshot_lock.lock().unwrap();
        write::preflight(&self.catalog)?;
        let before = write::restore(&self.catalog, &self.backup_dir(), write::KEEP_BACKUPS, id)?;
        Ok(backup_info(&before))
    }
}

fn backup_info(b: &Backup) -> BackupInfo {
    // Manifests from before `created_ms` carry only the local time.
    let created_ms = if b.created_ms > 0 {
        b.created_ms
    } else {
        chrono::NaiveDateTime::parse_from_str(&b.created, "%Y-%m-%dT%H:%M:%S%.f")
            .ok()
            .and_then(|t| t.and_local_timezone(chrono::Local).earliest())
            .map_or(0, |t| t.timestamp_millis().max(0) as u64)
    };
    BackupInfo {
        id: b.backup.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        path: b.backup.display().to_string(),
        created_ms,
        size: b.size,
        note: b.note.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use c1model::{Collection, CollectionId, CollectionKind, Condition, Op, SmartRule, conformance};

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
        let compat = source.info().compatibility;
        assert_eq!((compat.read, compat.write), (c1model::Support::Tested, c1model::Support::Off));
        assert!(compat.format.contains("schema 1650"), "{}", compat.format);
    }

    #[test]
    fn a_catalog_that_cannot_be_read_says_which_version_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = dir.path().join("old.cocatalogdb");
        let Some(con) = write::tests::fixture(&catalog) else { return };
        con.execute_batch(
            "DROP TABLE ZPATHLOCATION; UPDATE ZVERSIONINFO SET ZVERSION = 1531, ZAUTHOR = '15.3.1.17 Pro Win'",
        )
        .unwrap();
        drop(con);
        let source = CaptureOne::open(&catalog, dir.path().join("work")).unwrap();
        let error = source.snapshot(None, &mut |_| Ok(())).unwrap_err().to_string();
        assert!(error.contains("Capture One 15.3.1 Pro (Windows), catalog schema 1531"), "{error}");
        assert!(error.contains("not checked against") && error.contains("ZPATHLOCATION"), "{error}");
        assert_eq!(source.info().compatibility.read, c1model::Support::Untested);
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

    /// The source's collections, and each photo's (by variant pk).
    fn read(source: &CaptureOne) -> (Vec<Collection>, HashMap<String, Vec<CollectionId>>) {
        let mut collections = Vec::new();
        let mut members = HashMap::new();
        source
            .snapshot(None, &mut |e| {
                match e {
                    SourceEvent::Collections(c) => collections = c,
                    SourceEvent::Upsert(p) => {
                        members.insert(p.id.ext.clone(), p.collections.clone());
                    }
                    _ => {}
                }
                Ok(())
            })
            .unwrap();
        (collections, members)
    }

    #[test]
    fn an_original_is_framed_as_capture_one_shows_it() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = dir.path().join("fixture.cocatalogdb");
        let Some(con) = write::tests::fixture(&catalog) else { return };
        // Imported into the catalog folder, turned to portrait and cropped (variant 1's
        // combined layer is layer 3).
        con.execute_batch(
            "UPDATE ZPATHLOCATION SET ZISRELATIVE = 1, ZRELATIVEPATH = 'Originals';
             UPDATE ZVARIANTLAYER SET ZROTATION = 270, ZCROP = '3000;2000;2000;3000' WHERE Z_PK = 3;",
        )
        .unwrap();
        drop(con);
        std::fs::create_dir_all(dir.path().join("Originals")).unwrap();
        std::fs::write(dir.path().join("Originals").join("DSCF0001.RAF"), b"raw").unwrap();
        let source = CaptureOne::open(&catalog, dir.path().join("work")).unwrap();
        let original = |pk: &str| source.image(&PhotoId::new(&source.info().id, pk), ImageTier::Original).unwrap();
        let framed = original("1").expect("the original is there");
        assert_eq!((framed.format, framed.rotation, framed.as_stored), (ImageFormat::Raw, 270, true));
        let crop = framed.crop.expect("cropped");
        assert!((crop.width - 0.5).abs() < 1e-3 && (crop.height - 0.5).abs() < 1e-3, "{crop:?}");
        // Variant 2's original is not on disk.
        assert_eq!(original("2"), None);
    }

    #[test]
    fn albums_are_made_filled_renamed_and_deleted() {
        let Some((_dir, source)) = source() else { return };
        if !ready(&source) {
            return;
        }
        let sid = source.info().id;
        let (group, album, smart) =
            (CollectionId::new_local("g-1"), CollectionId::new_local("a-1"), CollectionId::new_local("s-1"));
        let rule = SmartRule { match_all: true, conditions: vec![Condition::Rating { min: 4, max: 5 }] };
        let create = |id: &CollectionId, name: &str, kind, parent: Option<&CollectionId>, rule| {
            CollectionChange::Create { id: id.clone(), name: name.into(), kind, parent: parent.cloned(), rule }
        };
        let creates = [
            create(&group, "trips", CollectionKind::Group, None, None),
            create(&album, "japan", CollectionKind::Album, Some(&group), None),
            create(&smart, "best", CollectionKind::Smart, None, Some(rule.clone())),
        ];
        let member = |pk: &str, c: &CollectionId, add: bool| Change {
            photo: PhotoId::new(&sid, pk),
            op: if add {
                Op::AddToCollection { collection: c.clone() }
            } else {
                Op::RemoveFromCollection { collection: c.clone() }
            },
        };
        // Photos go into an album made in the same write; never into a smart album.
        let plan = source
            .plan_after(
                &[],
                &creates,
                &[member("1", &album, true), member("2", &album, true), member("3", &smart, true)],
            )
            .unwrap();
        assert_eq!(plan.apply.len(), 2);
        assert!(plan.conflicts[0].current.contains("smart album"), "{:?}", plan.conflicts);
        assert_eq!(source.apply(&plan).unwrap().applied, 5);

        let (collections, members) = read(&source);
        let japan = collections.iter().find(|c| c.name == "japan").unwrap().clone();
        assert_eq!((japan.path.as_str(), japan.kind), ("trips / japan", CollectionKind::Album));
        let best = collections.iter().find(|c| c.name == "best").unwrap().clone();
        assert_eq!((best.kind, best.rule.as_ref()), (CollectionKind::Smart, Some(&rule)));
        assert_eq!(members["1"], std::slice::from_ref(&japan.id));
        assert_eq!(members["2"], std::slice::from_ref(&japan.id));
        assert!(members["3"].is_empty());

        // By the catalog's own ids from here on.
        let changes = [
            CollectionChange::Rename { id: japan.id.clone(), name: "nippon".into() },
            CollectionChange::SetRule { id: best.id.clone(), rule: SmartRule { match_all: false, ..rule.clone() } },
        ];
        let plan =
            source.plan_after(&[], &changes, &[member("2", &japan.id, false), member("3", &japan.id, false)]).unwrap();
        assert_eq!((plan.apply.len(), plan.no_ops.len()), (1, 1));
        source.apply(&plan).unwrap();
        let (collections, members) = read(&source);
        assert!(collections.iter().any(|c| c.path == "trips / nippon"));
        assert!(!collections.iter().find(|c| c.id == best.id).unwrap().rule.as_ref().unwrap().match_all);
        assert_eq!(members["1"], std::slice::from_ref(&japan.id));
        assert!(members["2"].is_empty());

        // A group goes with what is in it, links and all; the photos stay.
        let trips = collections.iter().find(|c| c.name == "trips").unwrap().id.clone();
        let delete = [CollectionChange::Delete { id: trips }, CollectionChange::Delete { id: best.id.clone() }];
        source.apply(&source.plan_after(&[], &delete, &[]).unwrap()).unwrap();
        let (collections, members) = read(&source);
        assert!(collections.is_empty(), "{collections:?}");
        assert_eq!(members.len(), 5);
        assert!(members["1"].is_empty());
        let con = rusqlite::Connection::open(source.catalog_path()).unwrap();
        let links: i64 = con
            .query_row(
                "SELECT (SELECT COUNT(*) FROM ZIMAGEINCOLLECTION) + (SELECT COUNT(*) FROM ZVARIANTINCOLLECTION)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(links, 0);
    }
}
