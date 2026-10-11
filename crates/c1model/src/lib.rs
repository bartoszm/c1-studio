//! The canonical photo model and the [`PhotoSource`] contract.
//!
//! Everything above the data sources — the index, the UI, the map, AI and sync — only
//! knows these types. Capture One is one implementation (`source-c1`); another source is
//! a new crate that implements [`PhotoSource`] and passes [`conformance`].

pub mod change;
pub mod conformance;
pub mod memory;
pub mod photo;
pub mod source;

pub use change::{ApplyPlan, ApplyReport, Change, Conflict, KeywordRemap, Op};
pub use photo::{
    AssetId, Collection, CollectionId, CollectionKind, ColorLabel, Condition, EditInfo, Exif, FileInfo, GeoOrigin,
    GeoPoint, Iptc, KeywordNode, KeywordPath, Photo, PhotoId, Place, SmartRule, SourceId,
};
pub use source::{
    Capabilities, Cursor, Field, ImageFormat, ImageRef, ImageTier, PhotoSource, SourceError, SourceEvent, SourceInfo,
    WriteStatus,
};

/// Bumped whenever a serialised [`Photo`] or [`Collection`] changes shape, so stored copies
/// are rebuilt.
pub const MODEL_VERSION: u32 = 2;
