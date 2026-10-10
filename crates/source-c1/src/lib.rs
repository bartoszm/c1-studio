//! Capture One's catalog as a [`PhotoSource`].
//!
//! A catalog is a SQLite database with Core Data–style `Z*` tables; Capture One for
//! Windows has no scripting API, so this database is the only programmable surface.
//! `legacy/c1cat-py/skills/capture-one-catalog/references/schema.md` documents every
//! schema fact relied on here; each one was verified against a real catalog.

mod catalog;
mod config;
mod extract;
mod process;
mod smart;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use c1model::source::Result;
use c1model::{
    ApplyPlan, ApplyReport, Capabilities, Change, Cursor, ImageFormat, ImageRef, ImageTier, PhotoId, PhotoSource,
    SourceError, SourceEvent, SourceId, SourceInfo, WriteStatus,
};

pub use catalog::EXPECTED_ZVERSION;
pub use config::{catalog_id, data_dir, find_catalog};
pub use extract::{Extract, extract};

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
        // Read-only until the write path is ported with differential tests (stage 2).
        Capabilities {
            writable: vec![],
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
        match process::running_capture_one() {
            Some(name) => WriteStatus::Locked { by: name },
            None => WriteStatus::ReadOnly,
        }
    }

    fn plan(&self, _changes: &[Change]) -> Result<ApplyPlan> {
        Err(SourceError::Unsupported("writing to Capture One (stage 2)"))
    }

    fn apply(&self, _plan: &ApplyPlan) -> Result<ApplyReport> {
        Err(SourceError::Unsupported("writing to Capture One (stage 2)"))
    }
}
