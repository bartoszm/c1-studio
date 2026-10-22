//! Thumbnails and previews the source makes and keeps, wherever the definition's image
//! store says.
//!
//! - `image()` ([`Images::lookup`]) only looks: an image made earlier whose original has not
//!   changed since (and whose file is there), a small JPEG's original standing in for its
//!   preview, the original itself.
//! - `make_image()` ([`Images::make`]) decodes the original once and writes the tier asked
//!   for: a thumbnail alone, or a preview with the thumbnail if that is missing.
//! - An image is never written over a file the source did not make.
//! - The central store keeps under its size limit by dropping the least recently used,
//!   previews before thumbnails.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use c1model::{ImageFormat, ImageRef, ImageTier, Result, SourceError};
use serde::{Deserialize, Serialize};

use crate::FolderSource;
use crate::definition::{Definition, ImageStore};
use crate::meta::FileKind;
use crate::pattern::{self, Pattern, Preset, Tokens};
use crate::store::{FileRec, FileState, ImageRec, State, Writer};

/// Long edge of a thumbnail, in pixels.
pub const THUMB_EDGE: u32 = 450;
/// Long edge of a preview, in pixels.
pub const PREVIEW_EDGE: u32 = 2000;
const THUMB_QUALITY: u8 = 85;
const PREVIEW_QUALITY: u8 = 90;

/// A JPEG up to this size, with a long edge up to [`SMALL_EDGE`], is its own preview.
const SMALL_BYTES: u64 = 3 * 1024 * 1024;
const SMALL_EDGE: u32 = 2560;
/// The same, when the JPEG's dimensions are not known.
const SMALL_UNKNOWN_BYTES: u64 = 3 * 1024 * 1024 / 2;

/// The central store's size is looked at after this many photos have been made.
const EVICT_EVERY: u32 = 16;
/// Trimming goes on until the central store is at this share of its limit.
const EVICT_TO_PERCENT: u128 = 90;

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

/// One of the two tiers the source makes.
struct Spec {
    tier: ImageTier,
    /// `{tier}` in a pattern.
    name: &'static str,
    edge: u32,
    quality: u8,
}

const THUMB: Spec = Spec { tier: ImageTier::Thumb, name: "thumb", edge: THUMB_EDGE, quality: THUMB_QUALITY };
const PREVIEW: Spec = Spec { tier: ImageTier::Preview, name: "preview", edge: PREVIEW_EDGE, quality: PREVIEW_QUALITY };
const SPECS: [&Spec; 2] = [&THUMB, &PREVIEW];

fn spec(tier: ImageTier) -> Option<&'static Spec> {
    SPECS.into_iter().find(|s| s.tier == tier)
}

/// Where `tier` is in [`SPECS`] (and in arrays that follow it).
fn index(tier: ImageTier) -> usize {
    usize::from(tier == ImageTier::Preview)
}

/// The photo's pattern for `spec`'s tier; one that does not parse gives the central one.
fn pattern_for(store: &ImageStore, spec: &Spec) -> Pattern {
    let text = if spec.tier == ImageTier::Thumb { &store.thumb } else { &store.preview };
    Pattern::parse(text).unwrap_or_else(|_| {
        Pattern::parse(pattern::preset(Preset::Central, spec.tier == ImageTier::Thumb)).expect("the preset is valid")
    })
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// When the file at `path` was last written, ms since the epoch; 0 for one that is not there.
fn modified_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

/// A plain JPEG file, as `c1image` reads it.
fn jpeg_ref(path: PathBuf) -> ImageRef {
    ImageRef {
        path,
        format: ImageFormat::Jpeg,
        white_balance: None,
        rotation: 0,
        as_stored: false,
        monochrome: false,
        look: None,
        straighten: 0.0,
        crop: None,
    }
}

/// The original file of `rec`, under the root at `root`. A raw's embedded JPEG often lacks
/// the orientation of the raw's own IFD0, so that is applied as a turn of the pixels as
/// stored; every other file has its EXIF orientation applied by `c1image`.
fn original_ref(rec: &FileRec, root: &str) -> ImageRef {
    let raw = rec.kind == FileKind::Raw;
    ImageRef {
        format: rec.kind.image_format(),
        rotation: if raw { rec.meta.orientation } else { 0 },
        as_stored: raw,
        ..jpeg_ref(rec.path_under(root))
    }
}

/// A JPEG that is small enough to be shown as its own preview.
fn small_jpeg(rec: &FileRec) -> bool {
    if rec.kind != FileKind::Jpeg {
        return false;
    }
    match (rec.meta.exif.width, rec.meta.exif.height) {
        (Some(w), Some(h)) => rec.size <= SMALL_BYTES && w.max(h) <= SMALL_EDGE,
        _ => rec.size <= SMALL_UNKNOWN_BYTES,
    }
}

fn is_current(image: &ImageRec, rec: &FileRec) -> bool {
    image.src_size == rec.size && image.src_mtime_ns == rec.mtime_ns
}

fn other(e: impl std::fmt::Display) -> SourceError {
    SourceError::Other(e.to_string())
}

/// Deletes a file; one that is not there is as good.
fn remove_file(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Removes the folder `path` was in if that left it empty (a `.previews` folder, a hash
/// folder of the central store). Anything in it keeps it.
fn tidy(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
    }
}

/// Writes `bytes` to `path` through a temporary file beside it, so a reader never sees half
/// of an image.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Moves a file, copying it when a rename cannot (another volume).
fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if let Err(e) = std::fs::copy(from, to) {
        let _ = std::fs::remove_file(to);
        return Err(e);
    }
    std::fs::remove_file(from)
}

/// Photo ids being made right now; a second maker of the same one waits for the first.
struct Making {
    busy: Mutex<HashSet<String>>,
    free: Condvar,
}

struct MakingGuard<'a> {
    making: &'a Making,
    ext: String,
}

impl Making {
    fn enter(&self, ext: &str) -> MakingGuard<'_> {
        let mut busy = self.busy.lock().unwrap();
        while busy.contains(ext) {
            busy = self.free.wait(busy).unwrap();
        }
        busy.insert(ext.to_owned());
        MakingGuard { making: self, ext: ext.to_owned() }
    }
}

impl Drop for MakingGuard<'_> {
    fn drop(&mut self) {
        self.making.busy.lock().unwrap().remove(&self.ext);
        self.making.free.notify_all();
    }
}

/// A photo at a new place, for [`Images::relocate`].
pub(crate) struct Moved {
    pub ext: String,
    /// Where it is now: its root's id and its path below it.
    pub root: String,
    pub relpath: String,
}

/// What `make` needs of a photo, copied out of the store so the slow work holds no lock.
struct Job {
    rec: FileRec,
    root: String,
    store: ImageStore,
    data: String,
    /// The recorded image of each of [`SPECS`] and whether it is up to date.
    images: [Option<(ImageRec, bool)>; 2],
}

pub(crate) struct Images {
    /// Photo id → last time (ms) each of its images ([`SPECS`] order; 0 for not yet) was
    /// looked up or made, for the central store's least-recently-used trimming. Kept in
    /// memory only: an image not used since the source was opened counts from its file's
    /// modification time.
    access: Mutex<HashMap<String, [u64; 2]>>,
    /// Every recorded image path, for the scan to leave them out. Built on first use. Taken
    /// after the store's lock, never before.
    generated: Mutex<Option<HashSet<PathBuf>>>,
    /// One maker per photo: two threads making the same photo would write the same files.
    making: Making,
    made: AtomicU32,
    /// When the source was opened: a central file older than that with no record was left by
    /// a crash ([`Images::drop_strays`]).
    opened: SystemTime,
    /// The central store was looked through for such files this session.
    strays_checked: AtomicBool,
}

impl Images {
    pub fn new() -> Self {
        Self {
            access: Mutex::new(HashMap::new()),
            generated: Mutex::new(None),
            making: Making { busy: Mutex::new(HashSet::new()), free: Condvar::new() },
            made: AtomicU32::new(0),
            opened: SystemTime::now(),
            strays_checked: AtomicBool::new(false),
        }
    }

    fn touch(&self, ext: &str, tier: ImageTier) {
        let mut access = self.access.lock().unwrap();
        match access.get_mut(ext) {
            Some(at) => at[index(tier)] = now_ms(),
            None => {
                let mut at = [0; 2];
                at[index(tier)] = now_ms();
                access.insert(ext.to_owned(), at);
            }
        }
    }

    fn generated_update(&self, f: impl FnOnce(&mut HashSet<PathBuf>)) {
        if let Some(set) = self.generated.lock().unwrap().as_mut() {
            f(set);
        }
    }

    fn root_path(src: &FolderSource, root: &str) -> Option<String> {
        src.def.read().unwrap().root(root).map(|r| r.path.clone())
    }

    /// The image of `tier` there is now for photo `ext`, without making anything. Counts
    /// as a use of it for the central store's trimming.
    pub fn lookup(&self, src: &FolderSource, ext: &str, tier: ImageTier) -> Result<Option<ImageRef>> {
        self.find(src, ext, tier, true)
    }

    /// [`Images::lookup`]; a use of the image only when `used` (not for a check of the
    /// whole library, which would make every image just used).
    pub fn find(&self, src: &FolderSource, ext: &str, tier: ImageTier, used: bool) -> Result<Option<ImageRef>> {
        let (made, stand_in) = {
            let state = src.store().read();
            let Some(rec) = state.files.get(ext) else { return Ok(None) };
            let offline = match rec.state {
                FileState::Detached => return Ok(None),
                FileState::Offline => true,
                FileState::Present => false,
            };
            if tier == ImageTier::Original {
                if offline {
                    return Ok(None);
                }
                return Ok(Self::root_path(src, &rec.root).map(|root| original_ref(rec, &root)));
            }
            let Some(spec) = spec(tier) else { return Ok(None) };
            let made = state.images.get(&(ext.to_owned(), tier)).filter(|i| is_current(i, rec)).map(|i| i.path.clone());
            let stand_in = (spec.tier == ImageTier::Preview && !offline && small_jpeg(rec))
                .then(|| Self::root_path(src, &rec.root).map(|root| original_ref(rec, &root)))
                .flatten();
            (made, stand_in)
        };
        // Looked at outside the store's lock: the file may be on a slow drive. A record can
        // outlive its file (deleted by hand, carried off with a folder).
        if let Some(path) = made.filter(|p| p.is_file()) {
            if used {
                self.touch(ext, tier);
            }
            return Ok(Some(jpeg_ref(path)));
        }
        Ok(stand_in)
    }

