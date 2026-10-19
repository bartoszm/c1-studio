//! Thumbnails made here for photos whose source has none (Capture One leaves about 4% of
//! variants without a `.cot`). Made from the source's preview, else from its original,
//! framed as [`c1image::load`] frames them, and kept as JPEGs in a folder of their own,
//! each with a note of what it was made from, so it is made again when that changes. The
//! source's own thumbnail always wins once it has one.
//!
//! Making one decodes a whole preview (50 MB and more), so only [`MAKERS`] are made at a
//! time, by the window's tiles and the run over every photo together, and two asking for
//! the same photo share one making. A photo none could be made for is not tried again
//! until [`Thumbs::forget`].

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::UNIX_EPOCH;

use c1model::{ImageFormat, ImageRef, ImageTier, PhotoId, PhotoSource};
use serde::{Deserialize, Serialize};

use crate::filter::{Filter, Hidden, Trashed};
use crate::library::{Library, QueryOptions, Sort};

/// The long edge, as Capture One's own thumbnails.
const LONG_EDGE: u32 = 450;
const QUALITY: u8 = 85;
/// Thumbnails made at once, by everyone. Each holds a decoded preview in memory.
pub const MAKERS: usize = 3;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Progress {
    /// Photos tried so far, of `total` without a thumbnail.
    pub done: usize,
    pub total: usize,
    /// Thumbnails made.
    pub made: usize,
    /// Photos none could be made for; [`Failure`] says why.
    pub failed: usize,
}

/// Why no thumbnail could be made for a photo, in kinds the user can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// No preview, and the original is not where the catalog says: its drive is not
    /// connected, or it was moved.
    Unreachable,
    /// No preview, and the original is a kind of file this app cannot read (HEIC, PNG,
    /// PSD, a TIFF without a JPEG inside).
    Unsupported,
    /// The preview or the original is there but could not be read: damaged, or making
    /// the thumbnail failed outright.
    Unreadable,
    /// The catalog could not be asked where they are: Capture One busy with it, or
    /// another error. Worth trying again.
    Catalog,
    /// Made, but it could not be saved (the disk full?).
    Unsaved,
}

impl Kind {
    /// The reason as the footer and the log show it.
    pub fn reason(self) -> &'static str {
        match self {
            Kind::Unreachable => "no preview, and the original is not found (drive not connected?)",
            Kind::Unsupported => "no preview, and the original is a kind of file this app cannot read",
            Kind::Unreadable => "the preview or the original could not be read",
            Kind::Catalog => "the catalog could not be read (Capture One busy with it?)",
            Kind::Unsaved => "the thumbnail could not be saved",
        }
    }
}

/// Why making a thumbnail failed: the kind, and what happened ("no preview; original
/// not found").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Why {
    pub kind: Kind,
    pub detail: String,
}

impl std::fmt::Display for Why {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

/// A photo no thumbnail could be made for, and why.
#[derive(Clone, Debug)]
pub struct Failure {
    pub row: u64,
    pub why: Why,
}

/// What a thumbnail is made from: the tier, how to render it, and the file as it was
/// (size and modification time), so a new render of it is noticed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Recipe {
    tier: ImageTier,
    image: ImageRef,
    size: u64,
    modified: u64,
}

impl Recipe {
    /// `None` when the file cannot be looked at: as good as not there.
    fn of(tier: ImageTier, image: &ImageRef) -> Option<Self> {
        let meta = std::fs::metadata(&image.path).ok()?;
        let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64;
        // The source's own thumbnail, which a preview borrows its colours from, wins once
        // it is there, so it does not count here.
        let image = ImageRef { look: None, ..image.clone() };
        Some(Self { tier, image, size: meta.len(), modified })
    }
}

/// The note kept beside a made thumbnail (`<name>.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    from: Recipe,
    /// A preview that was there but could not be read, so the original was used: not
    /// worth trying again until it changes.
    passed_over: Option<Recipe>,
}

/// Whether a thumbnail made as `made` says should be made again, `now` being what one
/// would be made from today. With nothing to make one from now (the drive with the
/// original not connected), or only the original where it was made from the preview,
/// the one there stays.
fn stale(made: Option<&Record>, now: Option<&Recipe>) -> bool {
    let Some(now) = now else { return false };
    let Some(made) = made else {
        // Made before notes were kept.
        return true;
    };
    if made.from.tier == ImageTier::Preview && now.tier == ImageTier::Original {
        return false;
    }
    &made.from != now && made.passed_over.as_ref() != Some(now)
}

