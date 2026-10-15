//! Thumbnails and previews the source makes and keeps, wherever the definition's image
//! store says.
//!
//! - `image()` ([`Images::lookup`]) only looks: an image made earlier whose original has not
//!   changed since, a small JPEG's original standing in for its preview, the original itself.
//! - `make_image()` ([`Images::make`]) decodes the original once and writes both tiers.
//! - The central store keeps under its size limit by dropping the least recently used.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use c1model::{ImageFormat, ImageRef, ImageTier, Result, SourceError};
use serde::{Deserialize, Serialize};

use crate::FolderSource;
use crate::definition::ImageStore;
use crate::meta::FileKind;
use crate::pattern::{self, Pattern, Preset, Tokens};
use crate::store::{FileRec, FileState, ImageRec, Writer};

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
    /// Photo id → last time (ms) one of its images was looked up, for the central store's
    /// least-recently-used trimming. Kept in memory only.
    access: Mutex<HashMap<String, u64>>,
    /// Every recorded image path, for the scan to leave them out. Built on first use. Taken
    /// after the store's lock, never before.
    generated: Mutex<Option<HashSet<PathBuf>>>,
    /// One maker per photo: two threads making the same photo would write the same files.
    making: Making,
    made: AtomicU32,
}

impl Images {
    pub fn new() -> Self {
        Self {
            access: Mutex::new(HashMap::new()),
            generated: Mutex::new(None),
            making: Making { busy: Mutex::new(HashSet::new()), free: Condvar::new() },
            made: AtomicU32::new(0),
        }
    }

