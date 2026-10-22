//! The contract between a data source and the rest of the app.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::change::{ApplyPlan, ApplyReport, Change, CollectionChange, KeywordRemap};
use crate::photo::{Collection, KeywordNode, Photo, PhotoId, SourceId};

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("source not found: {0}")]
    NotFound(String),
    #[error("source is locked by {by}; close it and try again")]
    Locked { by: String },
    #[error("not supported by this source: {0}")]
    Unsupported(&'static str),
    #[error("source schema is not the one this build understands: {0}")]
    Schema(String),
    #[error("plan is stale: the source changed after it was made")]
    StalePlan,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T, E = SourceError> = std::result::Result<T, E>;

/// Opaque position in a source's history. Only the source that issued it reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor(pub String);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceInfo {
    pub id: SourceId,
    /// Human-readable, e.g. "Capture One · fiuji".
    pub name: String,
    /// Source-specific version, e.g. the catalog schema version.
    pub version: String,
    pub compatibility: Compatibility,
}

/// How far the app has been checked against a source's format, one way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Support {
    /// Checked against this version.
    Tested,
    /// Not checked: the app tries, and says so.
    Untested,
    /// Not done for this version.
    Off,
}

/// What made a source and how far the app trusts itself with it, for telling the user,
/// above all when something does not work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compatibility {
    /// The program and version that made it: "Capture One 16.5.5 Pro (Windows), catalog
    /// schema 1650".
    pub format: String,
    pub read: Support,
    pub write: Support,
    /// Why, in a sentence or two, when either is not `Tested`.
    pub note: Option<String>,
}

impl Compatibility {
    pub fn tested(format: impl Into<String>) -> Self {
        Self { format: format.into(), read: Support::Tested, write: Support::Tested, note: None }
    }
}

/// Fields a source can write back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Field {
    Rating,
    Color,
    Keywords,
    Iptc,
    Place,
    Geo,
    Collections,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub writable: Vec<Field>,
    pub tiers: Vec<ImageTier>,
    /// Geo is stored per asset, so editing one variant moves all of them (Capture One).
    pub geo_per_asset: bool,
    pub collections: bool,
    /// [`PhotoSource::rescan`] looks for changes the source cannot see by itself (files
    /// added on disk).
    #[serde(default)]
    pub rescan: bool,
    /// [`PhotoSource::make_image`] renders thumbnails and previews the source does not
    /// have yet (from the original).
    #[serde(default)]
    pub makes_images: bool,
    /// What [`PhotoSource::remove`] does to a photo deleted for good.
    #[serde(default)]
    pub removes: Removal,
}

/// What deleting a photo for good does at the source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Removal {
    /// The source cannot delete photos.
    #[default]
    None,
    /// The photo's files go (to the system's recycle bin, for a folder source); the next
    /// snapshot deletes the photo.
    Files,
    /// The source marks the photo deleted in its own trash ([`Photo::trashed`]), and every
    /// variant of the same asset with it (Capture One); the files stay.
    MarksTrashed,
}

/// What [`PhotoSource::remove`] did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoveReport {
    /// Deleted for good, or marked deleted: the next snapshot shows it.
    pub removed: Vec<PhotoId>,
    /// Left as they were, each with why (the source no longer has it, its drive is offline).
    pub failed: Vec<(PhotoId, String)>,
    /// The copy made first, for a source that keeps them (Capture One).
    pub backup: Option<String>,
    /// What went wrong after the photos were removed, which did not undo it (a file that
    /// could not be moved to the recycle bin yet).
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Capabilities {
    pub fn can_write(&self, field: Field) -> bool {
        self.writable.contains(&field)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WriteStatus {
    Ready,
    /// Another program holds the source; writing now would lose data.
    Locked {
        by: String,
    },
    ReadOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageTier {
    /// Small, fast, already rendered with the photo's edits (~450 px).
    Thumb,
    /// Screen-sized render (~2000 px).
    Preview,
    /// The original file; may be offline.
    Original,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Jpeg,
    JpegXl,
    /// A camera raw; the reader extracts its embedded preview.
    Raw,
    Png,
    Tiff,
}

/// Where to get the pixels, plus how to turn them into display-ready sRGB.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageRef {
    pub path: PathBuf,
    pub format: ImageFormat,
    /// Per-channel multipliers for linear camera-space data (Capture One previews).
    pub white_balance: Option<[f32; 3]>,
    /// Clockwise degrees to turn the decoded pixels, 0, 90, 180 or 270: for files stored
    /// in sensor orientation (Capture One previews). EXIF orientation is applied first,
    /// unless `as_stored` says the turn starts from the pixels as stored.
    pub rotation: u16,
    /// Read the pixels as stored, ignoring EXIF orientation: `rotation` already holds the
    /// whole turn (Capture One's for an original, whose own includes the camera's).
    #[serde(default)]
    pub as_stored: bool,
    /// Render in black and white (the source's develop settings say so).
    pub monochrome: bool,
    /// A small render of the same picture with the source's edits applied (Capture One's
    /// thumbnail), framed like this image once turned and straightened. Its colours are
    /// matched onto this image, and if it is grey, so is this.
    pub look: Option<PathBuf>,
    /// Clockwise degrees to straighten by after `rotation`, under 45 either way. The canvas
    /// grows to hold the turned picture.
    #[serde(default)]
    pub straighten: f32,
    /// The part of the turned and straightened picture to show.
    #[serde(default)]
    pub crop: Option<Crop>,
}

/// A crop, in the displayed orientation: the offset of its centre from the picture's
/// centre and its size, as fractions of the picture's width and height before
/// straightening grew the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    pub dx: f32,
    pub dy: f32,
    pub width: f32,
    pub height: f32,
}

/// How far a long piece of work in the source has got (a folder scan), for showing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceProgress {
    /// What it is doing: "Scanning folders", "Reading metadata".
    pub phase: String,
    pub done: u64,
    /// `None` while the amount of work is not known yet.
    pub total: Option<u64>,
    /// A detail, such as the folder being read.
    pub note: Option<String>,
}