/// A thumbnail [`Thumbs::make`] returns; `new` when this call made it, not one made while
/// it waited.
#[derive(Clone, Debug)]
pub struct Made {
    pub image: ImageRef,
    pub new: bool,
}

/// Lets `limit` through at once; the rest wait.
struct Gate {
    free: Mutex<usize>,
    freed: Condvar,
}

/// A place through a [`Gate`], given back when dropped, a panic included.
struct Turn<'a>(&'a Gate);

impl Gate {
    fn new(limit: usize) -> Self {
        Self { free: Mutex::new(limit.max(1)), freed: Condvar::new() }
    }

    fn enter(&self) -> Turn<'_> {
        let mut free = lock(&self.free);
        while *free == 0 {
            free = self.freed.wait(free).unwrap_or_else(|e| e.into_inner());
        }
        *free -= 1;
        Turn(self)
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        *lock(&self.0.free) += 1;
        self.0.freed.notify_one();
    }
}

/// A lock that a panic elsewhere does not spoil: what it guards is always left whole.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Thumbs {
    dir: PathBuf,
    gate: Gate,
    /// One lock per photo being made, so a second request waits for the first.
    making: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Made thumbnails found to match their source this session.
    checked: Mutex<HashSet<String>>,
    /// Photos none could be made for this session, and why.
    failed: Mutex<HashMap<String, Why>>,
}

