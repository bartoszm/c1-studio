//! The optional XMP export after a write: sidecars beside RAWs, and XMP embedded in JPEG,
//! TIFF and DNG files when changing originals is allowed.
//!
//! Lightroom reads XMP only on import or "Read Metadata from Files", so this is an export,
//! not a sync. A failure is reported, never undoes the write to the store.

use std::path::PathBuf;

use c1model::Photo;

use crate::definition::Definition;
use crate::meta::FileKind;

/// Writes the XMP for each `(photo as the user sees it, its file, the file's kind)`, as
/// `def` says (nothing unless `def.write_xmp`). Returns what could not be written, with why.
pub(crate) fn export(def: &Definition, photos: &[(Photo, PathBuf, FileKind)]) -> Vec<String> {
    let _ = (def, photos);
    Vec::new() // WP1
}
