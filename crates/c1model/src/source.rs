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
}

/// Where to get the pixels, plus how to turn them into display-ready sRGB.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageRef {
    pub path: PathBuf,
    pub format: ImageFormat,
    /// Per-channel multipliers for linear camera-space data (Capture One previews).
    pub white_balance: Option<[f32; 3]>,
    /// Clockwise degrees to turn the decoded pixels, 0, 90, 180 or 270: for files stored
    /// in sensor orientation (Capture One previews). EXIF orientation is applied anyway.
    pub rotation: u16,
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
pub trait PhotoSource: Send + Sync {
    fn info(&self) -> SourceInfo;

    fn capabilities(&self) -> Capabilities;

    /// Streams the source's state into `sink`. With `since` set to a cursor from an
    /// earlier call, a source may send only what changed; if nothing changed it sends
    /// `BeginIncremental` then `End` with the same cursor.
    fn snapshot(&self, since: Option<&Cursor>, sink: &mut dyn FnMut(SourceEvent) -> Result<()>) -> Result<()>;

    /// Locates pixels for a photo. `Ok(None)` means this tier does not exist for it
    /// (not rendered yet, or the original's volume is offline).
    fn image(&self, id: &PhotoId, tier: ImageTier) -> Result<Option<ImageRef>>;

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

    /// Why an album change can no longer be written (the album or its parent is gone),
    /// or `None` if it can, once the `earlier` ones have been applied.
    fn check_collection(&self, _earlier: &[CollectionChange], _change: &CollectionChange) -> Result<Option<String>> {
        Ok(None)
    }

    /// Writes a plan produced by [`PhotoSource::plan`]: its keyword remaps first, then
    /// its album changes, then its photo changes. Atomic: all or nothing.
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
}

/// A copy of a source kept before writing to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackupInfo {
    /// What [`PhotoSource::restore`] takes: the file name, for Capture One.
    pub id: String,
    /// Where it is, for showing.
    pub path: String,
    /// Milliseconds since the epoch.
    pub created_ms: u64,
    pub size: u64,
    /// Why it was made, when not before an ordinary write: "before restoring …".
    pub note: Option<String>,
}