impl Thumbs {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            gate: Gate::new(MAKERS),
            making: Mutex::default(),
            checked: Mutex::default(),
            failed: Mutex::default(),
        }
    }

    fn path(&self, id: &PhotoId) -> PathBuf {
        let name: String =
            id.ext.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        self.dir.join(format!("{name}.jpg"))
    }

    /// Forgets what this session learnt: which photos failed and which made thumbnails
    /// match their source. After a sync (the catalog may have changed) and before a run
    /// the user asks for again.
    pub fn forget(&self) {
        lock(&self.checked).clear();
        lock(&self.failed).clear();
    }

    /// Why none could be made for `id` this session, if it was tried.
    pub fn failure(&self, id: &PhotoId) -> Option<Why> {
        lock(&self.failed).get(&id.ext).cloned()
    }

    /// One made here earlier, as it is.
    fn made(&self, id: &PhotoId) -> Option<ImageRef> {
        let path = self.path(id);
        path.is_file().then_some(ImageRef {
            path,
            format: ImageFormat::Jpeg,
            white_balance: None,
            rotation: 0,
            as_stored: false,
            monochrome: false,
            look: None,
            straighten: 0.0,
            crop: None,
        })
    }

    fn record(&self, id: &PhotoId) -> Option<Record> {
        serde_json::from_slice(&std::fs::read(self.path(id).with_extension("json")).ok()?).ok()
    }

    /// What a thumbnail would be made from now: the preview, else the original. Looks
    /// them up only, without reading either.
    fn best_now(&self, source: &dyn PhotoSource, id: &PhotoId) -> c1model::source::Result<Option<Recipe>> {
        for tier in [ImageTier::Preview, ImageTier::Original] {
            if let Some(recipe) = source.image(id, tier)?.and_then(|r| Recipe::of(tier, &r)) {
                return Ok(Some(recipe));
            }
        }
        Ok(None)
    }

    /// One made here that still matches what it was made from, or that is the best there
    /// is: making it again failed, or the catalog cannot be asked just now.
    fn current(&self, source: &dyn PhotoSource, id: &PhotoId) -> Option<ImageRef> {
        let made = self.made(id)?;
        if lock(&self.checked).contains(&id.ext) || lock(&self.failed).contains_key(&id.ext) {
            return Some(made);
        }
        match self.best_now(source, id) {
            Ok(now) if stale(self.record(id).as_ref(), now.as_ref()) => None,
            Ok(_) => {
                lock(&self.checked).insert(id.ext.clone());
                Some(made)
            }
            // Asked again next time.
            Err(_) => Some(made),
        }
    }

    /// The source's thumbnail, or else one made here.
    pub fn get(&self, source: &dyn PhotoSource, id: &PhotoId) -> anyhow::Result<Option<ImageRef>> {
        Ok(match source.image(id, ImageTier::Thumb)? {
            Some(r) => Some(r),
            None => self.current(source, id),
        })
    }

    /// [`Thumbs::get`], making the thumbnail first when there is none yet, or the one
    /// there is out of date. A photo that failed this session is not tried again.
    pub fn get_or_make(&self, source: &dyn PhotoSource, id: &PhotoId) -> anyhow::Result<Option<Made>> {
        if let Some(image) = self.get(source, id)? {
            return Ok(Some(Made { image, new: false }));
        }
        if self.failure(id).is_none()
            && let Ok(made) = self.make(source, id)
        {
            return Ok(Some(made));
        }
        // Out of date, but better than nothing.
        Ok(self.made(id).map(|image| Made { image, new: false }))
    }

    /// Makes the thumbnail from the preview, else the original, unless another request
    /// made it while this one waited. The error says why neither served.
    pub fn make(&self, source: &dyn PhotoSource, id: &PhotoId) -> Result<Made, Why> {
        let one = {
            let mut making = lock(&self.making);
            making.entry(id.ext.clone()).or_default().clone()
        };
        let result = {
            let _mine = lock(&one);
            match (self.current(source, id), self.failure(id)) {
                (Some(image), _) => Ok(Made { image, new: false }),
                (None, Some(why)) => Err(why),
                (None, None) => {
                    let _turn = self.gate.enter();
                    let made = catch_unwind(AssertUnwindSafe(|| self.make_now(source, id)))
                        .unwrap_or_else(|panic| Err(Why { kind: Kind::Unreadable, detail: crashed(panic) }));
                    if let Err(why) = &made {
                        lock(&self.checked).remove(&id.ext);
                        lock(&self.failed).insert(id.ext.clone(), why.clone());
                    }
                    made.map(|image| Made { image, new: true })
                }
            }
        };
        // Every handle to the lock is taken and let go under the map's lock, so the last
        // request out finds only the map's and its own.
        let mut making = lock(&self.making);
        if making.get(&id.ext).is_some_and(|m| Arc::ptr_eq(m, &one)) && Arc::strong_count(&one) == 2 {
            making.remove(&id.ext);
        }
        drop(one);
        result
    }

    fn make_now(&self, source: &dyn PhotoSource, id: &PhotoId) -> Result<ImageRef, Why> {
        let catalog = |e: c1model::SourceError| Why { kind: Kind::Catalog, detail: format!("{e}") };
        // A source that renders its own thumbnails (a folder source) makes it itself.
        match source.make_image(id, ImageTier::Thumb) {
            Ok(Some(made)) => return Ok(made),
            Ok(None) => {}
            Err(e) => return Err(Why { kind: Kind::Unreadable, detail: format!("{e}") }),
        }
        let mut notes = Vec::new();
        let mut passed_over = None;
        let mut kind = Kind::Unreachable;
        for tier in [ImageTier::Preview, ImageTier::Original] {
            let name = if tier == ImageTier::Preview { "preview" } else { "original" };
            let Some(recipe) = source.image(id, tier).map_err(catalog)?.and_then(|r| Recipe::of(tier, &r)) else {
                notes.push(if tier == ImageTier::Preview { "no preview".into() } else { "original not found".into() });
                continue;
            };
            match c1image::load_fitted(&recipe.image, LONG_EDGE) {
                Ok(pixels) => {
                    // Taken as checked before it is written, so a tile asking meanwhile
                    // does not look its source up again.
                    lock(&self.checked).insert(id.ext.clone());
                    let record = Record { from: recipe, passed_over };
                    return self.save(id, &pixels, &record).map_err(|e| Why {
                        kind: Kind::Unsaved,
                        detail: format!("saving into {}: {e}", self.dir.display()),
                    });
                }
                Err(e) => {
                    kind = match (&e, tier) {
                        (c1image::Error::NoPreview { .. }, ImageTier::Original) => Kind::Unsupported,
                        _ => Kind::Unreadable,
                    };
                    notes.push(format!("{name} unreadable ({e})"));
                    if tier == ImageTier::Preview {
                        passed_over = Some(recipe);
                    }
                }
            }
        }
        // An unreadable preview stays the trouble when the original is not found.
        Err(Why { kind, detail: notes.join("; ") })
    }

    /// Writes the thumbnail, then the note of what it was made from. A thumbnail newer
    /// than its note looks out of date, and is made again: never the other way round.
    fn save(&self, id: &PhotoId, pixels: &c1image::RgbImage, record: &Record) -> std::io::Result<ImageRef> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path(id);
        write_whole(&path, &c1image::encode_jpeg(pixels, QUALITY))?;
        write_whole(&path.with_extension("json"), &serde_json::to_vec(record).map_err(std::io::Error::other)?)?;
        self.made(id).ok_or_else(|| std::io::Error::other(format!("{} vanished once written", path.display())))
    }

    /// The library's photos that show no thumbnail yet (or one out of date), by row.
    pub fn missing(&self, library: &Library, source: &dyn PhotoSource) -> anyhow::Result<Vec<(u64, PhotoId)>> {
        let all = Filter { trashed: Trashed::Include, hidden: Hidden::Include, ..Filter::default() };
        let rows = library.query(&all, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows;
        let mine = source.info().id;
        let mut out = Vec::new();
        for row in rows {
            let Some(id) = library.photo_id(row) else { continue };
            // `has_image`, not `get`: looking over the whole library is not using the images.
            if id.source == mine && !source.has_image(&id, ImageTier::Thumb)? && self.current(source, &id).is_none() {
                out.push((row, id));
            }
        }
        Ok(out)
    }

    /// Makes the thumbnails `missing` lists, on `threads` threads (at most [`MAKERS`]
    /// make at once, with the window's). `stop` ends it early; `report` hears now and
    /// then. Returns the last progress and the photos that failed.
    pub fn fill(
        &self,
        source: &dyn PhotoSource,
        missing: &[(u64, PhotoId)],
        threads: usize,
        stop: &AtomicBool,
        report: impl Fn(Progress) + Sync,
    ) -> (Progress, Vec<Failure>) {
        let next = AtomicUsize::new(0);
        let progress = Mutex::new(Progress { total: missing.len(), ..Progress::default() });
        let failures = Mutex::new(Vec::new());
        report(*lock(&progress));
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1) {
                scope.spawn(|| {
                    while !stop.load(Ordering::Relaxed) {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((row, id)) = missing.get(i) else { break };
                        // A panic outside the making itself counts as a failure too.
                        let made = catch_unwind(AssertUnwindSafe(|| self.make(source, id)))
                            .unwrap_or_else(|panic| Err(Why { kind: Kind::Unreadable, detail: crashed(panic) }));
                        if let Err(why) = &made {
                            lock(&failures).push(Failure { row: *row, why: why.clone() });
                        }
                        let now = {
                            let mut p = lock(&progress);
                            p.done += 1;
                            if made.is_ok() {
                                p.made += 1
                            } else {
                                p.failed += 1
                            }
                            *p
                        };
                        if now.done % 25 == 0 || now.done == now.total {
                            report(now);
                        }
                    }
                });
            }
        });
        let mut failures = failures.into_inner().unwrap_or_else(|e| e.into_inner());
        failures.sort_by_key(|f| f.row);
        (progress.into_inner().unwrap_or_else(|e| e.into_inner()), failures)
    }
}

