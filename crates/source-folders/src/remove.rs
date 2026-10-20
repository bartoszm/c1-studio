//! Deleting photos for good: their files go to the system's recycle bin.
//!
//! The records go first, in one store write that also lists the files to go
//! ([`Writer::discard`]); the files are moved after it, outside the store's lock. A file
//! that cannot be moved yet stays listed, and the walk leaves it out, so a crash or a
//! locked file never brings the photo back as a new one; the next removal or scan tries
//! again.
//!
//! [`Writer::discard`]: crate::store::Writer::discard

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use c1model::{PhotoId, RemoveReport, Result};

use crate::FolderSource;
use crate::scan::mtime_ns;
use crate::store::{FileState, join_rel};

/// Moves files to the recycle bin, all or none: the source's own is [`recycle_bin`];
/// tests use one that deletes.
pub type Recycler = fn(&[PathBuf]) -> std::result::Result<(), String>;

/// The system's recycle bin.
pub fn recycle_bin(paths: &[PathBuf]) -> std::result::Result<(), String> {
    trash::delete_all(paths).map_err(|e| e.to_string())
}

/// A photo that can go, with its files.
struct Going {
    id: PhotoId,
    ext: String,
    files: Vec<PathBuf>,
}

pub(crate) fn remove(src: &FolderSource, photos: &[PhotoId]) -> Result<RemoveReport> {
    let busy = src.busy.lock().unwrap();
    let def = src.definition();
    let mut report = RemoveReport::default();
    let mut going = Vec::new();
    {
        let state = src.store().read();
        let exts: HashSet<&str> = photos.iter().filter(|id| id.source == src.id).map(|id| id.ext.as_str()).collect();
        for id in photos {
            let rec = if id.source == src.id { state.files.get(&id.ext) } else { None };
            let Some(rec) = rec.filter(|_| !state.tombs.contains_key(&id.ext)) else {
                report.failed.push((id.clone(), "not in the source".into()));
                continue;
            };
            let root = match (rec.state, def.root(&rec.root)) {
                (FileState::Present, Some(root)) => root,
                (FileState::Offline, _) => {
                    report.failed.push((id.clone(), "its folder could not be reached".into()));
                    continue;
                }
                _ => {
                    report.failed.push((id.clone(), "its folder is no longer in the source".into()));
                    continue;
                }
            };
            let original = rec.path_under(&root.path);
            let mut files = Vec::new();
            match std::fs::metadata(&original) {
                // Gone already: only the record is left to go.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    report.failed.push((id.clone(), format!("{}: {e}", original.display())));
                    continue;
                }
                Ok(meta) if meta.len() != rec.size || mtime_ns(&meta) != rec.mtime_ns => {
                    report.failed.push((id.clone(), "the file changed since it was read: look again first".into()));
                    continue;
                }
                Ok(_) => files.push(original),
            }
            // A sidecar shared with a file that stays (`X.xmp` of both `X.RAF` and `X.JPG`)
            // stays with it.
            if let Some(sidecar) = &rec.sidecar {
                let shared = state.files.iter().any(|(other, r)| {
                    !exts.contains(other.as_str())
                        && r.root == rec.root
                        && r.reldir() == rec.reldir()
                        && r.sidecar.as_ref().is_some_and(|s| s.eq_ignore_ascii_case(sidecar))
                });
                let path = join_rel(Path::new(&root.path), rec.reldir()).join(sidecar);
                if !shared && path.is_file() {
                    files.push(path);
                }
            }
            going.push(Going { id: id.clone(), ext: id.ext.clone(), files });
        }
    }
    if !going.is_empty() {
        src.store().write(|w| {
            for photo in &going {
                // Before the record goes: the images are found through it.
                src.images.invalidate(w, src, &photo.ext)?;
                w.remove_file(&photo.ext)?;
                for file in &photo.files {
                    w.discard(&photo.ext, file)?;
                }
            }
            Ok(())
        })?;
        // Once written (no scan meanwhile: this holds `busy`), so a failed write leaves
        // the files to be found as photos.
        for file in going.iter().flat_map(|p| &p.files) {
            src.images.discarding(file);
        }
        report.removed = going.into_iter().map(|d| d.id).collect();
    }
    drop(busy);
    src.images.sweep(src);
    report.warnings = recycle(src);
    Ok(report)
}

/// Moves the files still to go to the recycle bin, and says which could not be, and why.
pub(crate) fn recycle(src: &FolderSource) -> Vec<String> {
    let paths: Vec<PathBuf> = src.store().read().discarded.keys().cloned().collect();
    if paths.is_empty() {
        return Vec::new();
    }
    let recycler = src.recycler();
    let mut gone = Vec::new();
    let mut problems = Vec::new();
    // One call for them all (much faster); one by one only to find which failed.
    let (here, missing): (Vec<PathBuf>, Vec<PathBuf>) = paths.into_iter().partition(|p| p.exists());
    gone.extend(missing);
    if here.is_empty() || recycler(&here).is_ok() {
        gone.extend(here);
    } else {
        for path in here {
            match recycler(std::slice::from_ref(&path)) {
                Ok(()) => gone.push(path),
                Err(_) if !path.exists() => gone.push(path),
                Err(e) => problems.push(format!("{}: not moved to the recycle bin yet: {e}", path.display())),
            }
        }
    }
    for path in &gone {
        src.images.discarded(path);
    }
    // A crash that loses this only means trying files that are gone already.
    if let Err(e) = src.store().write_lazy(|w| gone.iter().try_for_each(|path| w.discarded(path))) {
        problems.push(e.to_string());
    }
    problems
}
