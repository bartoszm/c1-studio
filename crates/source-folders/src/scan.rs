//! Walking the roots: new, changed, moved, removed and offline files, into the store.
//!
//! A walk lists every photo file under each root (leaving out the definition's excludes
//! and the images the source made), then compares with the store:
//! - same path, size and modification time (and sidecar): unchanged, not read;
//! - same path, otherwise changed: read again, its images dropped;
//! - a new path: first matched by content against the records whose path is gone (moved
//!   or renamed), offline or detached, which keeps its id; else a new id;
//! - a known path not found while its root is reachable: removed;
//! - every file under a root that cannot be reached: offline, kept.
//!
//! All of it is written in one store transaction.

use serde::{Deserialize, Serialize};

use c1model::Result;

use crate::FolderSource;

/// What a walk of the roots found.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanOutcome {
    pub added: usize,
    pub changed: usize,
    /// Kept their id at a new path (moved, renamed, or under a root picked again).
    pub moved: usize,
    pub removed: usize,
    /// Roots that could not be reached, by display name; their photos are offline.
    pub offline_roots: Vec<String>,
    /// Files that could not be read (with why), left out or kept as they were.
    pub problems: Vec<String>,
    pub millis: u64,
}

/// Walks the roots and brings the store up to date with them.
pub(crate) fn scan(src: &FolderSource) -> Result<ScanOutcome> {
    let _ = src;
    Ok(ScanOutcome::default()) // WP2
}
