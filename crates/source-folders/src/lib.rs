//! Plain photo folders as a [`PhotoSource`].
//!
//! A source is a set of root folders, listed with what to leave out in a definition file
//! (`*.c1folders`, [`Definition`]) whose path is the source's location. The roots are
//! walked for JPEG, PNG, TIFF and camera raw files; what each file says about itself (EXIF,
//! XMP, IPTC) is read once and kept, with the id the file was given, in the provider's own
//! store (`folders.redb` in its `work_dir`). The user's edits go into that store too, on
//! top of what the files say, and are the truth; the originals are never changed unless the
//! definition allows XMP to be written into them.
//!
//! A photo keeps its id when its file is moved or renamed, or its root is picked again:
//! files are told apart by their content ([`ids`]), not their path.
//!
//! The roots are walked when the source is opened and when asked to ([`PhotoSource::rescan`]);
//! in between, snapshots come from the store and cost nothing.

pub mod definition;
mod export;
pub mod ids;
pub mod images;
pub mod layer;
pub mod meta;
pub mod pattern;
mod provider;
pub mod remove;
mod scan;
mod snapshot;
pub mod store;
mod write;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};

use c1model::{
    ApplyPlan, ApplyReport, BackupInfo, Capabilities, Change, CollectionChange, Compatibility, Cursor, Field, ImageRef,
    ImageTier, KeywordRemap, PhotoId, PhotoSource, Removal, RemoveReport, Result, SourceError, SourceEvent, SourceInfo,
    SourceProgress, Wording, WriteStatus,
};

pub use definition::{Definition, ImageStore, RootDef, SidecarNaming};

/// The provider's id, as sources are recorded with it.
pub const PROVIDER_ID: &str = "folders";
pub use images::{Migrate, MigrateReport};
pub use pattern::Preset;
pub use provider::FoldersProvider;
pub use scan::ScanOutcome;

use crate::images::Images;
use crate::store::Store;

/// The store's file in the provider's `work_dir`.
pub const STORE_FILE: &str = "folders.redb";

pub struct FolderSource {
    location: PathBuf,
    work_dir: PathBuf,
    id: c1model::SourceId,
    def: RwLock<Definition>,
    store: Store,
    images: Images,
    progress: Mutex<Option<SourceProgress>>,
    /// The next snapshot walks the roots first.
    scan_due: AtomicBool,
    /// One scan or one write at a time.
    pub(crate) busy: Mutex<()>,
    last_scan: Mutex<Option<ScanOutcome>>,
    recycler: RwLock<remove::Recycler>,
    backup_heads: write::BackupHeads,
    trees: Mutex<snapshot::Trees>,
}

impl FolderSource {
    /// Opens the source the definition file at `location` describes, keeping its store in
    /// `work_dir`. The roots are walked on the first snapshot.
    pub fn open(location: &Path, work_dir: &Path) -> Result<Self> {
        let def = Definition::load(location).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => SourceError::NotFound(location.display().to_string()),
            _ => SourceError::Other(e.to_string()),
        })?;
        std::fs::create_dir_all(work_dir)?;
        let store = Store::open(&work_dir.join(STORE_FILE))?;
        Ok(Self {
            location: location.to_owned(),
            work_dir: work_dir.to_owned(),
            id: def.source_id(),
            def: RwLock::new(def),
            store,
            images: Images::new(),
            progress: Mutex::new(None),
            scan_due: AtomicBool::new(true),
            busy: Mutex::new(()),
            last_scan: Mutex::new(None),
            backup_heads: Default::default(),
            trees: Default::default(),
            recycler: RwLock::new(remove::recycle_bin),
        })
    }

    /// Sends the files of photos deleted for good through `recycler` instead of the
    /// system's recycle bin (tests).
    pub fn set_recycler(&self, recycler: remove::Recycler) {
        *self.recycler.write().unwrap() = recycler;
    }

    pub(crate) fn recycler(&self) -> remove::Recycler {
        *self.recycler.read().unwrap()
    }

    /// The definition as last read.
    pub fn definition(&self) -> Definition {
        self.def.read().unwrap().clone()
    }

    /// The definition file.
    pub fn location(&self) -> &Path {
        &self.location
    }

    /// The provider's own folder: the store, backups, and the central image store (`{data}`).
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    /// What the last walk of the roots found, once one has run.
    pub fn last_scan(&self) -> Option<ScanOutcome> {
        self.last_scan.lock().unwrap().clone()
    }

    /// Reads the definition file again (after the app or the user changed it). A changed
    /// image store moves the images already made, or drops them to be made again, as `how`
    /// says (`None` leaves them where they are, unused). The next snapshot walks the roots.
    pub fn reload_definition(&self, how: Option<Migrate>) -> Result<Option<MigrateReport>> {
        let new = Definition::load(&self.location).map_err(|e| SourceError::Other(e.to_string()))?;
        if new.id != self.def.read().unwrap().id {
            return Err(SourceError::Other("the definition file now describes another source".into()));
        }
        if new.images != self.def.read().unwrap().images
            && let Some(problem) = self.images.check(self, &new.images)
        {
            return Err(SourceError::Other(format!("image store: {problem}")));
        }
        let old = std::mem::replace(&mut *self.def.write().unwrap(), new.clone());
        self.scan_due.store(true, Ordering::SeqCst);
        match how {
            Some(how) if old.images != new.images => {
                let _busy = self.busy.lock().unwrap();
                self.images.migrate(self, &old.images, &new.images, how).map(Some)
            }
            _ => Ok(None),
        }
    }

    /// Why `store` cannot hold this source's images (a pattern that is not valid, or that
    /// maps two photos to one file), or `None` if it can.
    pub fn check_image_store(&self, store: &ImageStore) -> Option<String> {
        self.images.check(self, store)
    }

    /// How many thumbnails and previews are recorded as made.
    pub fn image_count(&self) -> usize {
        self.store.read().images.len()
    }

    pub(crate) fn store(&self) -> &Store {
        &self.store
    }

    pub(crate) fn set_progress(&self, progress: Option<SourceProgress>) {
        *self.progress.lock().unwrap() = progress;
    }

    fn check_id(&self, id: &PhotoId) -> Result<()> {
        if id.source == self.id { Ok(()) } else { Err(SourceError::NotFound(id.to_string())) }
    }
}