    /// What `make` needs of photo `ext`, or `None` when there is no file to make from.
    fn job(&self, src: &FolderSource, ext: &str) -> Option<Job> {
        let mut job = {
            let state = src.store().read();
            let rec = state.files.get(ext).filter(|r| r.state == FileState::Present)?;
            let def = src.def.read().unwrap();
            let root = def.root(&rec.root)?.path.clone();
            let images =
                SPECS.map(|s| state.images.get(&(ext.to_owned(), s.tier)).map(|i| (i.clone(), is_current(i, rec))));
            Job {
                rec: rec.clone(),
                root,
                store: def.images.clone(),
                data: src.work_dir().to_string_lossy().into_owned(),
                images,
            }
        };
        // An image whose file is gone is made again.
        for (image, current) in job.images.iter_mut().flatten() {
            *current = *current && image.path.is_file();
        }
        Some(job)
    }

    /// Makes the image of `tier` of photo `ext` if it is not there and up to date, and
    /// returns it. A preview brings the thumbnail along if that is missing (it costs little
    /// from the same decode); a thumbnail comes alone.
    ///
    /// One thread at a time makes a given photo (the others wait, then find it made);
    /// different photos are made in parallel.
    pub fn make(&self, src: &FolderSource, ext: &str, tier: ImageTier) -> Result<Option<ImageRef>> {
        if spec(tier).is_none() {
            return self.lookup(src, ext, tier);
        }
        if let Some(found) = self.lookup(src, ext, tier)? {
            return Ok(Some(found));
        }
        let _making = self.making.enter(ext);
        if let Some(found) = self.lookup(src, ext, tier)? {
            return Ok(Some(found));
        }
        let Some(job) = self.job(src, ext) else { return Ok(None) };
        self.generate(src, ext, &job, tier)?;
        if self.made.fetch_add(1, Ordering::Relaxed) % EVICT_EVERY == EVICT_EVERY - 1 {
            // Making went well; a failed trim waits for the next.
            let _ = self.evict(src);
        }
        self.lookup(src, ext, tier)
    }

    /// Decodes the original once and writes the thumbnail if it is missing or stale, and for
    /// a preview asked for, the preview too (unless a small JPEG is its own).
    fn generate(&self, src: &FolderSource, ext: &str, job: &Job, tier: ImageTier) -> Result<()> {
        let stale = |i: usize| !job.images[i].as_ref().is_some_and(|(_, current)| *current);
        let needed = [stale(0), tier == ImageTier::Preview && stale(1) && !small_jpeg(&job.rec)];
        if !needed.contains(&true) {
            return Ok(());
        }
        // Paths recorded for this photo before: still images when a new one is discarded.
        let recorded: Vec<&Path> = job.images.iter().flatten().map(|(image, _)| image.path.as_path()).collect();
        let paths: [Option<PathBuf>; 2] = std::array::from_fn(|i| {
            let spec = SPECS[i];
            let tokens = Tokens {
                data: &job.data,
                root: &job.root,
                relpath: &job.rec.relpath,
                hash: ext,
                tier: spec.name,
                size: spec.edge,
            };
            needed[i].then(|| pattern_for(&job.store, spec).render(&tokens))
        });
        for path in paths.iter().flatten() {
            self.check_free(src, path, &recorded)?;
        }
        let full = c1image::load(&original_ref(&job.rec, &job.root)).map_err(other)?;
        let (preview, thumb) = if needed[1] {
            let preview = c1image::fit(full, PREVIEW_EDGE);
            let thumb = needed[0].then(|| c1image::fit(preview.clone(), THUMB_EDGE));
            (Some(preview), thumb)
        } else {
            (None, Some(c1image::fit(full, THUMB_EDGE)))
        };
        let mut written: Vec<(ImageTier, ImageRec)> = Vec::new();
        for ((spec, image), path) in SPECS.into_iter().zip([thumb, preview]).zip(paths) {
            let (Some(image), Some(path)) = (image, path) else { continue };
            let bytes = c1image::encode_jpeg(&image, spec.quality);
            // Known as made before it is there: a walk running meanwhile must not take it for a photo.
            self.mark_generated(src, &path);
            if let Err(e) = write_atomic(&path, &bytes) {
                self.discard(&written, &recorded);
                self.unmark_generated(&path, &recorded);
                return Err(e.into());
            }
            let rec =
                ImageRec { path, src_size: job.rec.size, src_mtime_ns: job.rec.mtime_ns, bytes: bytes.len() as u64 };
            written.push((spec.tier, rec));
        }
        // The file may have changed (and been scanned) while the images were made. Images in
        // the central store are always known as made (`is_generated`): a record a crash loses
        // only means the image is made again, so it need not wait for the disk.
        let central = src.work_dir().join("images");
        let central = written.iter().all(|(_, rec)| rec.path.starts_with(&central));
        let put = |w: &mut Writer| {
            let Some(cur) = w.state().files.get(ext).filter(|r| r.state != FileState::Detached) else {
                return Ok(false);
            };
            if cur.size != job.rec.size || cur.mtime_ns != job.rec.mtime_ns {
                return Ok(false);
            }
            for (tier, rec) in &written {
                w.put_image(ext, *tier, Some(rec.clone()))?;
            }
            Ok(true)
        };
        let stored = if central { src.store().write_lazy(put) } else { src.store().write(put) };
        if !matches!(stored, Ok(true)) {
            self.discard(&written, &recorded);
            return stored.map(|_| ());
        }
        // A pattern that changed leaves the image at its old path: take that one away.
        let old: Vec<PathBuf> = written
            .iter()
            .filter_map(|(tier, rec)| {
                let (old, _) = job.images[index(*tier)].as_ref()?;
                (old.path != rec.path).then(|| old.path.clone())
            })
            .collect();
        for path in &old {
            let _ = remove_file(path);
        }
        self.generated_update(|set| {
            for path in &old {
                set.remove(path);
            }
            set.extend(written.iter().map(|(_, rec)| rec.path.clone()));
        });
        for (tier, _) in &written {
            self.touch(ext, *tier);
        }
        Ok(())
    }

    /// Fails when `path` holds a file the source did not make (a photo that a pattern maps
    /// an image onto, say): that is never written over. `recorded` are the photo's own
    /// images.
    fn check_free(&self, src: &FolderSource, path: &Path, recorded: &[&Path]) -> Result<()> {
        if !path.exists() || recorded.contains(&path) || self.is_generated(src, path) {
            return Ok(());
        }
        if src.store().read().images.values().any(|i| i.path == path) {
            return Ok(());
        }
        Err(SourceError::Other(format!(
            "{}: a file the image would go to is there, not made by this source; it is not written over",
            path.display()
        )))
    }

    /// Deletes image files that were written and not recorded, and forgets them as made
    /// unless `recorded` (the photo's images recorded before) still holds the path.
    fn discard(&self, written: &[(ImageTier, ImageRec)], recorded: &[&Path]) {
        for (_, rec) in written {
            let _ = remove_file(&rec.path);
            self.unmark_generated(&rec.path, recorded);
        }
    }

    /// Adds `path` to the images made.
    fn mark_generated(&self, src: &FolderSource, path: &Path) {
        self.load_generated(src);
        self.generated_update(|set| {
            set.insert(path.to_owned());
        });
    }

    /// Takes `path` off the images made, unless one of `recorded` is there.
    fn unmark_generated(&self, path: &Path, recorded: &[&Path]) {
        if !recorded.contains(&path) {
            self.generated_update(|set| {
                set.remove(path);
            });
        }
    }

    /// Fills the list of images made from the store, once: the recorded images and those
    /// still to be deleted. Reads the store, so it must not be called inside a store write.
    fn load_generated(&self, src: &FolderSource) {
        if self.generated.lock().unwrap().is_some() {
            return;
        }
        let state = src.store().read();
        let mut generated = self.generated.lock().unwrap();
        generated.get_or_insert_with(|| {
            let images = state.images.values().map(|i| i.path.clone()).chain(state.doomed.keys().cloned());
            images.chain(state.discarded.keys().cloned()).collect()
        });
    }

    /// A photo's own file is to go to the recycle bin: the walk leaves it out until it has
    /// ([`Images::is_generated`]). Called inside the store write that records it.
    pub(crate) fn discarding(&self, path: &Path) {
        self.generated_update(|set| {
            set.insert(path.to_owned());
        });
    }

    /// A file [`Images::discarding`] named has gone.
    pub(crate) fn discarded(&self, path: &Path) {
        self.generated_update(|set| {
            set.remove(path);
        });
    }

    /// Drops the record of one image of photo `ext`, its file to be deleted by
    /// [`Images::sweep`] (the store keeps the path until then). Called inside a store write.
    fn drop_image(&self, w: &mut Writer, ext: &str, tier: ImageTier, image: &ImageRec) -> Result<()> {
        w.put_image(ext, tier, None)?;
        w.doom(ext, &image.path)
    }

    /// Deletes the files of the images whose records were dropped ([`Images::invalidate`],
    /// [`Images::relocate`]), after the write that dropped them: no file I/O under the
    /// store's lock. Also those a crash left before they were deleted. A path recorded again
    /// meanwhile (made anew) is kept.
    pub fn sweep(&self, src: &FolderSource) {
        let doomed: Vec<(PathBuf, String)> =
            src.store().read().doomed.iter().map(|(path, ext)| (path.clone(), ext.clone())).collect();
        if doomed.is_empty() {
            return;
        }
        let mut gone = Vec::new();
        for (path, ext) in doomed {
            // The photo's maker waits meanwhile, so an image it makes again at the same path
            // is not deleted after it was written.
            let _making = self.making.enter(&ext);
            if !src.store().read().doomed.contains_key(&path) {
                continue;
            }
            // A file that cannot be deleted stays on the list, for the next sweep: the scan
            // must not take it for a photo.
            if remove_file(&path).is_ok() {
                tidy(&path);
                self.generated_update(|set| {
                    set.remove(&path);
                });
                gone.push(path);
            }
        }
        // A crash that loses this only means deleting files that are gone already.
        let _ = src.store().write_lazy(|w| gone.iter().try_for_each(|path| w.spare(path)));
    }

