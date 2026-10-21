//! Deleting photos for good: their files go to the system's recycle bin.
//!
//! The records go first, in one store write that also lists the files to go, each as it
//! was then ([`Writer::discard`]); the files are moved after it, outside the store's lock. A
//! file that cannot be moved yet stays listed, and the walk leaves it out, so a crash or a
//! locked file never brings the photo back as a new one; the next removal or scan tries
//! again. A file found changed at a listed path is another file: it is left alone.
//!
//! [`Writer::discard`]: crate::store::Writer::discard

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use c1model::{PhotoId, RemoveReport, Result};

use crate::scan::mtime_ns;
use crate::store::{Discarded, FileRec, FileState, join_rel};
use crate::{FolderSource, RootDef};

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
    files: Vec<(PathBuf, Discarded)>,
}

/// A photo whose original can go: its record, root, and the original as it is now (`None`
/// when it is gone already).
struct Accepted<'a> {
    id: &'a PhotoId,
    rec: &'a FileRec,
    root: &'a RootDef,
    file: Option<(PathBuf, Discarded)>,
}

/// A file's size and modification time, as [`Discarded`] keeps them.
fn stamp(path: &Path, ext: &str) -> std::io::Result<Discarded> {
    let meta = std::fs::metadata(path)?;
    Ok(Discarded { ext: ext.to_owned(), size: meta.len(), mtime_ns: mtime_ns(&meta) })
}

pub(crate) fn remove(src: &FolderSource, photos: &[PhotoId]) -> Result<RemoveReport> {
    let busy = src.busy.lock().unwrap();
    let def = src.definition();
    let mut report = RemoveReport::default();
    let mut going = Vec::new();
    {
        let state = src.store().read();
        // The originals first: which photos can go at all.
        let mut accepted: Vec<Accepted> = Vec::new();
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
            if let Some(why) = no_recycle_bin(Path::new(&root.path)) {
                report.failed.push((id.clone(), why));
                continue;
            }
            let original = rec.path_under(&root.path);
            let file = match stamp(&original, &id.ext) {
                // Gone already: only the record is left to go.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    report.failed.push((id.clone(), format!("{}: {e}", original.display())));
                    continue;
                }
                Ok(was) if was.size != rec.size || was.mtime_ns != rec.mtime_ns => {
                    report.failed.push((id.clone(), "the file changed since it was read: look again first".into()));
                    continue;
                }
                Ok(was) => Some((original, was)),
            };
            accepted.push(Accepted { id, rec, root, file });
        }
        // Then their sidecars: one shared with a photo that stays (`X.xmp` of both `X.RAF`
        // and `X.JPG`, one of them refused above or not asked for) stays with it.
        let leaving: HashSet<&str> = accepted.iter().map(|a| a.id.ext.as_str()).collect();
        for Accepted { id, rec, root, file } in accepted {
            let mut files: Vec<(PathBuf, Discarded)> = file.into_iter().collect();
            if let Some(sidecar) = &rec.sidecar {
                let shared = state.files.iter().any(|(other, r)| {
                    !leaving.contains(other.as_str())
                        && r.root == rec.root
                        && r.reldir() == rec.reldir()
                        && r.sidecar.as_ref().is_some_and(|s| s.eq_ignore_ascii_case(sidecar))
                });
                let path = join_rel(Path::new(&root.path), rec.reldir()).join(sidecar);
                if !shared && let Ok(was) = stamp(&path, &id.ext) {
                    files.push((path, was));
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
                for (path, was) in &photo.files {
                    w.discard(path, was.clone())?;
                }
            }
            Ok(())
        })?;
        // Once written (no scan meanwhile: this holds `busy`), so a failed write leaves
        // the files to be found as photos.
        for (path, _) in going.iter().flat_map(|p| &p.files) {
            src.images.discarding(path);
        }
        report.removed = going.into_iter().map(|d| d.id).collect();
    }
    // In the order asked.
    report.failed.sort_by_key(|(id, _)| photos.iter().position(|p| p == id));
    drop(busy);
    src.images.sweep(src);
    report.warnings = recycle(src);
    Ok(report)
}

/// Why files under `root` cannot go to a recycle bin, or `None` if they can. Windows keeps
/// none on removable drives and network shares, and deletes for good there instead.
#[cfg(windows)]
fn no_recycle_bin(root: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Component;
    // The volume's root: `C:\`, `\\server\share\`.
    let mut volume = PathBuf::new();
    for part in root.components() {
        match part {
            Component::Prefix(_) | Component::RootDir => volume.push(part.as_os_str()),
            _ => break,
        }
    }
    let wide: Vec<u16> = volume.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: a NUL-terminated wide string that outlives the call.
    let kind = unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(wide.as_ptr()) };
    // DRIVE_FIXED: a local disk (an external one included), which has a recycle bin.
    const DRIVE_FIXED: u32 = 3;
    (kind != DRIVE_FIXED).then(|| {
        format!("{} has no recycle bin (a removable or network drive): delete it outside C1 Studio", volume.display())
    })
}

#[cfg(not(windows))]
fn no_recycle_bin(_root: &Path) -> Option<String> {
    None
}

/// Moves the files still to go to the recycle bin, and says which could not be, and why.
/// A file no longer as it was listed is another one put there since: it stays, and is
/// taken off the list, so the next walk finds it.
pub(crate) fn recycle(src: &FolderSource) -> Vec<String> {
    let listed: Vec<(PathBuf, Discarded)> =
        src.store().read().discarded.iter().map(|(p, d)| (p.clone(), d.clone())).collect();
    if listed.is_empty() {
        return Vec::new();
    }
    let recycler = src.recycler();
    let mut done = Vec::new();
    let mut here = Vec::new();
    let mut problems = Vec::new();
    for (path, was) in listed {
        match stamp(&path, &was.ext) {
            Ok(now) if now == was => here.push(path),
            // Gone already, or not the file deleted.
            _ => done.push(path),
        }
    }
    // One call for them all (much faster); one by one only to find which failed.
    if here.is_empty() || recycler(&here).is_ok() {
        done.extend(here);
    } else {
        for path in here {
            match recycler(std::slice::from_ref(&path)) {
                Ok(()) => done.push(path),
                Err(_) if !path.exists() => done.push(path),
                Err(e) => problems.push(format!("{}: not moved to the recycle bin yet: {e}", path.display())),
            }
        }
    }
    if let Err(e) = src.store().write(|w| done.iter().try_for_each(|path| w.discarded(path))) {
        problems.push(e.to_string());
        return problems;
    }
    for path in &done {
        src.images.discarded(path);
    }
    problems
}