impl PhotoSource for FolderSource {
    fn info(&self) -> SourceInfo {
        let def = self.def.read().unwrap();
        SourceInfo {
            id: self.id.clone(),
            name: format!("Photo folders · {}", def.name),
            version: store::STORE_VERSION.to_string(),
            compatibility: Compatibility::tested(format!("Photo folders ({} roots)", def.roots.len())),
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
            geo_per_asset: false,
            collections: true,
            rescan: true,
            makes_images: true,
            removes: Removal::Files,
        }
    }

    fn wording(&self) -> Wording {
        Wording {
            name: "Photo folders".into(),
            word: "the photo folders".into(),
            noun: "library".into(),
            // A backup holds what the app keeps beside the files, never the files.
            backup: "edits and albums kept for the folders".into(),
            owner: None,
        }
    }

    fn snapshot(&self, since: Option<&Cursor>, sink: &mut dyn FnMut(SourceEvent) -> Result<()>) -> Result<()> {
        snapshot::run(self, since, sink)
    }

    fn image(&self, id: &PhotoId, tier: ImageTier) -> Result<Option<ImageRef>> {
        self.check_id(id)?;
        self.images.lookup(self, &id.ext, tier)
    }

    fn has_image(&self, id: &PhotoId, tier: ImageTier) -> Result<bool> {
        self.check_id(id)?;
        Ok(self.images.find(self, &id.ext, tier, false)?.is_some())
    }

    fn make_image(&self, id: &PhotoId, tier: ImageTier) -> Result<Option<ImageRef>> {
        self.check_id(id)?;
        self.images.make(self, &id.ext, tier)
    }

    fn write_status(&self) -> WriteStatus {
        WriteStatus::Ready
    }

    fn plan(&self, changes: &[Change]) -> Result<ApplyPlan> {
        write::plan_after(self, &[], &[], changes).map(|mut plan| {
            plan.remaps.clear();
            plan.collections.clear();
            plan
        })
    }

    fn plan_after(
        &self,
        remaps: &[KeywordRemap],
        collections: &[CollectionChange],
        changes: &[Change],
    ) -> Result<ApplyPlan> {
        write::plan_after(self, remaps, collections, changes)
    }

    fn check_remap(&self, earlier: &[KeywordRemap], remap: &KeywordRemap) -> Result<Option<String>> {
        write::check_remap(self, earlier, remap)
    }

    fn check_remaps(&self, remaps: &[KeywordRemap]) -> Result<Vec<Option<String>>> {
        write::check_remaps(self, remaps)
    }

    fn check_collection(&self, earlier: &[CollectionChange], change: &CollectionChange) -> Result<Option<String>> {
        write::check_collection(self, earlier, change)
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplyReport> {
        write::apply(self, plan)
    }

    fn backups(&self) -> Result<Vec<BackupInfo>> {
        write::backups(self)
    }

    fn restore(&self, id: &str) -> Result<BackupInfo> {
        write::restore(self, id)
    }

    fn rescan(&self) -> Result<()> {
        self.scan_due.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn remove(&self, photos: &[PhotoId]) -> Result<RemoveReport> {
        remove::remove(self, photos)
    }

    fn offline_places(&self) -> Vec<String> {
        self.last_scan().map(|scan| scan.offline_roots).unwrap_or_default()
    }

    fn flush(&self) -> Result<()> {
        self.store.flush_lazy()
    }

    fn progress(&self) -> Option<SourceProgress> {
        self.progress.lock().unwrap().clone()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}