    /// The file of photo `ext` changed or is gone: drops the records of the images made of
    /// it, their files deleted by [`Images::sweep`] after the write. Called inside a store
    /// write: use `w.state()`, never `src.store()`.
    pub fn invalidate(&self, w: &mut Writer, _src: &FolderSource, ext: &str) -> Result<()> {
        for spec in SPECS {
            let Some(image) = w.state().images.get(&(ext.to_owned(), spec.tier)).cloned() else { continue };
            self.drop_image(w, ext, spec.tier, &image)?;
        }
        self.access.lock().unwrap().remove(ext);
        Ok(())
    }

    /// Photos at a new place (moved or renamed, or under a root whose path changed): where
    /// `def`'s patterns follow the photo's place, its images are no longer where they
    /// belong. An image that travelled with the photo (a folder renamed with its `.previews`
    /// in it) is found at its new path, the same size, and kept there; any other has its
    /// record dropped (its file deleted by [`Images::sweep`]), to be made again when asked
    /// for. `landed` holds the new paths where an image was found ([`Images::landed`], read
    /// before the write). Returns the new paths of the images kept. Called inside a store
    /// write.
    pub(crate) fn relocate(
        &self,
        w: &mut Writer,
        src: &FolderSource,
        def: &Definition,
        moves: &[Moved],
        landed: &HashSet<PathBuf>,
    ) -> Result<HashSet<PathBuf>> {
        let mut kept = HashSet::new();
        for (ext, tier, to, image) in self.destinations(w.state(), src, def, moves) {
            if !landed.contains(&to) {
                self.drop_image(w, &ext, tier, &image)?;
                continue;
            }
            // What is left at the old path (in a copy of the root) is no longer this source's.
            self.generated_update(|set| {
                set.remove(&image.path);
                set.insert(to.clone());
            });
            w.put_image(&ext, tier, Some(ImageRec { path: to.clone(), ..image }))?;
            kept.insert(to);
        }
        Ok(kept)
    }

    /// Where the patterns that follow a photo's place put the images of the photos in
    /// `moves`, for each image whose path that changes: (photo, tier, new path, the image as
    /// recorded).
    pub(crate) fn destinations(
        &self,
        state: &State,
        src: &FolderSource,
        def: &Definition,
        moves: &[Moved],
    ) -> Vec<(String, ImageTier, PathBuf, ImageRec)> {
        let mut out = Vec::new();
        let patterns = SPECS.map(|s| pattern_for(&def.images, s));
        if !patterns.iter().any(Pattern::depends_on_location) {
            return out;
        }
        let data = src.work_dir().to_string_lossy().into_owned();
        for Moved { ext, root, relpath } in moves {
            let Some(root) = def.root(root) else { continue };
            for (spec, pattern) in SPECS.into_iter().zip(&patterns) {
                if !pattern.depends_on_location() {
                    continue;
                }
                let Some(image) = state.images.get(&(ext.clone(), spec.tier)) else { continue };
                let tokens =
                    Tokens { data: &data, root: &root.path, relpath, hash: ext, tier: spec.name, size: spec.edge };
                let to = pattern.render(&tokens);
                if to != image.path {
                    out.push((ext.clone(), spec.tier, to, image.clone()));
                }
            }
        }
        out
    }

    /// The new paths of `wanted` ([`Images::destinations`]) where the image already is: it
    /// travelled with its photo (a folder renamed with its `.previews` in it), the same size.
    /// Reads the disk, so it is called before the store's lock is taken.
    pub(crate) fn landed(wanted: Vec<(String, ImageTier, PathBuf, ImageRec)>) -> HashSet<PathBuf> {
        use rayon::prelude::*;
        wanted
            .into_par_iter()
            .filter(|(_, _, to, image)| std::fs::metadata(to).is_ok_and(|m| m.is_file() && m.len() == image.bytes))
            .map(|(_, _, to, _)| to)
            .collect()
    }

    /// `path` is an image this source made (so a walk leaves it out). The first call reads
    /// the store, so it must not be made inside a store write.
    pub fn is_generated(&self, src: &FolderSource, path: &Path) -> bool {
        if path.starts_with(src.work_dir().join("images")) {
            return true;
        }
        self.load_generated(src);
        self.generated.lock().unwrap().as_ref().is_some_and(|set| set.contains(path))
    }

    /// Drops least recently used images until the central store is under its limit. The
    /// first time after the source was opened, also the files no record names.
    pub fn evict(&self, src: &FolderSource) -> Result<()> {
        if !self.strays_checked.swap(true, Ordering::Relaxed) {
            self.drop_strays(src);
        }
        let limit = src.def.read().unwrap().images.limit_bytes();
        match limit {
            Some(limit) => self.evict_to(src, limit).map(|_| ()),
            None => Ok(()),
        }
    }

