//! [`c1model::PhotoSource::snapshot`] from the store: everything, or what changed since a
//! cursor.
//!
//! The cursor is `<store uuid>:<generation>`. Given one of this store's, only what changed
//! after that generation is sent (`BeginIncremental`, upserts, deletes, and the collections
//! and keywords again if they changed); anything else gets everything (`BeginFull`).
//! The roots are walked first when a walk is due (on open, after `rescan`).

use c1model::{Cursor, Result, SourceEvent};

use crate::FolderSource;

pub(crate) fn run(
    src: &FolderSource,
    since: Option<&Cursor>,
    sink: &mut dyn FnMut(SourceEvent) -> Result<()>,
) -> Result<()> {
    let _ = (src, since, sink);
    todo!("WP2")
}