/// One item of a snapshot, delivered in order.
#[derive(Clone, Debug, PartialEq)]
pub enum SourceEvent {
    /// A complete snapshot follows: anything the consumer holds for this source and
    /// does not see again before [`SourceEvent::End`] has been deleted.
    BeginFull,
    /// Only changes follow.
    BeginIncremental,
    Upsert(Box<Photo>),
    Delete(PhotoId),
    Collections(Vec<Collection>),
    Keywords(Vec<KeywordNode>),
    End(Cursor),
}

/// A place photos come from. Implementations are blocking; callers that must not block
/// (the UI) run them on a worker thread.
/// How the app names a source in its text, so the UI need not know the provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wording {
    /// As a label: "Capture One", "Photo folders".
    pub name: String,
    /// In a sentence: "Write to Capture One", "write to the photo folders".
    pub word: String,
    /// What holds the photos, after "the": "catalog", "library".
    pub noun: String,
    /// The program that keeps the source open and has to be closed before the app writes
    /// to it: "Capture One". `None` when nothing does.
    pub owner: Option<String>,
}

impl Default for Wording {
    fn default() -> Self {
        Wording { name: "Source".into(), word: "the source".into(), noun: "library".into(), owner: None }
    }
}

pub trait PhotoSource: Send + Sync {
    fn info(&self) -> SourceInfo;

    fn capabilities(&self) -> Capabilities;

    /// How the app names this source in its text.
    fn wording(&self) -> Wording {
        Wording::default()
    }

    /// Streams the source's state into `sink`. With `since` set to a cursor from an
    /// earlier call, a source may send only what changed; if nothing changed it sends
    /// `BeginIncremental` then `End` with the same cursor.
    fn snapshot(&self, since: Option<&Cursor>, sink: &mut dyn FnMut(SourceEvent) -> Result<()>) -> Result<()>;

    /// Locates pixels for a photo. `Ok(None)` means this tier does not exist for it
    /// (not rendered yet, or the original's volume is offline). Called for every photo in
    /// bulk, so it only looks things up: making an image is [`PhotoSource::make_image`].
    fn image(&self, id: &PhotoId, tier: ImageTier) -> Result<Option<ImageRef>>;

    /// Makes a tier [`PhotoSource::image`] does not have yet, for a source that renders its
    /// own (from the original), and returns it. Slow: called off the UI thread.
    ///
    /// Callable from several threads at once, for different photos and also for the same
    /// one (a thumbnail fill and a tile asking for it); a source makes each image once and
    /// is safe under that. `Ok(None)`: the source does not make images, or has nothing to
    /// make one from right now (the original's volume is offline), which is not an error.
    /// `Err`: it tried and failed. After `Ok(Some(r))`, [`PhotoSource::image`] for the same
    /// tier returns the image made, until the source drops it.
    fn make_image(&self, _id: &PhotoId, _tier: ImageTier) -> Result<Option<ImageRef>> {
        Ok(None)
    }

    /// Whether [`PhotoSource::image`] has the tier for a photo, for checks over the whole
    /// library: unlike `image`, asking does not count as using the image (a source that
    /// trims its least recently used images keeps its order).
    fn has_image(&self, id: &PhotoId, tier: ImageTier) -> Result<bool> {
        Ok(self.image(id, tier)?.is_some())
    }

    /// [`PhotoSource::image`], else, for a thumbnail or preview, one
    /// [`PhotoSource::make_image`] makes: what showing a photo asks for.
    fn image_or_make(&self, id: &PhotoId, tier: ImageTier) -> Result<Option<ImageRef>> {
        match self.image(id, tier)? {
            Some(found) => Ok(Some(found)),
            None if tier != ImageTier::Original => self.make_image(id, tier),
            None => Ok(None),
        }
    }

    /// Places of the source that could not be reached at its last read (a drive not
    /// mounted), named for the user. Their photos are kept, marked offline.
    fn offline_places(&self) -> Vec<String> {
        Vec::new()
    }

