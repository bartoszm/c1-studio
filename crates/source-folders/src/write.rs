//! Writing edits into the store: plans, apply, backups and restoring them.
//!
//! Plans and writes work on the photos as the user sees them ([`crate::layer::compose`]),
//! through `c1model::change::{plan_after_with, apply_with}`; what changed goes back into the
//! store as each photo's [`crate::layer::UserLayer`]. Before a write the user's part of the
//! store (layers and albums) is saved as a backup in `<work_dir>/backups`, newest five kept.

use c1model::{ApplyPlan, ApplyReport, BackupInfo, Change, CollectionChange, KeywordRemap, Result};

use crate::FolderSource;

pub(crate) fn plan_after(
    src: &FolderSource,
    remaps: &[KeywordRemap],
    collections: &[CollectionChange],
    changes: &[Change],
) -> Result<ApplyPlan> {
    let _ = (src, remaps, collections, changes);
    todo!("WP3")
}

pub(crate) fn check_remap(
    src: &FolderSource,
    earlier: &[KeywordRemap],
    remap: &KeywordRemap,
) -> Result<Option<String>> {
    let _ = (src, earlier, remap);
    todo!("WP3")
}

pub(crate) fn check_collection(
    src: &FolderSource,
    earlier: &[CollectionChange],
    change: &CollectionChange,
) -> Result<Option<String>> {
    let _ = (src, earlier, change);
    todo!("WP3")
}

pub(crate) fn apply(src: &FolderSource, plan: &ApplyPlan) -> Result<ApplyReport> {
    let _ = (src, plan);
    todo!("WP3")
}

pub(crate) fn backups(src: &FolderSource) -> Result<Vec<BackupInfo>> {
    let _ = src;
    todo!("WP3")
}

pub(crate) fn restore(src: &FolderSource, id: &str) -> Result<BackupInfo> {
    let _ = (src, id);
    todo!("WP3")
}