    /// Deletes the files in the central store's folder that no record names, made before the
    /// source was opened: image records there are written lazily, so a crash can lose the
    /// last few while their files stay, and [`Images::evict_to`] only counts recorded
    /// images. A file made since may still be waiting for its record, so it is left. Returns
    /// how many files went.
    pub(crate) fn drop_strays(&self, src: &FolderSource) -> usize {
        let central = src.work_dir().join("images");
        if !central.is_dir() {
            return 0;
        }
        let recorded: HashSet<PathBuf> = {
            let state = src.store().read();
            let images = state.images.values().map(|i| &i.path);
            images.chain(state.doomed.keys()).filter(|p| p.starts_with(&central)).cloned().collect()
        };
        let mut gone = 0;
        for entry in walkdir::WalkDir::new(&central).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() || recorded.contains(entry.path()) {
                continue;
            }
            let before = entry.metadata().ok().and_then(|m| m.modified().ok()).is_some_and(|t| t < self.opened);
            if before && remove_file(entry.path()).is_ok() {
                tidy(entry.path());
                gone += 1;
            }
        }
        gone
    }

    /// If the images in the central store's folder (`{data}/images`; images kept anywhere
    /// else are never dropped) take more than `limit` bytes, drops some until they take 90%
    /// of it: previews first, least recently used first, then thumbnails if that was not
    /// enough. An image not used since the source was opened counts from its file's
    /// modification time. Returns how many images were dropped.
    pub(crate) fn evict_to(&self, src: &FolderSource, limit: u64) -> Result<usize> {
        let central = src.work_dir().join("images");
        let images: Vec<(String, ImageTier, ImageRec)> = {
            let state = src.store().read();
            let total: u64 = state.images.values().filter(|i| i.path.starts_with(&central)).map(|i| i.bytes).sum();
            if total <= limit {
                return Ok(0);
            }
            state
                .images
                .iter()
                .filter(|(_, i)| i.path.starts_with(&central))
                .map(|((ext, tier), i)| (ext.clone(), *tier, i.clone()))
                .collect()
        };
        let used: Vec<u64> = {
            let access = self.access.lock().unwrap();
            images.iter().map(|(ext, tier, _)| access.get(ext).map_or(0, |at| at[index(*tier)])).collect()
        };
        // An image not used yet counts from its file's time, read once: kept as its use.
        let used: Vec<u64> = images
            .iter()
            .zip(used)
            .map(|((_, _, image), used)| if used > 0 { used } else { modified_ms(&image.path).max(1) })
            .collect();
        {
            let mut access = self.access.lock().unwrap();
            for ((ext, tier, _), &at) in images.iter().zip(&used) {
                let slot = &mut access.entry(ext.clone()).or_default()[index(*tier)];
                if *slot == 0 {
                    *slot = at;
                }
            }
        }
        // (a thumbnail, last used, which): previews sort first.
        let mut order: Vec<(bool, u64, usize)> = images
            .iter()
            .zip(used)
            .enumerate()
            .map(|(i, ((_, tier, _), used))| (*tier == ImageTier::Thumb, used, i))
            .collect();
        order.sort_unstable();
        let mut total: u64 = images.iter().map(|(_, _, i)| i.bytes).sum();
        let target = (u128::from(limit) * EVICT_TO_PERCENT / 100) as u64;
        let mut victims: Vec<&(String, ImageTier, ImageRec)> = Vec::new();
        for (_, _, i) in order {
            if total <= target {
                break;
            }
            total -= images[i].2.bytes;
            victims.push(&images[i]);
        }
        let paths = src.store().write(|w| {
            let mut paths = Vec::new();
            for (ext, tier, image) in &victims {
                // Unless it was made again meanwhile.
                if w.state().images.get(&(ext.clone(), *tier)) == Some(image) {
                    paths.push(image.path.clone());
                    w.put_image(ext, *tier, None)?;
                }
            }
            Ok(paths)
        })?;
        for path in &paths {
            if remove_file(path).is_ok() {
                tidy(path);
            }
        }
        self.generated_update(|set| {
            for path in &paths {
                set.remove(path);
            }
        });
        let mut access = self.access.lock().unwrap();
        for (ext, tier, _) in &victims {
            if let Some(at) = access.get_mut(ext) {
                at[index(*tier)] = 0;
            }
        }
        Ok(paths.len())
    }

    /// Moves the images made under `old` to where `new` puts them, or deletes them.
    ///
    /// Every recorded image is taken as made under `old`. The files are moved first, then
    /// all the records are updated in one write. An image that cannot be moved stays where
    /// it was, with its record, and is listed in the report.
    pub fn migrate(
        &self,
        src: &FolderSource,
        _old: &ImageStore,
        new: &ImageStore,
        how: Migrate,
    ) -> Result<MigrateReport> {
        let plan = self.migration_plan(src, new, how);
        let mut report = MigrateReport::default();
        // (photo, tier, where its record goes: the new path, or nowhere)
        let mut records: Vec<(String, ImageTier, Option<PathBuf>)> = Vec::new();
        let mut gone: Vec<PathBuf> = Vec::new();
        let mut arrived: Vec<PathBuf> = Vec::new();
        // Images about to move away: a path one of them leaves is free once it has.
        let leaving: HashSet<PathBuf> =
            plan.iter().filter(|(_, _, f, t)| t.as_ref() != Some(f)).map(|(_, _, f, _)| f.clone()).collect();
        for (ext, tier, from, to) in plan {
            match to {
                Some(to) if to == from => {}
                // Never over a file that is not one of these images: an original, a file
                // not in the store, another image of this run that has not moved yet.
                Some(to) if from.exists() && to.exists() && !leaving.contains(&to) => {
                    report.failed.push(format!("{}: not moved, a file is already at {}", from.display(), to.display()));
                }
                Some(to) if from.exists() => match move_file(&from, &to) {
                    Ok(()) => {
                        tidy(&from);
                        report.moved += 1;
                        gone.push(from);
                        arrived.push(to.clone());
                        records.push((ext, tier, Some(to)));
                    }
                    Err(e) => report.failed.push(format!("{}: {e}", from.display())),
                },
                _ => match remove_file(&from) {
                    Ok(()) => {
                        tidy(&from);
                        report.removed += 1;
                        gone.push(from);
                        records.push((ext, tier, None));
                    }
                    Err(e) => report.failed.push(format!("{}: {e}", from.display())),
                },
            }
        }
        src.store().write(|w| {
            for (ext, tier, to) in &records {
                match to {
                    Some(to) => {
                        let Some(mut image) = w.state().images.get(&(ext.clone(), *tier)).cloned() else { continue };
                        image.path = to.clone();
                        w.put_image(ext, *tier, Some(image))?;
                    }
                    None => w.put_image(ext, *tier, None)?,
                }
            }
            Ok(())
        })?;
        self.generated_update(|set| {
            for path in &gone {
                set.remove(path);
            }
            set.extend(arrived);
        });
        Ok(report)
    }

    /// Every recorded image with where `how` sends it: the path `new` gives it, or `None`
    /// to delete it (also when its photo or root is gone).
    fn migration_plan(
        &self,
        src: &FolderSource,
        new: &ImageStore,
        how: Migrate,
    ) -> Vec<(String, ImageTier, PathBuf, Option<PathBuf>)> {
        let state = src.store().read();
        let def = src.def.read().unwrap();
        let data = src.work_dir().to_string_lossy().into_owned();
        let patterns = SPECS.map(|s| pattern_for(new, s));
        let mut plan: Vec<_> = state
            .images
            .iter()
            .filter_map(|((ext, tier), image)| {
                let spec_index = SPECS.iter().position(|s| s.tier == *tier)?;
                let to = match how {
                    Migrate::Regenerate => None,
                    Migrate::Move => state
                        .files
                        .get(ext)
                        .filter(|r| r.state != FileState::Detached)
                        .and_then(|rec| Some((rec, def.root(&rec.root)?)))
                        .map(|(rec, root)| {
                            patterns[spec_index].render(&Tokens {
                                data: &data,
                                root: &root.path,
                                relpath: &rec.relpath,
                                hash: ext,
                                tier: SPECS[spec_index].name,
                                size: SPECS[spec_index].edge,
                            })
                        }),
                };
                Some((ext.clone(), *tier, image.path.clone(), to))
            })
            .collect();
        plan.sort_by(|a, b| (&a.0, a.1 == ImageTier::Preview).cmp(&(&b.0, b.1 == ImageTier::Preview)));
        plan
    }

    /// Why `store` cannot hold this source's images, or `None`.
    pub fn check(&self, src: &FolderSource, store: &ImageStore) -> Option<String> {
        let same = store.thumb == store.preview;
        let mut patterns = Vec::new();
        for (spec, text) in [(&THUMB, &store.thumb), (&PREVIEW, &store.preview)] {
            let label = if same {
                String::new()
            } else {
                format!("{} pattern: ", if spec.tier == ImageTier::Thumb { "Thumbnail" } else { "Preview" })
            };
            let pattern = match Pattern::parse(text) {
                Ok(p) => p,
                Err(e) => return Some(format!("{label}{e}")),
            };
            if let Some(problem) = pattern.static_problem() {
                return Some(format!("{label}{problem}"));
            }
            patterns.push(pattern);
        }
        let state = src.store().read();
        let def = src.def.read().unwrap();
        let data = src.work_dir().to_string_lossy().into_owned();
        // Every photo's file, by its path in lower case (Windows and macOS ignore case).
        let photos: HashMap<String, &FileRec> = state
            .files
            .values()
            .filter_map(|rec| Some((rec.path_under(&def.root(&rec.root)?.path).to_string_lossy().to_lowercase(), rec)))
            .collect();
        let mut rendered = Vec::new();
        for (ext, rec) in state.files.iter().filter(|(_, r)| r.state != FileState::Detached) {
            let Some(root) = def.root(&rec.root) else { continue };
            for (spec, pattern) in SPECS.into_iter().zip(&patterns) {
                let tokens = Tokens {
                    data: &data,
                    root: &root.path,
                    relpath: &rec.relpath,
                    hash: ext,
                    tier: spec.name,
                    size: spec.edge,
                };
                let path = pattern.render(&tokens);
                if let Some(photo) = photos.get(&path.to_string_lossy().to_lowercase()) {
                    let what = if spec.tier == ImageTier::Thumb { "thumbnail" } else { "preview" };
                    return Some(format!(
                        "The {what} of {} would be written over the photo {}",
                        rec.name(),
                        photo.path_under(&def.root(&photo.root)?.path).display()
                    ));
                }
                rendered.push((path, format!("{ext}/{}", spec.name)));
            }
        }
        let (path, labels) = pattern::shared_paths(rendered).into_iter().next()?;
        Some(describe_collision(&state.files, &path, &labels))
    }
}