    /// Looks again for changes the source cannot notice by itself (files added, changed or
    /// removed on disk); the next [`PhotoSource::snapshot`] reports them.
    ///
    /// A cheap request: the work, and the time it takes, land in that next `snapshot`
    /// (minutes for a large folder), which also changes the source's state. Callers ask
    /// [`Capabilities::rescan`] first; a source with it false returns `Unsupported`.
    fn rescan(&self) -> Result<()> {
        Err(SourceError::Unsupported("rescanning"))
    }

    /// Where a long-running snapshot or rescan has got, while it runs. `None` between runs.
    ///
    /// Polled from another thread while `snapshot` runs, so it is cheap and never waits on
    /// a lock the run holds for long. Advisory: a reading may be a little behind.
    fn progress(&self) -> Option<SourceProgress> {
        None
    }

    /// The concrete source, for operations only one kind of source has (a folder source's
    /// image store). `None` for a source that offers none. Only the code that manages that
    /// kind of source downcasts; what every source can tell belongs on this trait.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    fn write_status(&self) -> WriteStatus;

    /// Dry run: sorts changes into apply / conflict / no-op without writing.
    fn plan(&self, changes: &[Change]) -> Result<ApplyPlan>;

    /// Like [`PhotoSource::plan`], with the source as it will be once `remaps` and
    /// `collections` are applied; the plan carries both.
    fn plan_after(
        &self,
        remaps: &[KeywordRemap],
        collections: &[CollectionChange],
        changes: &[Change],
    ) -> Result<ApplyPlan> {
        let mut plan = self.plan(changes)?;
        plan.remaps = remaps.to_vec();
        plan.collections = collections.to_vec();
        Ok(plan)
    }

    /// Why a keyword tree change can no longer be written (its keywords are gone), or
    /// `None` if it can, once the `earlier` ones have been applied.
    fn check_remap(&self, _earlier: &[KeywordRemap], _remap: &KeywordRemap) -> Result<Option<String>> {
        Ok(None)
    }

    /// [`PhotoSource::check_remap`] for remaps queued one after another: each checked once
    /// the ones before it that can be written are applied. One answer per remap.
    fn check_remaps(&self, remaps: &[KeywordRemap]) -> Result<Vec<Option<String>>> {
        let mut ok = Vec::new();
        let mut whys = Vec::new();
        for remap in remaps {
            let why = self.check_remap(&ok, remap)?;
            if why.is_none() {
                ok.push(remap.clone());
            }
            whys.push(why);
        }
        Ok(whys)
    }

    /// Why an album change can no longer be written (the album or its parent is gone),
    /// or `None` if it can, once the `earlier` ones have been applied.
    fn check_collection(&self, _earlier: &[CollectionChange], _change: &CollectionChange) -> Result<Option<String>> {
        Ok(None)
    }

    /// Writes a plan produced by [`PhotoSource::plan`]: its keyword remaps first, then
    /// its album changes, then its photo changes. The write to the source's own store is
    /// atomic: all or nothing. What follows it (a folder source's XMP files) is not: each
    /// file is written whole or not at all, but a failure does not undo the write. Those
    /// are listed in [`ApplyReport::warnings`], and are not covered by
    /// [`PhotoSource::backups`] / [`PhotoSource::restore`], which keep what the source
    /// holds itself.
    fn apply(&self, plan: &ApplyPlan) -> Result<ApplyReport>;

    /// The copies the source kept before writing, newest first. Empty for a source that
    /// keeps none.
    fn backups(&self) -> Result<Vec<BackupInfo>> {
        Ok(Vec::new())
    }

    /// Puts a backup (by [`BackupInfo::id`]) back as the source. The source as it was is
    /// backed up first, so this can be undone the same way; that backup is returned.
    fn restore(&self, _id: &str) -> Result<BackupInfo> {
        Err(SourceError::Unsupported("restoring backups"))
    }

    /// Deletes photos for good, as [`Capabilities::removes`] says: their files go, or the
    /// source marks them deleted. Not undone by the app; a source that keeps backups makes
    /// one first. A photo the source cannot remove (gone already, on an offline drive,
    /// changed on disk since it was read) is listed in [`RemoveReport::failed`] and the
    /// others still go. The next [`PhotoSource::snapshot`] reports what changed. Callers ask
    /// `removes` first; a source with [`Removal::None`] returns `Unsupported`.
    fn remove(&self, _photos: &[PhotoId]) -> Result<RemoveReport> {
        Err(SourceError::Unsupported("deleting photos"))
    }
}

/// A copy of a source kept before writing to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackupInfo {
    /// What [`PhotoSource::restore`] takes: what the source names a copy by (the file name, for Capture One).
    pub id: String,
    /// Where it is, for showing.
    pub path: String,
    /// Milliseconds since the epoch.
    pub created_ms: u64,
    pub size: u64,
    /// Why it was made, when not before an ordinary write: "before restoring …".
    pub note: Option<String>,
}