    fn touch(&self, ext: &str) {
        let mut access = self.access.lock().unwrap();
        match access.get_mut(ext) {
            Some(at) => *at = now_ms(),
            None => {
                access.insert(ext.to_owned(), now_ms());
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

    /// The image of `tier` there is now for photo `ext`, without making anything.
    pub fn lookup(&self, src: &FolderSource, ext: &str, tier: ImageTier) -> Result<Option<ImageRef>> {
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
        if let Some(image) = state.images.get(&(ext.to_owned(), tier)).filter(|i| is_current(i, rec)) {
            self.touch(ext);
            return Ok(Some(jpeg_ref(image.path.clone())));
        }
        if spec.tier == ImageTier::Preview && !offline && small_jpeg(rec) {
            return Ok(Self::root_path(src, &rec.root).map(|root| original_ref(rec, &root)));
        }
        Ok(None)
    }

    /// What `make` needs of photo `ext`, or `None` when there is no file to make from.
    fn job(&self, src: &FolderSource, ext: &str) -> Option<Job> {
        let state = src.store().read();
        let rec = state.files.get(ext).filter(|r| r.state == FileState::Present)?;
        let def = src.def.read().unwrap();
        let root = def.root(&rec.root)?.path.clone();
        let images =
            SPECS.map(|s| state.images.get(&(ext.to_owned(), s.tier)).map(|i| (i.clone(), is_current(i, rec))));
        Some(Job {
            rec: rec.clone(),
            root,
            store: def.images.clone(),
            data: src.work_dir().to_string_lossy().into_owned(),
            images,
        })
    }

    /// Makes the thumbnail and the preview of photo `ext` (if they are not there and up to
    /// date) and returns the one of `tier`.
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
        self.generate(src, ext, &job)?;
        if self.made.fetch_add(1, Ordering::Relaxed) % EVICT_EVERY == EVICT_EVERY - 1 {
            // Making went well; a failed trim waits for the next.
            let _ = self.evict(src);
        }
        self.lookup(src, ext, tier)
    }

    /// Decodes the original once and writes the tiers of `job` that are missing or stale.
    fn generate(&self, src: &FolderSource, ext: &str, job: &Job) -> Result<()> {
        let needed: [bool; 2] = std::array::from_fn(|i| {
            let current = job.images[i].as_ref().is_some_and(|(_, current)| *current);
            !current && !(SPECS[i].tier == ImageTier::Preview && small_jpeg(&job.rec))
        });
        if !needed.contains(&true) {
            return Ok(());
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
        for (spec, image) in SPECS.into_iter().zip([thumb, preview]) {
            let Some(image) = image else { continue };
            let bytes = c1image::encode_jpeg(&image, spec.quality);
            let tokens = Tokens {
                data: &job.data,
                root: &job.root,
                relpath: &job.rec.relpath,
                hash: ext,
                tier: spec.name,
                size: spec.edge,
            };
            let path = pattern_for(&job.store, spec).render(&tokens);
            if let Err(e) = write_atomic(&path, &bytes) {
                self.discard(&written);
                return Err(e.into());
            }
            let rec =
                ImageRec { path, src_size: job.rec.size, src_mtime_ns: job.rec.mtime_ns, bytes: bytes.len() as u64 };
            written.push((spec.tier, rec));
        }
        // The file may have changed (and been scanned) while the images were made.
        let recorded = src.store().write(|w| {
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
        });
        if !matches!(recorded, Ok(true)) {
            self.discard(&written);
            return recorded.map(|_| ());
        }
        // A pattern that changed leaves the image at its old path: take that one away.
        let old: Vec<PathBuf> = written
            .iter()
            .filter_map(|(tier, rec)| {
                let (old, _) = job.images[usize::from(*tier == ImageTier::Preview)].as_ref()?;
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
        self.touch(ext);
        Ok(())
    }

    /// Deletes image files that were written and not recorded.
    fn discard(&self, written: &[(ImageTier, ImageRec)]) {
        for (_, rec) in written {
            let _ = remove_file(&rec.path);
        }
    }

    /// The file of photo `ext` changed or is gone: deletes the images made of it and their
    /// records. Called inside a store write: use `w.state()`, never `src.store()`.
    pub fn invalidate(&self, w: &mut Writer, _src: &FolderSource, ext: &str) -> Result<()> {
        for spec in SPECS {
            let Some(image) = w.state().images.get(&(ext.to_owned(), spec.tier)).cloned() else { continue };
            // A file that cannot be deleted stays on the list: the scan must not take it for a photo.
            if remove_file(&image.path).is_ok() {
                self.generated_update(|set| {
                    set.remove(&image.path);
                });
                tidy(&image.path);
            }
            w.put_image(ext, spec.tier, None)?;
        }
        self.access.lock().unwrap().remove(ext);
        Ok(())
    }

    /// `path` is an image this source made (so a walk leaves it out). The first call reads
    /// the store, so it must not be made inside a store write.
    pub fn is_generated(&self, src: &FolderSource, path: &Path) -> bool {
        if path.starts_with(src.work_dir().join("images")) {
            return true;
        }
        if let Some(set) = self.generated.lock().unwrap().as_ref() {
            return set.contains(path);
        }
        let state = src.store().read();
        let mut generated = self.generated.lock().unwrap();
        let set = generated.get_or_insert_with(|| state.images.values().map(|i| i.path.clone()).collect());
        set.contains(path)
    }

    /// Drops least recently used images until the central store is under its limit.
    pub fn evict(&self, src: &FolderSource) -> Result<()> {
        let limit = src.def.read().unwrap().images.limit_bytes();
        match limit {
            Some(limit) => self.evict_to(src, limit).map(|_| ()),
            None => Ok(()),
        }
    }

    /// Drops the images of the photos looked at longest ago (those never looked at first)
    /// if the recorded images take more than `limit` bytes, until they take 90% of it.
    /// Returns how many photos lost theirs.
    pub(crate) fn evict_to(&self, src: &FolderSource, limit: u64) -> Result<usize> {
        let victims: Vec<String> = {
            let state = src.store().read();
            let mut bytes: HashMap<&str, u64> = HashMap::new();
            for ((ext, _), image) in &state.images {
                *bytes.entry(ext).or_default() += image.bytes;
            }
            let mut total: u64 = bytes.values().sum();
            if total <= limit {
                return Ok(0);
            }
            let target = (u128::from(limit) * EVICT_TO_PERCENT / 100) as u64;
            let access = self.access.lock().unwrap();
            let mut order: Vec<(u64, &str, u64)> =
                bytes.into_iter().map(|(ext, b)| (access.get(ext).copied().unwrap_or(0), ext, b)).collect();
            order.sort();
            let mut victims = Vec::new();
            for (_, ext, b) in order {
                if total <= target {
                    break;
                }
                total -= b;
                victims.push(ext.to_owned());
            }
            victims
        };
        let paths = src.store().write(|w| {
            let mut paths = Vec::new();
            for ext in &victims {
                for spec in SPECS {
                    if let Some(image) = w.state().images.get(&(ext.clone(), spec.tier)).cloned() {
                        paths.push(image.path);
                        w.put_image(ext, spec.tier, None)?;
                    }
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
        for ext in &victims {
            access.remove(ext);
        }
        Ok(victims.len())
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
        for (ext, tier, from, to) in plan {
            match to {
                Some(to) if to == from => {}
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
                rendered.push((pattern.render(&tokens), format!("{ext}/{}", spec.name)));
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
    fn nothing_is_looked_up_before_it_is_made_and_make_writes_both_tiers() {
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
        let preview = fx.lookup(A, ImageTier::Preview).expect("made with the thumbnail");
        assert_eq!(preview.path, images.join("preview").join("00").join(format!("{A}.jpg")));
        assert_eq!(dims(&preview.path), (2000, 1333));
        assert_eq!(fx.lookup(A, ImageTier::Thumb).unwrap(), thumb);
        for tier in [ImageTier::Thumb, ImageTier::Preview] {
            let image = fx.recorded(A, tier).unwrap();
            let r = fx.src.store().read().files[A].clone();
            assert_eq!((image.src_size, image.src_mtime_ns), (r.size, r.mtime_ns));
            assert_eq!(image.bytes, std::fs::metadata(&image.path).unwrap().len());
        }
        assert_eq!(fx.make(A, ImageTier::Preview).unwrap(), preview, "asking again changes nothing");
    }

    #[test]
    fn a_changed_original_makes_its_images_stale() {
        let fx = fixture(Preset::Central);
        fx.big(A, "big.jpg");
        let thumb = fx.make(A, ImageTier::Thumb).unwrap();
        fx.edit(A, |r| r.size += 1);
        assert_eq!(fx.lookup(A, ImageTier::Thumb), None);
        assert_eq!(fx.lookup(A, ImageTier::Preview), None);
        std::fs::remove_file(&thumb.path).unwrap();
        let again = fx.make(A, ImageTier::Thumb).unwrap();
        assert_eq!(again, thumb, "the same place");
        assert!(again.path.is_file(), "written again");
        let size = fx.src.store().read().files[A].size;
        assert_eq!(fx.recorded(A, ImageTier::Thumb).unwrap().src_size, size);
        assert_eq!(fx.recorded(A, ImageTier::Preview).unwrap().src_size, size);
        fx.edit(A, |r| r.mtime_ns += 1);
        assert_eq!(fx.lookup(A, ImageTier::Preview), None, "the modification time counts too");
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
        let thumb = fx.make(A, ImageTier::Thumb).unwrap();
        assert_eq!(dims(&thumb.path), (450, 338));
        assert_eq!(dims(&fx.lookup(A, ImageTier::Preview).unwrap().path), (1600, 1200));
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
        let thumb = fx.make(A, ImageTier::Thumb).unwrap();
        let base = fx.root.join(".c1studio");
        assert_eq!(thumb.path, base.join("thumb").join("2024").join("rome").join("big.jpg.jpg"));
        assert!(base.join("preview").join("2024").join("rome").join("big.jpg.jpg").is_file());
        assert!(fx.src.images.is_generated(&fx.src, &thumb.path));
    }

    #[test]
    fn the_beside_store_writes_next_to_the_photo() {
        let fx = fixture(Preset::Beside);
        fx.big(A, "2024/big.jpg");
        let thumb = fx.make(A, ImageTier::Thumb).unwrap();
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
        let a = fx.make(A, ImageTier::Thumb).unwrap();
        let a_preview = fx.lookup(A, ImageTier::Preview).unwrap();
        let b = fx.make(B, ImageTier::Thumb).unwrap();
        assert!(fx.src.images.is_generated(&fx.src, &a.path));
        fx.src.store().write(|w| fx.src.images.invalidate(w, &fx.src, A)).unwrap();
        assert!(!a.path.exists() && !a_preview.path.exists());
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
    fn generated_images_are_known_to_the_scan() {
        let fx = fixture(Preset::Beside);
        let images = &fx.src.images;
        let elsewhere = fx.root.join("big.jpg");
        assert!(!images.is_generated(&fx.src, &elsewhere), "also before anything is made (and builds the list)");
        fx.big(A, "big.jpg");
        let thumb = fx.make(A, ImageTier::Thumb).unwrap();
        assert!(images.is_generated(&fx.src, &thumb.path), "added when made");
        assert!(!images.is_generated(&fx.src, &elsewhere));
        let preview = fx.lookup(A, ImageTier::Preview).unwrap();
        assert!(images.is_generated(&fx.src, &preview.path));
        // Under the central store's folder, whatever the records say.
        assert!(images.is_generated(&fx.src, &fx.src.work_dir().join("images").join("thumb").join("zz").join("x.jpg")));
        assert!(!images.is_generated(&fx.src, &fx.src.work_dir().join("folders.redb")));

        // A list built after the images were made has them too.
        let fresh = Images::new();
        assert!(fresh.is_generated(&fx.src, &thumb.path));
        assert!(!fresh.is_generated(&fx.src, &elsewhere));
    }

    #[test]
    fn eviction_drops_the_least_recently_used_first() {
        let fx = fixture(Preset::Central);
        for (ext, name) in [(A, "a.jpg"), (B, "b.jpg"), (C, "c.jpg")] {
            fx.big(ext, name);
            fx.make(ext, ImageTier::Thumb).unwrap();
        }
        let (a, b, c) = (fx.files(A), fx.files(B), fx.files(C));
        // B was looked up longest ago, then C; A most recently.
        {
            let mut access = fx.src.images.access.lock().unwrap();
            access.insert(A.into(), 3000);
            access.insert(B.into(), 1000);
            access.insert(C.into(), 2000);
        }
        let total: u64 = fx.src.store().read().images.values().map(|i| i.bytes).sum();

        assert_eq!(fx.src.images.evict_to(&fx.src, total).unwrap(), 0, "under the limit");
        assert!(fx.recorded(B, ImageTier::Thumb).is_some());

        // 90% of the limit is two photos' worth of three: one goes.
        assert_eq!(fx.src.images.evict_to(&fx.src, total - 1).unwrap(), 1);
        assert!(fx.recorded(B, ImageTier::Thumb).is_none() && fx.recorded(B, ImageTier::Preview).is_none());
        assert!(b.iter().all(|p| !p.exists()), "files go too");
        assert!(a.iter().chain(&c).all(|p| p.is_file()));

        // Photos never looked up count as the oldest.
        fx.src.images.access.lock().unwrap().remove(C);
        assert_eq!(fx.src.images.evict_to(&fx.src, 1).unwrap(), 2, "everything, as far as it takes");
        assert!(fx.src.store().read().images.is_empty());
    }

    #[test]
    fn unknown_photos_go_before_known_ones() {
        let fx = fixture(Preset::Central);
        for (ext, name) in [(A, "a.jpg"), (B, "b.jpg")] {
            fx.big(ext, name);
            fx.make(ext, ImageTier::Thumb).unwrap();
        }
        fx.src.images.access.lock().unwrap().remove(B);
        let total: u64 = fx.src.store().read().images.values().map(|i| i.bytes).sum();
        assert_eq!(fx.src.images.evict_to(&fx.src, total - 1).unwrap(), 1);
        assert!(fx.recorded(B, ImageTier::Thumb).is_none() && fx.recorded(A, ImageTier::Thumb).is_some());
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
        let fx = fixture_with(store);
        fx.big(A, "a.jpg");
        fx.make(A, ImageTier::Thumb).unwrap();
        fx.src.images.evict(&fx.src).unwrap();
        assert!(fx.recorded(A, ImageTier::Thumb).is_none());

        let fx = fixture(Preset::Central);
        fx.big(A, "a.jpg");
        fx.make(A, ImageTier::Thumb).unwrap();
        fx.src.images.evict(&fx.src).unwrap();
        assert!(fx.recorded(A, ImageTier::Thumb).is_some(), "4 GB is plenty");
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
        let old_thumb = fx.make(A, ImageTier::Thumb).unwrap();
        fx.make(B, ImageTier::Thumb).unwrap();
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
        let thumb = fx.make(A, ImageTier::Thumb).unwrap();
        let preview = fx.lookup(A, ImageTier::Preview).unwrap();
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
        let a = fx.make(A, ImageTier::Thumb).unwrap();
        let b = fx.make(B, ImageTier::Thumb).unwrap();
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
        assert_eq!(fx.src.store().read().images.len(), exts.len() * 2);
        for ext in &exts {
            assert!(fx.lookup(ext, ImageTier::Thumb).unwrap().path.is_file());
        }
    }
}
