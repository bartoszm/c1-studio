//! Thumbnails and previews the source makes and keeps, wherever the definition's image
//! store says.
//!
//! - `image()` ([`Images::lookup`]) only looks: an image made earlier whose original has not
//!   changed since, a small JPEG's original standing in for its preview, the original itself.
//! - `make_image()` ([`Images::make`]) decodes the original once and writes both tiers.
//! - The central store keeps under its size limit by dropping the least recently used.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use c1model::{ImageRef, ImageTier, Result};
use serde::{Deserialize, Serialize};

use crate::FolderSource;
use crate::definition::ImageStore;
use crate::store::Writer;

/// What to do with images already made when the image store changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Migrate {
    /// Move each file to where the new store puts it.
    Move,
    /// Delete them; they are made again as needed.
    Regenerate,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MigrateReport {
    pub moved: usize,
    pub removed: usize,
    pub failed: Vec<String>,
}

pub(crate) struct Images {
    /// Photo id → last time (ms) one of its images was looked up, for the central store's
    /// least-recently-used trimming. Kept in memory only.
    access: Mutex<HashMap<String, u64>>,
}

impl Images {
    pub fn new() -> Self {
        Self { access: Mutex::new(HashMap::new()) }
    }

    /// The image of `tier` there is now for photo `ext`, without making anything.
    pub fn lookup(&self, src: &FolderSource, ext: &str, tier: ImageTier) -> Result<Option<ImageRef>> {
        let _ = (src, ext, tier, &self.access);
        Ok(None) // WP4
    }

    /// Makes the thumbnail and the preview of photo `ext` (if they are not there and up to
    /// date) and returns the one of `tier`.
    pub fn make(&self, src: &FolderSource, ext: &str, tier: ImageTier) -> Result<Option<ImageRef>> {
        let _ = (src, ext, tier);
        Ok(None) // WP4
    }

    /// The file of photo `ext` changed or is gone: deletes the images made of it and their
    /// records. Called inside a store write: use `w.state()`, never `src.store()`.
    pub fn invalidate(&self, w: &mut Writer, src: &FolderSource, ext: &str) -> Result<()> {
        let _ = (w, src, ext);
        Ok(()) // WP4
    }

    /// `path` is an image this source made (so a walk leaves it out).
    pub fn is_generated(&self, src: &FolderSource, path: &Path) -> bool {
        let _ = (src, path);
        false // WP4
    }

    /// Drops least recently used images until the central store is under its limit.
    pub fn evict(&self, src: &FolderSource) -> Result<()> {
        let _ = src;
        Ok(()) // WP4
    }

    /// Moves the images made under `old` to where `new` puts them, or deletes them.
    pub fn migrate(
        &self,
        src: &FolderSource,
        old: &ImageStore,
        new: &ImageStore,
        how: Migrate,
    ) -> Result<MigrateReport> {
        let _ = (src, old, new, how);
        Ok(MigrateReport::default()) // WP4
    }

    /// Why `store` cannot hold this source's images, or `None`.
    pub fn check(&self, src: &FolderSource, store: &ImageStore) -> Option<String> {
        let _ = (src, store);
        None // WP4
    }
}