/// Writes `bytes` aside and moves them in, so a reader never sees half a file; what is
/// left aside when either fails is removed.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static WRITES: AtomicUsize = AtomicUsize::new(0);
    let ext = path.extension().map_or(String::new(), |e| e.to_string_lossy().into_owned());
    let part =
        path.with_extension(format!("{ext}.{}-{}.part", std::process::id(), WRITES.fetch_add(1, Ordering::Relaxed)));
    let written = std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

/// What a panic said, for the log.
fn crashed(panic: Box<dyn std::any::Any + Send>) -> String {
    let text = panic.downcast_ref::<&str>().copied().or_else(|| panic.downcast_ref::<String>().map(String::as_str));
    format!("making it crashed: {}", text.unwrap_or("no message"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    use c1model::{
        ApplyPlan, ApplyReport, Capabilities, Change, Compatibility, Cursor, SourceError, SourceEvent, SourceId,
        SourceInfo, WriteStatus,
    };

    use super::*;

    /// A source with a preview and an original on disk, as the test sets them; counts how
    /// often each is looked up.
    #[derive(Default)]
    struct Fake {
        preview: Mutex<Option<ImageRef>>,
        original: Mutex<Option<ImageRef>>,
        lookups: AtomicUsize,
        /// Looking up takes this long, so requests overlap.
        slow: Option<Duration>,
        busy: AtomicBool,
        panics: AtomicBool,
    }

    impl PhotoSource for Fake {
        fn info(&self) -> SourceInfo {
            SourceInfo {
                id: SourceId("t".into()),
                name: "fake".into(),
                version: "1".into(),
                compatibility: Compatibility::tested("fake"),
            }
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        fn snapshot(
            &self,
            _: Option<&Cursor>,
            _: &mut dyn FnMut(SourceEvent) -> c1model::source::Result<()>,
        ) -> c1model::source::Result<()> {
            Ok(())
        }
        fn image(&self, _: &PhotoId, tier: ImageTier) -> c1model::source::Result<Option<ImageRef>> {
            if self.panics.load(Ordering::SeqCst) && tier != ImageTier::Thumb {
                panic!("decoder fell over");
            }
            if self.busy.load(Ordering::SeqCst) {
                return Err(SourceError::Other("database is locked".into()));
            }
            match tier {
                ImageTier::Thumb => Ok(None),
                ImageTier::Preview => {
                    self.lookups.fetch_add(1, Ordering::SeqCst);
                    if let Some(slow) = self.slow {
                        std::thread::sleep(slow);
                    }
                    Ok(lock(&self.preview).clone())
                }
                ImageTier::Original => Ok(lock(&self.original).clone()),
            }
        }
        fn write_status(&self) -> WriteStatus {
            WriteStatus::ReadOnly
        }
        fn plan(&self, _: &[Change]) -> c1model::source::Result<ApplyPlan> {
            Ok(ApplyPlan::default())
        }
        fn apply(&self, _: &ApplyPlan) -> c1model::source::Result<ApplyReport> {
            Ok(ApplyReport::default())
        }
    }

    fn jpeg(dir: &Path, name: &str, width: u32, height: u32) -> ImageRef {
        let path = dir.join(name);
        let pixels = c1image::RgbImage::from_pixel(width, height, [200, 100, 50].into());
        std::fs::write(&path, c1image::encode_jpeg(&pixels, 90)).unwrap();
        ImageRef {
            path,
            format: ImageFormat::Jpeg,
            white_balance: None,
            rotation: 0,
            as_stored: true,
            monochrome: false,
            look: None,
            straighten: 0.0,
            crop: None,
        }
    }

    fn id() -> PhotoId {
        PhotoId { source: SourceId("t".into()), ext: "7".into() }
    }

    fn recipe(tier: ImageTier, path: &str, modified: u64) -> Recipe {
        let image = ImageRef { path: path.into(), ..jpeg_ref() };
        Recipe { tier, image, size: 10, modified }
    }

    fn jpeg_ref() -> ImageRef {
        ImageRef {
            path: PathBuf::new(),
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

    #[test]
    fn file_names_keep_to_safe_characters() {
        let thumbs = Thumbs::new("t");
        let id = PhotoId { source: c1model::SourceId("c1:x".into()), ext: "12/34:5".into() };
        assert_eq!(thumbs.path(&id), Path::new("t").join("12_34_5.jpg"));
    }

    #[test]
    fn a_thumbnail_is_made_again_when_what_it_came_from_changes() {
        let preview = recipe(ImageTier::Preview, "p.cop", 1);
        let original = recipe(ImageTier::Original, "o.raf", 1);
        let from = |r: &Recipe| Record { from: r.clone(), passed_over: None };
        // The same source: it stays.
        assert!(!stale(Some(&from(&preview)), Some(&preview)));
        // Rendered again, or cropped or turned since.
        assert!(stale(Some(&from(&preview)), Some(&recipe(ImageTier::Preview, "p.cop", 2))));
        let turned = Recipe { image: ImageRef { rotation: 90, ..preview.image.clone() }, ..preview.clone() };
        assert!(stale(Some(&from(&preview)), Some(&turned)));
        // Made from the original, and now there is a preview.
        assert!(stale(Some(&from(&original)), Some(&preview)));
        // Made from the preview, which is gone: the original would be no better.
        assert!(!stale(Some(&from(&preview)), Some(&original)));
        // Nothing to make one from now: what there is stays.
        assert!(!stale(Some(&from(&original)), None));
        // A preview that could not be read is not tried again until it changes.
        let around = Record { from: original.clone(), passed_over: Some(preview.clone()) };
        assert!(!stale(Some(&around), Some(&preview)));
        assert!(stale(Some(&around), Some(&recipe(ImageTier::Preview, "p.cop", 2))));
        // Made before notes were kept.
        assert!(stale(None, Some(&original)));
    }

    #[test]
    fn made_from_the_original_then_again_once_a_preview_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let thumbs = Thumbs::new(dir.path().join("thumbs"));
        let source = Fake::default();
        *lock(&source.original) = Some(jpeg(dir.path(), "o.jpg", 800, 600));
        let made = thumbs.get_or_make(&source, &id()).unwrap().unwrap();
        assert!(made.new);
        assert_eq!(thumbs.record(&id()).unwrap().from.tier, ImageTier::Original);
        // Asked again: it is there and checked.
        assert!(!thumbs.get_or_make(&source, &id()).unwrap().unwrap().new);
        *lock(&source.preview) = Some(jpeg(dir.path(), "p.jpg", 900, 600));
        // Known good this session, until the session forgets (a sync).
        assert!(!thumbs.get_or_make(&source, &id()).unwrap().unwrap().new);
        thumbs.forget();
        assert!(thumbs.get(&source, &id()).unwrap().is_none());
        assert!(thumbs.get_or_make(&source, &id()).unwrap().unwrap().new);
        assert_eq!(thumbs.record(&id()).unwrap().from.tier, ImageTier::Preview);
        // Nothing is left aside.
        let left: Vec<_> = std::fs::read_dir(dir.path().join("thumbs")).unwrap().flatten().collect();
        assert_eq!(left.len(), 2, "{left:?}");
    }

    #[test]
    fn requests_for_the_same_photo_share_one_making() {
        let dir = tempfile::tempdir().unwrap();
        let thumbs = Thumbs::new(dir.path().join("thumbs"));
        let source = Fake { slow: Some(Duration::from_millis(50)), ..Fake::default() };
        *lock(&source.preview) = Some(jpeg(dir.path(), "p.jpg", 800, 600));
        let new = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..6 {
                scope.spawn(|| {
                    let made = thumbs.get_or_make(&source, &id()).unwrap().unwrap();
                    new.fetch_add(made.new as usize, Ordering::SeqCst);
                });
            }
        });
        assert_eq!(new.into_inner(), 1);
        assert_eq!(source.lookups.load(Ordering::SeqCst), 1);
        assert!(lock(&thumbs.making).is_empty());
    }

    #[test]
    fn failures_are_remembered_until_forgotten_and_say_why() {
        let dir = tempfile::tempdir().unwrap();
        let thumbs = Thumbs::new(dir.path().join("thumbs"));
        let source = Fake::default();
        let why = thumbs.make(&source, &id()).unwrap_err();
        assert_eq!(why.kind, Kind::Unreachable);
        assert_eq!(why.detail, "no preview; original not found");
        // Not looked up again: the tiles retry often.
        assert!(thumbs.get_or_make(&source, &id()).unwrap().is_none());
        assert_eq!(source.lookups.load(Ordering::SeqCst), 1);
        // A file this app cannot read.
        let psd = dir.path().join("a.psd");
        std::fs::write(&psd, b"8BPS not a raw").unwrap();
        *lock(&source.original) = Some(ImageRef { path: psd, format: ImageFormat::Raw, ..jpeg_ref() });
        thumbs.forget();
        assert_eq!(thumbs.make(&source, &id()).unwrap_err().kind, Kind::Unsupported);
        // The catalog busy is not a missing drive.
        source.busy.store(true, Ordering::SeqCst);
        thumbs.forget();
        assert_eq!(thumbs.make(&source, &id()).unwrap_err().kind, Kind::Catalog);
        // Nor does it count as having a thumbnail.
        assert!(thumbs.get(&source, &id()).is_err());
    }

    #[test]
    fn a_making_that_panics_counts_as_failed_and_the_run_ends() {
        let dir = tempfile::tempdir().unwrap();
        let thumbs = Thumbs::new(dir.path().join("thumbs"));
        let source = Fake::default();
        source.panics.store(true, Ordering::SeqCst);
        let photos: Vec<(u64, PhotoId)> =
            (0..5).map(|n| (n, PhotoId { source: SourceId("t".into()), ext: n.to_string() })).collect();
        let (done, failures) = thumbs.fill(&source, &photos, 2, &AtomicBool::new(false), |_| {});
        assert_eq!((done.done, done.made, done.failed), (5, 0, 5));
        assert!(
            failures.iter().all(|f| f.why.kind == Kind::Unreadable && f.why.detail.contains("decoder fell over")),
            "{failures:?}"
        );
        // The gate gave its places back.
        assert_eq!(*lock(&thumbs.gate.free), MAKERS);
    }
}