/// "DSCF0001.RAF and DSCF0001.JPG would share /x/DSCF0001.jpg", from the `ext/tier` labels
/// of the images that map to `path`.
fn describe_collision(files: &std::collections::BTreeMap<String, FileRec>, path: &Path, labels: &[String]) -> String {
    let mut exts: Vec<&str> = Vec::new();
    for label in labels {
        let ext = label.rsplit_once('/').map_or(label.as_str(), |(e, _)| e);
        if !exts.contains(&ext) {
            exts.push(ext);
        }
    }
    let file = |ext: &str| files.get(ext).map(|r| r.name().to_owned());
    if let [only] = exts.as_slice() {
        let name = file(only).unwrap_or_else(|| (*only).to_owned());
        return format!("The thumbnail and the preview of {name} would share {}", path.display());
    }
    let names: Vec<String> = exts.iter().filter_map(|e| file(e)).collect();
    let list = match names.as_slice() {
        [a, b] => format!("{a} and {b}"),
        [a, b, rest @ ..] => {
            let more = rest.len();
            if more == 1 { format!("{a}, {b} and {}", rest[0]) } else { format!("{a}, {b} and {more} more") }
        }
        [a] => a.clone(),
        [] => "Some photos".to_owned(),
    };
    format!("{list} would share {}", path.display())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use c1model::{PhotoId, PhotoSource};

    use crate::definition::Definition;
    use crate::store::tests::rec;

    struct Fx {
        dir: TempDir,
        src: FolderSource,
        root: PathBuf,
        root_id: String,
    }

    const A: &str = "00000000000000a1";
    const B: &str = "00000000000000b2";
    const C: &str = "00000000000000c3";

    fn fixture_with(images: ImageStore) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        std::fs::create_dir_all(&root).unwrap();
        let mut def = Definition::new("Test", std::slice::from_ref(&root));
        def.images = images;
        let location = dir.path().join("test.c1folders");
        def.save(&location).unwrap();
        let src = FolderSource::open(&location, &dir.path().join("work")).unwrap();
        Fx { dir, src, root, root_id: def.roots[0].id.clone() }
    }

    fn fixture(preset: Preset) -> Fx {
        fixture_with(ImageStore::preset(preset))
    }

    /// A store of one pattern for both tiers.
    fn custom(pattern: &str) -> ImageStore {
        ImageStore { thumb: pattern.into(), preview: pattern.into(), ..ImageStore::preset(Preset::Custom) }
    }

    fn gradient(w: u32, h: u32) -> image::RgbImage {
        image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 255 / w) as u8, (y * 255 / h) as u8, 128]))
    }

    impl Fx {
        /// Writes an image file of `w` x `h` at `relpath` (its format from the extension).
        fn image(&self, relpath: &str, w: u32, h: u32) {
            let path = crate::store::join_rel(&self.root, relpath);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            gradient(w, h).save(&path).unwrap();
        }

        /// Records the file at `relpath` as photo `ext`, as a scan would.
        fn add(&self, ext: &str, relpath: &str, kind: FileKind, dims: Option<(u32, u32)>) {
            let meta = std::fs::metadata(crate::store::join_rel(&self.root, relpath)).unwrap();
            let mtime = meta.modified().unwrap().duration_since(UNIX_EPOCH).unwrap().as_nanos() as i64;
            let mut r = rec(&self.root_id, relpath);
            r.kind = kind;
            r.size = meta.len();
            r.mtime_ns = mtime;
            r.meta.exif.width = dims.map(|d| d.0);
            r.meta.exif.height = dims.map(|d| d.1);
            self.src.store().write(|w| w.put_file(ext, r)).unwrap();
        }

        fn edit(&self, ext: &str, f: impl FnOnce(&mut FileRec)) {
            let mut r = self.src.store().read().files[ext].clone();
            f(&mut r);
            self.src.store().write(|w| w.put_file(ext, r)).unwrap();
        }

        fn lookup(&self, ext: &str, tier: ImageTier) -> Option<ImageRef> {
            self.src.images.lookup(&self.src, ext, tier).unwrap()
        }

        fn make(&self, ext: &str, tier: ImageTier) -> Option<ImageRef> {
            self.src.images.make(&self.src, ext, tier).unwrap()
        }

        fn recorded(&self, ext: &str, tier: ImageTier) -> Option<ImageRec> {
            self.src.store().read().images.get(&(ext.to_owned(), tier)).cloned()
        }

        /// A 3000 x 2000 JPEG recorded as photo `ext`.
        fn big(&self, ext: &str, relpath: &str) {
            self.image(relpath, 3000, 2000);
            self.add(ext, relpath, FileKind::Jpeg, Some((3000, 2000)));
        }

        /// Both tiers' files of photo `ext`, as recorded.
        fn files(&self, ext: &str) -> Vec<PathBuf> {
            [ImageTier::Thumb, ImageTier::Preview]
                .into_iter()
                .filter_map(|t| self.recorded(ext, t))
                .map(|i| i.path)
                .collect()
        }
    }

    fn dims(path: &Path) -> (u32, u32) {
        image::image_dimensions(path).unwrap()
    }

    #[test]
    fn passes_the_image_conformance_check() {
        let fx = fixture(Preset::Central);
        fx.big(A, "2024/big.jpg");
        let probe = PhotoId::new(&fx.src.definition().source_id(), A);
        assert_eq!(c1model::conformance::check_images(&fx.src, &probe), Vec::<String>::new());
    }

    #[test]
    fn a_check_over_the_library_is_not_a_use() {
        let fx = fixture(Preset::Central);
        fx.big(A, "2024/big.jpg");
        let probe = PhotoId::new(&fx.src.definition().source_id(), A);
        fx.make(A, ImageTier::Thumb);
        fx.src.images.access.lock().unwrap().clear();
        assert!(fx.src.has_image(&probe, ImageTier::Thumb).unwrap());
        assert!(fx.src.images.access.lock().unwrap().is_empty(), "has_image leaves the trimming order");
        assert!(fx.src.image(&probe, ImageTier::Thumb).unwrap().is_some());
        assert!(fx.src.images.access.lock().unwrap().contains_key(A), "image is a use");
    }

    #[test]
    fn nothing_is_looked_up_before_it_is_made_and_a_thumbnail_comes_alone() {
        let fx = fixture(Preset::Central);
        fx.big(A, "2024/big.jpg");
        assert_eq!(fx.lookup(A, ImageTier::Thumb), None);
        assert_eq!(fx.lookup(A, ImageTier::Preview), None);
        let original = fx.lookup(A, ImageTier::Original).unwrap();
        assert_eq!(original.path, fx.root.join("2024").join("big.jpg"));
        assert_eq!((original.format, original.rotation, original.as_stored), (ImageFormat::Jpeg, 0, false));

        let thumb = fx.make(A, ImageTier::Thumb).expect("made");
        assert_eq!(thumb.format, ImageFormat::Jpeg);
        assert!(!thumb.as_stored && thumb.rotation == 0);
        let images = fx.dir.path().join("work").join("images");
        assert_eq!(thumb.path, images.join("thumb").join("00").join(format!("{A}.jpg")));
        assert_eq!(dims(&thumb.path), (450, 300));
        assert_eq!(fx.lookup(A, ImageTier::Preview), None, "no preview with the thumbnail");
        assert!(!images.join("preview").exists());

        let thumb_rec = fx.recorded(A, ImageTier::Thumb).unwrap();
        let preview = fx.make(A, ImageTier::Preview).expect("made");
        assert_eq!(preview.path, images.join("preview").join("00").join(format!("{A}.jpg")));
        assert_eq!(dims(&preview.path), (2000, 1333));
        assert_eq!(fx.recorded(A, ImageTier::Thumb).unwrap(), thumb_rec, "the thumbnail is left as it was");
        assert_eq!(fx.lookup(A, ImageTier::Thumb).unwrap(), thumb);
        for tier in [ImageTier::Thumb, ImageTier::Preview] {
            let image = fx.recorded(A, tier).unwrap();
            let r = fx.src.store().read().files[A].clone();
            assert_eq!((image.src_size, image.src_mtime_ns), (r.size, r.mtime_ns));
            assert_eq!(image.bytes, std::fs::metadata(&image.path).unwrap().len());
        }
        assert_eq!(fx.make(A, ImageTier::Preview).unwrap(), preview, "asking again changes nothing");

        // A preview brings the thumbnail along when that is missing.
        fx.big(B, "other.jpg");
        fx.make(B, ImageTier::Preview).unwrap();
        assert_eq!(dims(&fx.lookup(B, ImageTier::Thumb).unwrap().path), (450, 300));
    }

    #[test]
    fn a_changed_original_makes_its_images_stale() {
        let fx = fixture(Preset::Central);
        fx.big(A, "big.jpg");
        let preview = fx.make(A, ImageTier::Preview).unwrap();
        let thumb = fx.lookup(A, ImageTier::Thumb).unwrap();
        fx.edit(A, |r| r.size += 1);
        assert_eq!(fx.lookup(A, ImageTier::Thumb), None);
        assert_eq!(fx.lookup(A, ImageTier::Preview), None);
        std::fs::remove_file(&thumb.path).unwrap();
        let again = fx.make(A, ImageTier::Thumb).unwrap();
        assert_eq!(again, thumb, "the same place");
        assert!(again.path.is_file(), "written again");
        let size = fx.src.store().read().files[A].size;
        assert_eq!(fx.recorded(A, ImageTier::Thumb).unwrap().src_size, size);
        assert_eq!(fx.lookup(A, ImageTier::Preview), None, "the preview is made when asked for");
        // Written over the stale one, which this source made.
        assert_eq!(fx.make(A, ImageTier::Preview).unwrap(), preview);
        assert_eq!(fx.recorded(A, ImageTier::Preview).unwrap().src_size, size);
        fx.edit(A, |r| r.mtime_ns += 1);
        assert_eq!(fx.lookup(A, ImageTier::Preview), None, "the modification time counts too");
    }

    #[test]
    fn an_image_whose_file_is_gone_is_made_again() {
        let fx = fixture(Preset::Beside);
        fx.big(A, "a.jpg");
        let thumb = fx.make(A, ImageTier::Thumb).unwrap();
        std::fs::remove_file(&thumb.path).unwrap();
        assert_eq!(fx.lookup(A, ImageTier::Thumb), None, "the record alone is not enough");
        assert!(fx.recorded(A, ImageTier::Thumb).is_some());
        let again = fx.make(A, ImageTier::Thumb).unwrap();
        assert_eq!(again, thumb);
        assert!(again.path.is_file());
    }

    #[test]
    fn an_image_is_never_written_over_a_file_the_source_did_not_make() {
        let fx = fixture_with(custom("{dir}/{stem}.{tier}.jpg"));
        fx.big(A, "a.jpg");
        // A photo of its own where A's thumbnail would go.
        fx.image("a.thumb.jpg", 600, 400);
        fx.add(B, "a.thumb.jpg", FileKind::Jpeg, Some((600, 400)));
        let before = std::fs::read(fx.root.join("a.thumb.jpg")).unwrap();
        let err = fx.src.images.make(&fx.src, A, ImageTier::Thumb).unwrap_err();
        assert!(matches!(&err, SourceError::Other(m) if m.contains("a.thumb.jpg")), "{err}");
        assert_eq!(std::fs::read(fx.root.join("a.thumb.jpg")).unwrap(), before);
        assert!(fx.recorded(A, ImageTier::Thumb).is_none());
        let err = fx.src.images.make(&fx.src, A, ImageTier::Preview).unwrap_err();
        assert!(err.to_string().contains("a.thumb.jpg"), "nothing is written when one of them cannot be: {err}");
        assert!(!fx.root.join("a.preview.jpg").exists());
        // The photo's own images are made (and made again over themselves) as usual.
        let b = fx.make(B, ImageTier::Thumb).unwrap();
        assert_eq!(b.path, fx.root.join("a.thumb.thumb.jpg"));
        fx.edit(B, |r| r.size += 1);
        assert_eq!(fx.make(B, ImageTier::Thumb).unwrap(), b);

        // A file under the central store's folder is the source's own.
        let fx = fixture(Preset::Central);
        fx.big(A, "a.jpg");
        let path = fx.src.work_dir().join("images").join("thumb").join("00").join(format!("{A}.jpg"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"left over").unwrap();
        assert_eq!(fx.make(A, ImageTier::Thumb).unwrap().path, path);
        assert_eq!(dims(&path), (450, 300));
    }

    #[test]
    fn a_failed_write_leaves_nothing_on_the_list() {
        let fx = fixture(Preset::Beside);
        fx.big(A, "a.jpg");
        // Where the images should go is a file, so their folder cannot be made.
        std::fs::write(fx.root.join(".previews"), b"in the way").unwrap();
        assert!(fx.src.images.make(&fx.src, A, ImageTier::Preview).is_err());
        let thumb = fx.root.join(".previews").join("a.thumb.jpg");
        assert!(!fx.src.images.is_generated(&fx.src, &thumb));
        assert!(fx.src.store().read().images.is_empty());
    }

    #[test]
    fn a_small_jpeg_is_its_own_preview() {
        let fx = fixture(Preset::Central);
        fx.image("small.jpg", 800, 600);
        fx.add(B, "small.jpg", FileKind::Jpeg, Some((800, 600)));
        let preview = fx.lookup(B, ImageTier::Preview).expect("the original");
        assert_eq!(preview.path, fx.root.join("small.jpg"));
        assert_eq!(fx.lookup(B, ImageTier::Thumb), None);
        let thumb = fx.make(B, ImageTier::Thumb).unwrap();
        assert_eq!(dims(&thumb.path), (450, 338));
        assert!(fx.recorded(B, ImageTier::Preview).is_none(), "no preview is written");
        assert_eq!(fx.make(B, ImageTier::Preview).unwrap().path, fx.root.join("small.jpg"));

        // Too long an edge, too many bytes, unknown dimensions.
        fx.edit(B, |r| r.meta.exif.width = Some(2561));
        assert_eq!(fx.lookup(B, ImageTier::Preview), None);
        fx.edit(B, |r| {
            r.meta.exif.width = Some(2560);
            r.size = 3 * 1024 * 1024 + 1;
        });
        assert_eq!(fx.lookup(B, ImageTier::Preview), None);
        fx.edit(B, |r| {
            r.meta.exif.width = None;
            r.size = 1024 * 1024;
        });
        assert!(fx.lookup(B, ImageTier::Preview).is_some());
        fx.edit(B, |r| r.size = 2 * 1024 * 1024);
        assert_eq!(fx.lookup(B, ImageTier::Preview), None);
        fx.edit(B, |r| {
            r.meta.exif.width = Some(800);
            r.size = 10;
            r.kind = FileKind::Png;
        });
        assert_eq!(fx.lookup(B, ImageTier::Preview), None, "only JPEGs");
    }

    #[test]
    fn png_and_tiff_originals_are_made_into_jpegs() {
        let fx = fixture(Preset::Central);
        fx.image("a.png", 1600, 1200);
        fx.add(A, "a.png", FileKind::Png, Some((1600, 1200)));
        fx.image("b.tif", 1000, 500);
        fx.add(B, "b.tif", FileKind::Tiff, None);
        let original = fx.lookup(A, ImageTier::Original).unwrap();
        assert_eq!((original.format, original.as_stored), (ImageFormat::Png, false));
        assert_eq!(fx.lookup(B, ImageTier::Original).unwrap().format, ImageFormat::Tiff);
        assert_eq!(fx.lookup(A, ImageTier::Preview), None, "a PNG is never its own preview");
        let preview = fx.make(A, ImageTier::Preview).unwrap();
        assert_eq!(dims(&preview.path), (1600, 1200));
        assert_eq!(dims(&fx.lookup(A, ImageTier::Thumb).unwrap().path), (450, 338));
        let preview = fx.make(B, ImageTier::Preview).unwrap();
        assert_eq!(dims(&preview.path), (1000, 500));
        assert_eq!(dims(&fx.lookup(B, ImageTier::Thumb).unwrap().path), (450, 225));
    }

    #[test]
    fn a_raw_is_read_as_stored_with_its_own_orientation() {
        let fx = fixture(Preset::Central);
        std::fs::write(fx.root.join("DSCF0001.RAF"), b"not really a raw").unwrap();
        fx.add(A, "DSCF0001.RAF", FileKind::Raw, None);
        fx.edit(A, |r| r.meta.orientation = 90);
        let original = fx.lookup(A, ImageTier::Original).unwrap();
        assert_eq!(original.path, fx.root.join("DSCF0001.RAF"));
        assert_eq!((original.format, original.rotation, original.as_stored), (ImageFormat::Raw, 90, true));
        assert_eq!(fx.lookup(A, ImageTier::Preview), None);
        // Cannot be decoded: the reader's message comes back.
        let err = fx.src.images.make(&fx.src, A, ImageTier::Thumb).unwrap_err();
        assert!(matches!(&err, SourceError::Other(m) if m.contains("DSCF0001.RAF")), "{err}");
        assert!(fx.recorded(A, ImageTier::Thumb).is_none());
    }

    #[test]
    fn missing_offline_and_detached_photos() {
        let fx = fixture(Preset::Central);
        assert_eq!(fx.lookup(A, ImageTier::Thumb), None);
        assert_eq!(fx.make(A, ImageTier::Thumb), None);
        fx.big(A, "big.jpg");
        let thumb = fx.make(A, ImageTier::Thumb).unwrap();

        fx.edit(A, |r| r.state = FileState::Offline);
        assert_eq!(fx.lookup(A, ImageTier::Original), None);
        assert_eq!(fx.lookup(A, ImageTier::Thumb), Some(thumb), "what was made stays usable");

        fx.edit(A, |r| r.state = FileState::Detached);
        for tier in [ImageTier::Thumb, ImageTier::Preview, ImageTier::Original] {
            assert_eq!(fx.lookup(A, tier), None);
        }

        // Offline and not made yet: nothing to make from.
        fx.big(B, "two.jpg");
        fx.edit(B, |r| r.state = FileState::Offline);
        assert_eq!(fx.make(B, ImageTier::Thumb), None);
        fx.image("small.jpg", 800, 600);
        fx.add(C, "small.jpg", FileKind::Jpeg, Some((800, 600)));
        fx.edit(C, |r| r.state = FileState::Offline);
        assert_eq!(fx.lookup(C, ImageTier::Preview), None, "an offline original cannot stand in");
    }

    #[test]
    fn the_per_root_store_writes_under_the_root() {
        let fx = fixture(Preset::PerRoot);
        fx.big(A, "2024/rome/big.jpg");
        let preview = fx.make(A, ImageTier::Preview).unwrap();
        let thumb = fx.lookup(A, ImageTier::Thumb).unwrap();
        let base = fx.root.join(".c1studio");
        assert_eq!(thumb.path, base.join("thumb").join("2024").join("rome").join("big.jpg.jpg"));
        assert_eq!(preview.path, base.join("preview").join("2024").join("rome").join("big.jpg.jpg"));
        assert!(preview.path.is_file());
        assert!(fx.src.images.is_generated(&fx.src, &thumb.path));
    }

    #[test]
    fn the_beside_store_writes_next_to_the_photo() {
        let fx = fixture(Preset::Beside);
        fx.big(A, "2024/big.jpg");
        fx.make(A, ImageTier::Preview).unwrap();
        let thumb = fx.lookup(A, ImageTier::Thumb).unwrap();
        let dir = fx.root.join("2024").join(".previews");
        assert_eq!(thumb.path, dir.join("big.thumb.jpg"));
        assert!(dir.join("big.preview.jpg").is_file());
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(files.len(), 2, "no temporary file stays: {files:?}");
    }

    #[test]
    fn an_unparsable_pattern_falls_back_to_the_central_one() {
        let mut store = ImageStore::preset(Preset::Custom);
        store.thumb = "{nope}.jpg".into();
        store.preview = String::new();
        let fx = fixture_with(store);
        fx.big(A, "big.jpg");
        let thumb = fx.make(A, ImageTier::Thumb).unwrap();
        assert!(thumb.path.starts_with(fx.dir.path().join("work").join("images")), "{}", thumb.path.display());
    }

    #[test]
    fn a_changed_pattern_moves_the_image_and_removes_the_old_one() {
        let fx = fixture(Preset::Central);
        fx.big(A, "big.jpg");
        let old = fx.make(A, ImageTier::Thumb).unwrap();
        fx.edit(A, |r| r.size += 1);
        fx.src.def.write().unwrap().images = ImageStore::preset(Preset::Beside);
        let new = fx.make(A, ImageTier::Thumb).unwrap();
        assert_eq!(new.path, fx.root.join(".previews").join("big.thumb.jpg"));
        assert!(new.path.is_file() && !old.path.exists());
        assert!(fx.src.images.is_generated(&fx.src, &new.path));
    }

    #[test]
    fn invalidate_deletes_both_images_and_their_records() {
        let fx = fixture(Preset::Beside);
        fx.big(A, "a.jpg");
        fx.big(B, "b.jpg");
        let a_preview = fx.make(A, ImageTier::Preview).unwrap();
        let a = fx.lookup(A, ImageTier::Thumb).unwrap();
        let b = fx.make(B, ImageTier::Thumb).unwrap();
        assert!(fx.src.images.is_generated(&fx.src, &a.path));
        fx.src.store().write(|w| fx.src.images.invalidate(w, &fx.src, A)).unwrap();
        assert!(a.path.exists(), "deleted only once the write is done");
        assert!(fx.src.images.is_generated(&fx.src, &a.path), "until it is deleted");
        fx.src.images.sweep(&fx.src);
        assert!(!a.path.exists() && !a_preview.path.exists());
        assert!(fx.src.store().read().doomed.is_empty());
        assert!(fx.recorded(A, ImageTier::Thumb).is_none() && fx.recorded(A, ImageTier::Preview).is_none());
        assert!(b.path.is_file() && fx.recorded(B, ImageTier::Thumb).is_some());
        assert!(!fx.src.images.is_generated(&fx.src, &a.path), "no longer on the list");
        assert!(fx.src.images.is_generated(&fx.src, &b.path));
        // A photo with no images, or one that is gone, is not an error.
        fx.src.store().write(|w| fx.src.images.invalidate(w, &fx.src, A)).unwrap();
        fx.src.store().write(|w| fx.src.images.invalidate(w, &fx.src, "ffffffffffffffff")).unwrap();
        // The images are made again afterwards.
        assert!(fx.make(A, ImageTier::Thumb).unwrap().path.is_file());
    }

    #[test]
    fn a_failed_write_deletes_no_image() {
        let fx = fixture(Preset::Beside);
        fx.big(A, "a.jpg");
        let a = fx.make(A, ImageTier::Thumb).unwrap();
        let failed: Result<()> = fx.src.store().write(|w| {
            fx.src.images.invalidate(w, &fx.src, A)?;
            Err(SourceError::Other("no".into()))
        });
        assert!(failed.is_err());
        fx.src.images.sweep(&fx.src);
        assert!(a.path.is_file() && fx.recorded(A, ImageTier::Thumb).is_some(), "record and file both kept");
    }

    #[test]
    fn an_image_a_crash_left_undeleted_is_deleted_by_the_next_sweep() {
        let fx = fixture(Preset::Beside);
        fx.big(A, "a.jpg");
        let a = fx.make(A, ImageTier::Thumb).unwrap();
        fx.src.store().write(|w| fx.src.images.invalidate(w, &fx.src, A)).unwrap();
        // The process ends here, before the sweep: what it knew in memory is gone.
        fx.src.store().reload().unwrap();
        let fresh = Images::new();
        assert!(fresh.is_generated(&fx.src, &a.path), "not taken for a photo meanwhile");
        fresh.sweep(&fx.src);
        assert!(!a.path.exists() && fx.src.store().read().doomed.is_empty());
        assert!(!fresh.is_generated(&fx.src, &a.path));
    }

    #[test]
    fn an_image_made_again_before_the_sweep_is_kept() {
        let fx = fixture(Preset::Beside);
        fx.big(A, "a.jpg");
        let a = fx.make(A, ImageTier::Thumb).unwrap();
        fx.src.store().write(|w| fx.src.images.invalidate(w, &fx.src, A)).unwrap();
        assert_eq!(fx.make(A, ImageTier::Thumb).unwrap().path, a.path);
        assert!(fx.src.store().read().doomed.is_empty(), "recorded again, so no longer to be deleted");
        fx.src.images.sweep(&fx.src);
        assert!(a.path.is_file() && fx.recorded(A, ImageTier::Thumb).is_some());
    }

    #[test]
    fn a_sweep_waits_for_the_photos_maker() {
        let fx = fixture(Preset::Beside);
        fx.big(A, "a.jpg");
        let a = fx.make(A, ImageTier::Thumb).unwrap();
        fx.src.store().write(|w| fx.src.images.invalidate(w, &fx.src, A)).unwrap();
        std::thread::scope(|s| {
            // A maker at work on the photo: the sweep has read its list and waits.
            let making = fx.src.images.making.enter(A);
            let sweep = s.spawn(|| fx.src.images.sweep(&fx.src));
            std::thread::sleep(std::time::Duration::from_millis(50));
            assert!(a.path.is_file(), "not deleted under the maker");
            // It makes the image again at the same path, and records it.
            let rec = fx.src.store().read().doomed.keys().next().cloned().unwrap();
            std::fs::write(&rec, b"made again").unwrap();
            let image = ImageRec { path: rec, src_size: 0, src_mtime_ns: 0, bytes: 10 };
            fx.src.store().write(|w| w.put_image(A, ImageTier::Thumb, Some(image))).unwrap();
            drop(making);
            sweep.join().unwrap();
        });
        assert!(a.path.is_file(), "the image made again is kept");
    }

    #[test]
    fn generated_images_are_known_to_the_scan() {
        let fx = fixture(Preset::Beside);
        let images = &fx.src.images;
        let elsewhere = fx.root.join("big.jpg");
        assert!(!images.is_generated(&fx.src, &elsewhere), "also before anything is made (and builds the list)");
        fx.big(A, "big.jpg");
        let thumb = fx.make(A, ImageTier::Thumb).unwrap();
        assert!(images.is_generated(&fx.src, &thumb.path), "added when made");
        assert!(!images.is_generated(&fx.src, &elsewhere));
        let preview = fx.make(A, ImageTier::Preview).unwrap();
        assert!(images.is_generated(&fx.src, &preview.path));
        // Under the central store's folder, whatever the records say.
        assert!(images.is_generated(&fx.src, &fx.src.work_dir().join("images").join("thumb").join("zz").join("x.jpg")));
        assert!(!images.is_generated(&fx.src, &fx.src.work_dir().join("folders.redb")));

        // A list built after the images were made has them too.
        let fresh = Images::new();
        assert!(fresh.is_generated(&fx.src, &thumb.path));
        assert!(!fresh.is_generated(&fx.src, &elsewhere));
    }

    impl Fx {
        /// Sets when photo `ext`'s images were last used, `[thumbnail, preview]` (0: not
        /// since the source was opened).
        fn used(&self, ext: &str, at: [u64; 2]) {
            self.src.images.access.lock().unwrap().insert(ext.into(), at);
        }

        /// What the recorded images (of `tier`, or all) take.
        fn bytes(&self, tier: Option<ImageTier>) -> u64 {
            let state = self.src.store().read();
            state.images.iter().filter(|((_, t), _)| tier.is_none_or(|w| w == *t)).map(|(_, i)| i.bytes).sum()
        }

        /// Three big photos, A, B and C, with both their images made.
        fn three(&self) {
            for (ext, name) in [(A, "a.jpg"), (B, "b.jpg"), (C, "c.jpg")] {
                self.big(ext, name);
                self.make(ext, ImageTier::Preview).unwrap();
            }
        }
    }

    #[test]
    fn eviction_drops_previews_first_least_recently_used_first() {
        let fx = fixture(Preset::Central);
        fx.three();
        let (a, b, c) = (fx.files(A), fx.files(B), fx.files(C));
        // B's preview was looked at longest ago, then C's, then A's; the thumbnails since.
        fx.used(A, [5000, 3000]);
        fx.used(B, [5000, 1000]);
        fx.used(C, [5000, 2000]);
        let total = fx.bytes(None);
        assert_eq!(fx.src.images.evict_to(&fx.src, total).unwrap(), 0, "under the limit");

        // 90% of the limit: one preview's worth goes.
        assert_eq!(fx.src.images.evict_to(&fx.src, total - 1).unwrap(), 1);
        assert!(fx.recorded(B, ImageTier::Preview).is_none() && !b[1].exists(), "its file goes too");
        assert!(fx.recorded(B, ImageTier::Thumb).is_some() && b[0].is_file(), "the thumbnail stays");
        assert!(a.iter().chain(&c).all(|p| p.is_file()));

        // Room for the thumbnails alone: every preview goes, every thumbnail stays.
        let thumbs = fx.bytes(Some(ImageTier::Thumb));
        assert_eq!(fx.src.images.evict_to(&fx.src, thumbs * 10 / 9 + 2).unwrap(), 2);
        assert_eq!(fx.bytes(Some(ImageTier::Preview)), 0);
        assert_eq!(fx.bytes(Some(ImageTier::Thumb)), thumbs);

        // Then thumbnails, least recently used first.
        fx.used(A, [1000, 0]);
        assert_eq!(fx.src.images.evict_to(&fx.src, thumbs - 1).unwrap(), 1);
        assert!(fx.recorded(A, ImageTier::Thumb).is_none() && fx.recorded(C, ImageTier::Thumb).is_some());
        assert_eq!(fx.src.images.evict_to(&fx.src, 1).unwrap(), 2, "everything, as far as it takes");
        assert!(fx.src.store().read().images.is_empty());
    }

    #[test]
    fn after_a_restart_the_files_times_tell_what_was_used_last() {
        let fx = fixture(Preset::Central);
        fx.three();
        fx.src.images.access.lock().unwrap().clear();
        let written = |ext: &str, secs: u64| {
            let path = fx.recorded(ext, ImageTier::Preview).unwrap().path;
            let file = std::fs::File::options().write(true).open(path).unwrap();
            file.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(secs)).unwrap();
        };
        written(B, 500);
        written(A, 1000);
        written(C, 2000);
        // B's preview is the oldest file, but was looked at since the source was opened.
        fx.lookup(B, ImageTier::Preview).unwrap();
        assert_eq!(fx.src.images.evict_to(&fx.src, fx.bytes(None) - 1).unwrap(), 1);
        assert!(fx.recorded(A, ImageTier::Preview).is_none());
        assert_eq!(fx.src.images.evict_to(&fx.src, fx.bytes(None) - 1).unwrap(), 1);
        assert!(fx.recorded(C, ImageTier::Preview).is_none());
        assert!(fx.recorded(B, ImageTier::Preview).is_some());
    }

    #[test]
    fn files_a_crash_left_without_a_record_are_dropped_once() {
        let fx = fixture(Preset::Central);
        fx.three();
        let set_old = |path: &Path| {
            let file = std::fs::File::options().write(true).open(path).unwrap();
            file.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1000)).unwrap();
        };
        // A's preview lost its record in a crash; its file stays, from before the opening.
        let lost = fx.recorded(A, ImageTier::Preview).unwrap().path;
        fx.src.store().write(|w| w.put_image(A, ImageTier::Preview, None)).unwrap();
        set_old(&lost);
        // B's preview is recorded and old: kept. A file made since the opening: kept.
        set_old(&fx.recorded(B, ImageTier::Preview).unwrap().path);
        let fresh = fx.src.work_dir().join("images").join("fresh.jpg");
        std::fs::write(&fresh, b"x").unwrap();

        assert_eq!(fx.src.images.drop_strays(&fx.src), 1);
        assert!(!lost.exists());
        assert!(fresh.is_file());
        assert!(fx.files(B).iter().chain(&fx.files(C)).all(|p| p.is_file()));

        // Trimming does it once a session.
        let again = |path: &Path| {
            std::fs::write(path, b"x").unwrap();
            set_old(path);
        };
        again(&lost);
        fx.src.images.evict(&fx.src).unwrap();
        assert!(!lost.exists(), "the first trim looks");
        again(&lost);
        fx.src.images.evict(&fx.src).unwrap();
        assert!(lost.exists(), "the next ones do not");
    }

    #[test]
    fn only_the_central_store_is_trimmed() {
        let mut store = ImageStore::preset(Preset::PerRoot);
        store.central_limit_mb = Some(0);
        let fx = fixture_with(store);
        fx.big(A, "a.jpg");
        fx.make(A, ImageTier::Thumb).unwrap();
        fx.src.images.evict(&fx.src).unwrap();
        assert!(fx.recorded(A, ImageTier::Thumb).is_some());

        let mut store = ImageStore::preset(Preset::Central);
        store.central_limit_mb = Some(0);
        let fx = fixture_with(store.clone());
        fx.big(A, "a.jpg");
        fx.make(A, ImageTier::Thumb).unwrap();
        fx.src.images.evict(&fx.src).unwrap();
        assert!(fx.recorded(A, ImageTier::Thumb).is_none());

        let fx = fixture(Preset::Central);
        fx.big(A, "a.jpg");
        fx.make(A, ImageTier::Thumb).unwrap();
        fx.src.images.evict(&fx.src).unwrap();
        assert!(fx.recorded(A, ImageTier::Thumb).is_some(), "4 GB is plenty");

        // Images made beside the photos, left there when the store became the central one,
        // are not the central store's to drop.
        let fx = fixture(Preset::Beside);
        fx.big(A, "a.jpg");
        fx.big(B, "b.jpg");
        let beside = fx.make(A, ImageTier::Thumb).unwrap();
        fx.src.def.write().unwrap().images = store;
        fx.make(B, ImageTier::Thumb).unwrap();
        fx.src.images.evict(&fx.src).unwrap();
        assert!(fx.recorded(B, ImageTier::Thumb).is_none());
        assert!(fx.recorded(A, ImageTier::Thumb).is_some() && beside.path.is_file());
    }

    #[test]
    fn making_trims_the_central_store_now_and_then() {
        let mut store = ImageStore::preset(Preset::Central);
        store.central_limit_mb = Some(0);
        let fx = fixture_with(store);
        fx.image("a.jpg", 600, 400);
        for i in 0..EVICT_EVERY {
            let ext = format!("{i:016x}");
            fx.add(&ext, "a.jpg", FileKind::Jpeg, Some((600, 400)));
            fx.make(&ext, ImageTier::Thumb);
        }
        // The sixteenth make looked at the size: a limit of nothing leaves nothing.
        assert!(fx.src.store().read().images.is_empty());
    }

    #[test]
    fn migrating_moves_the_files_and_their_records() {
        let fx = fixture(Preset::Central);
        fx.big(A, "2024/a.jpg");
        fx.big(B, "b.jpg");
        fx.make(A, ImageTier::Preview).unwrap();
        fx.make(B, ImageTier::Preview).unwrap();
        let old_thumb = fx.lookup(A, ImageTier::Thumb).unwrap();
        let old = fx.src.definition().images;
        let new = ImageStore::preset(Preset::Beside);
        let report = fx.src.images.migrate(&fx.src, &old, &new, Migrate::Move).unwrap();
        assert_eq!(report, MigrateReport { moved: 4, removed: 0, failed: vec![] });
        let thumb = fx.root.join("2024").join(".previews").join("a.thumb.jpg");
        assert!(thumb.is_file() && fx.root.join("2024").join(".previews").join("a.preview.jpg").is_file());
        assert!(fx.root.join(".previews").join("b.thumb.jpg").is_file());
        assert!(!old_thumb.path.exists());
        assert_eq!(fx.recorded(A, ImageTier::Thumb).unwrap().path, thumb);
        assert!(fx.src.images.is_generated(&fx.src, &thumb));
        // Nothing needs making: the moved images are current.
        assert_eq!(fx.lookup(A, ImageTier::Thumb).unwrap().path, thumb);
        assert_eq!(dims(&thumb), (450, 300));
        // Moving to where they are is no work.
        let again = fx.src.images.migrate(&fx.src, &new, &new, Migrate::Move).unwrap();
        assert_eq!(again, MigrateReport::default());
    }

    #[test]
    fn migrating_to_regenerate_deletes_the_files_and_records() {
        let fx = fixture(Preset::Central);
        fx.big(A, "a.jpg");
        let preview = fx.make(A, ImageTier::Preview).unwrap();
        let thumb = fx.lookup(A, ImageTier::Thumb).unwrap();
        let old = fx.src.definition().images;
        let new = ImageStore::preset(Preset::PerRoot);
        let report = fx.src.images.migrate(&fx.src, &old, &new, Migrate::Regenerate).unwrap();
        assert_eq!(report, MigrateReport { moved: 0, removed: 2, failed: vec![] });
        assert!(!thumb.path.exists() && !preview.path.exists());
        assert!(fx.src.store().read().images.is_empty());
        assert_eq!(fx.lookup(A, ImageTier::Thumb), None);
    }

    #[test]
    fn migrating_drops_images_of_photos_that_are_gone_and_reports_failures() {
        let fx = fixture(Preset::Central);
        fx.big(A, "a.jpg");
        fx.big(B, "b.jpg");
        fx.make(A, ImageTier::Preview).unwrap();
        fx.make(B, ImageTier::Preview).unwrap();
        let (a, b) = (fx.lookup(A, ImageTier::Thumb).unwrap(), fx.lookup(B, ImageTier::Thumb).unwrap());
        fx.edit(A, |r| r.state = FileState::Detached);
        // Where B's images should go is a file, so their folder cannot be made.
        std::fs::write(fx.root.join(".previews"), b"in the way").unwrap();
        let old = fx.src.definition().images;
        let report = fx.src.images.migrate(&fx.src, &old, &ImageStore::preset(Preset::Beside), Migrate::Move).unwrap();
        assert_eq!(report.removed, 2, "A's two images");
        assert_eq!(report.moved, 0);
        assert_eq!(report.failed.len(), 2, "{:?}", report.failed);
        assert!(!a.path.exists());
        assert!(b.path.is_file(), "left where it was");
        assert_eq!(fx.recorded(B, ImageTier::Thumb).unwrap().path, b.path);
    }

    #[test]
    fn check_accepts_the_presets() {
        let fx = fixture(Preset::Central);
        fx.big(A, "a.jpg");
        fx.big(B, "b.jpg");
        for preset in [Preset::Central, Preset::PerRoot, Preset::Beside] {
            assert_eq!(fx.src.check_image_store(&ImageStore::preset(preset)), None, "{preset:?}");
        }
    }

    #[test]
    fn check_explains_what_is_wrong_with_a_pattern() {
        let fx = fixture(Preset::Central);
        let check = |thumb: &str, preview: &str| {
            let store =
                ImageStore { thumb: thumb.into(), preview: preview.into(), ..ImageStore::preset(Preset::Custom) };
            fx.src.check_image_store(&store)
        };
        let problem = check("{nope}.jpg", "{data}/{tier}/{hash}.jpg").unwrap();
        assert!(problem.starts_with("Thumbnail pattern: ") && problem.contains("{nope}"), "{problem}");
        let problem = check("{data}/{tier}/{hash}.jpg", "{data}/{hash}.png").unwrap();
        assert!(problem.starts_with("Preview pattern: ") && problem.contains(".jpg"), "{problem}");
        let problem = check("{data}/{hash}.jpg", "{data}/{hash}.jpg").unwrap();
        assert!(problem.contains("{tier} or {size}") && !problem.contains("pattern:"), "{problem}");
        let problem = check("{data}/{tier}/x.jpg", "{data}/{tier}/{hash}.jpg").unwrap();
        assert!(problem.starts_with("Thumbnail pattern: ") && problem.contains("share files"), "{problem}");
        assert_eq!(check("{data}/{tier}/{hash}.jpg", "{data}/{size}/{hash}.jpg"), None);
    }

    #[test]
    fn check_names_the_photos_that_would_share_a_file() {
        let fx = fixture(Preset::Central);
        for (ext, name) in [(A, "DSCF1.RAF"), (B, "DSCF1.JPG"), (C, "DSCF2.RAF")] {
            std::fs::write(fx.root.join(name), b"x").unwrap();
            fx.add(ext, name, FileKind::from_ext(name.rsplit('.').next().unwrap()).unwrap(), None);
        }
        // No tier in the name.
        let problem = fx.src.check_image_store(&custom("{dir}/{stem}.jpg")).unwrap();
        assert!(problem.contains("{tier} or {size}"), "{problem}");
        // A tier, but the RAF and the JPG of a shot have one stem.
        let problem = fx.src.check_image_store(&custom("{dir}/{stem}.{tier}.jpg")).unwrap();
        assert!(problem.starts_with("DSCF1.RAF and DSCF1.JPG would share "), "{problem}");
        assert!(problem.ends_with("DSCF1.preview.jpg"), "{problem}");
        // The name keeps them apart.
        assert_eq!(fx.src.check_image_store(&custom("{dir}/{name}.{tier}.jpg")), None);
        // Detached photos do not count.
        fx.edit(B, |r| r.state = FileState::Detached);
        assert_eq!(fx.src.check_image_store(&custom("{dir}/{stem}.{tier}.jpg")), None);
    }

    #[test]
    fn check_counts_the_photos_beyond_the_first_few() {
        let fx = fixture(Preset::Central);
        let names = ["x.RAF", "x.JPG", "x.DNG", "x.NEF", "y.RAF"];
        for (i, name) in names.into_iter().enumerate() {
            std::fs::write(fx.root.join(name), b"x").unwrap();
            fx.add(&format!("{i:016x}"), name, FileKind::from_ext(name.rsplit('.').next().unwrap()).unwrap(), None);
        }
        let problem = fx.src.check_image_store(&custom("{dir}/{stem}.{tier}.jpg")).unwrap();
        assert!(problem.starts_with("x.RAF, x.JPG and 2 more would share "), "{problem}");
        fx.edit(&format!("{:016x}", 3), |r| r.state = FileState::Detached);
        let problem = fx.src.check_image_store(&custom("{dir}/{stem}.{tier}.jpg")).unwrap();
        assert!(problem.starts_with("x.RAF, x.JPG and x.DNG would share "), "{problem}");
    }

    #[test]
    fn check_names_a_photo_an_image_would_be_written_over() {
        let fx = fixture(Preset::Central);
        for (ext, name) in [(A, "x.jpg"), (B, "X.PREVIEW.JPG")] {
            std::fs::write(fx.root.join(name), b"x").unwrap();
            fx.add(ext, name, FileKind::Jpeg, None);
        }
        // Told apart from photos by case alone is not apart at all.
        let problem = fx.src.check_image_store(&custom("{dir}/{stem}.{tier}.jpg")).unwrap();
        assert!(problem.starts_with("The preview of x.jpg would be written over the photo "), "{problem}");
        assert!(problem.ends_with("X.PREVIEW.JPG"), "{problem}");
        assert_eq!(fx.src.check_image_store(&custom("{dir}/.previews/{stem}.{tier}.jpg")), None);
    }

    #[test]
    fn two_threads_making_one_photo_wait_for_each_other() {
        let fx = fixture(Preset::Central);
        fx.big(A, "a.jpg");
        let paths: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| fx.src.images.make(&fx.src, A, ImageTier::Preview).unwrap().unwrap().path))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(paths.windows(2).all(|w| w[0] == w[1]));
        assert!(paths[0].is_file());
        assert_eq!(fx.src.store().read().images.len(), 2);
        assert!(fx.src.images.making.busy.lock().unwrap().is_empty());
    }

    #[test]
    fn different_photos_are_made_in_parallel() {
        let fx = fixture(Preset::Central);
        let exts: Vec<String> = (0..6).map(|i| format!("{i:016x}")).collect();
        for (i, ext) in exts.iter().enumerate() {
            fx.image(&format!("p{i}.jpg"), 2800, 2000);
            fx.add(ext, &format!("p{i}.jpg"), FileKind::Jpeg, Some((2800, 2000)));
        }
        std::thread::scope(|scope| {
            for ext in &exts {
                let fx = &fx;
                scope.spawn(move || fx.make(ext, ImageTier::Thumb).unwrap());
            }
        });
        assert_eq!(fx.src.store().read().images.len(), exts.len(), "thumbnails only");
        for ext in &exts {
            assert!(fx.lookup(ext, ImageTier::Thumb).unwrap().path.is_file());
        }
    }
}
