//! Walking the roots: new, changed, moved, removed and offline files, into the store.
//!
//! A walk lists every photo file under each root (leaving out the definition's excludes
//! and the images the source made), then compares with the store:
//! - same path, size and modification time (and sidecar): unchanged, not read;
//! - same path, otherwise changed: read again, its images dropped (kept when only its
//!   sidecar changed);
//! - a new path: first matched by content against the records whose path is gone (moved
//!   or renamed), offline or detached, which keeps its id; else a new id. Where the image
//!   store's patterns follow the photo's path, its images go with it (or are made again),
//!   as they do for every photo under a root whose path changed;
//! - a known path not found while its root is reachable: removed;
//! - every file under a root that cannot be reached, or that is empty though it held
//!   photos (a drive not mounted): offline, kept.
//!
//! All of it is written in one store transaction.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Instant, UNIX_EPOCH};

use c1model::{Result, SourceProgress};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use rayon::prelude::*;
use redb::{ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::FolderSource;
use crate::definition::{Definition, RootDef, SidecarNaming};
use crate::ids::{self, ContentKey};
use crate::images::Moved;
use crate::meta::{self, FileKind, FileMeta};
use crate::store::{FileRec, FileState, State, Writer, other};

/// root id → JSON [`RootDef`] as of the last walk, to tell when a root's path or name changed.
const ROOTS: TableDefinition<&str, &str> = TableDefinition::new("scan_roots");

const SCANNING: &str = "Scanning folders";
const READING: &str = "Reading new and changed files";

/// What a walk of the roots found.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanOutcome {
    pub added: usize,
    pub changed: usize,
    /// Kept their id at a new path (moved, renamed, or under a root picked again).
    pub moved: usize,
    /// Gone from the library: files no longer found, and the files of roots taken out of
    /// the definition.
    pub removed: usize,
    /// Roots that could not be reached, by display name; their photos are offline.
    pub offline_roots: Vec<String>,
    /// Files that could not be read (with why), left out or kept as they were.
    pub problems: Vec<String>,
    pub millis: u64,
    /// Files whose metadata was read: none when nothing changed.
    #[serde(default)]
    pub read: usize,
}

/// Walks the roots and brings the store up to date with them.
pub(crate) fn scan(src: &FolderSource) -> Result<ScanOutcome> {
    let busy = src.busy.lock().unwrap();
    let started = Instant::now();
    let result = scan_locked(src, &src.definition());
    src.set_progress(None);
    let mut outcome = result?;
    outcome.millis = started.elapsed().as_millis() as u64;
    *src.last_scan.lock().unwrap() = Some(outcome.clone());
    drop(busy);
    let _ = src.images.evict(src);
    // Files of photos deleted for good that could not go to the recycle bin before.
    let _ = crate::remove::recycle(src);
    Ok(outcome)
}

/// A photo file the walk found.
struct Found {
    root: String,
    relpath: String,
    path: PathBuf,
    kind: FileKind,
    size: u64,
    mtime_ns: i64,
    /// Sidecar file name and its modification time.
    sidecar: Option<(String, i64)>,
}

impl Found {
    fn same_as(&self, rec: &FileRec) -> bool {
        self.size == rec.size
            && self.mtime_ns == rec.mtime_ns
            && self.sidecar.as_ref().map(|(n, _)| n) == rec.sidecar.as_ref()
            && self.sidecar.as_ref().map(|(_, t)| *t) == rec.sidecar_mtime_ns
    }

    fn name(&self) -> &str {
        self.relpath.rsplit('/').next().unwrap_or(&self.relpath)
    }

    /// `rec` describing this file as it is now, keeping its id-bound parts.
    fn onto(&self, mut rec: FileRec) -> FileRec {
        rec.root = self.root.clone();
        rec.relpath = self.relpath.clone();
        rec.kind = self.kind;
        rec.size = self.size;
        rec.mtime_ns = self.mtime_ns;
        rec.sidecar = self.sidecar.as_ref().map(|(n, _)| n.clone());
        rec.sidecar_mtime_ns = self.sidecar.as_ref().map(|(_, t)| *t);
        rec.state = FileState::Present;
        rec
    }
}

/// What the walk of the roots gave.
#[derive(Default)]
struct Walked {
    found: Vec<Found>,
    /// Root id → reachable.
    reachable: HashMap<String, bool>,
    /// (root id, relpath) of folders (or files) that could not be read: what the store has
    /// under them stays as it is. An empty relpath is the whole root.
    unreadable: Vec<(String, String)>,
    problems: Vec<String>,
    /// The definition's excludes, to tell a file now excluded from one that is gone.
    excludes: GlobSet,
}

impl Walked {
    /// The record's file lies under an exclude: not listed, but not gone either.
    fn excluded(&self, rec: &FileRec) -> bool {
        self.excludes.is_match(&rec.relpath)
    }

    fn kept(&self, rec: &FileRec) -> bool {
        self.unreadable.iter().any(|(root, dir)| {
            *root == rec.root
                && (dir.is_empty()
                    || rec.relpath == *dir
                    || rec.relpath.strip_prefix(dir.as_str()).is_some_and(|r| r.starts_with('/')))
        })
    }
}

/// A file at a new path, with its content key.
struct New {
    found: Found,
    key: ContentKey,
    full: Option<String>,
}

/// A record whose file was not found where it was: gone, offline or detached. A new file
/// with its content takes its id.
struct Candidate {
    ext: String,
    rec: FileRec,
}

/// A file at a known path whose size, time or sidecar changed.
struct Changed {
    ext: String,
    rec: FileRec,
    found: Found,
    key: Option<ContentKey>,
    full: Option<String>,
}

/// Everything one walk writes.
#[derive(Default)]
struct Plan {
    /// Records to write: `None` for a new id.
    puts: Vec<(Option<String>, FileRec, bool)>,
    removes: Vec<String>,
    detach: Vec<(String, FileRec)>,
    offline: Vec<(String, FileRec)>,
    /// Whole-file hashes worked out for known records, by id.
    fulls: Vec<(String, String)>,
    outcome: ScanOutcome,
}

/// What was found, sorted against the store.
struct Sorted {
    /// Ids of the records found where they were.
    seen: HashSet<String>,
    changed: Vec<Changed>,
    news: Vec<Found>,
    candidates: Vec<Candidate>,
}

fn scan_locked(src: &FolderSource, def: &Definition) -> Result<ScanOutcome> {
    let mut walked = walk(src, def);
    let mut plan = Plan::default();
    plan.outcome.offline_roots =
        def.roots.iter().filter(|r| !walked.reachable[&r.id]).map(RootDef::display_name).collect();
    let Sorted { seen, mut changed, news, candidates } = sort_out(src, &mut walked, &mut plan);

    // Content keys of the files at new paths and of the changed ones.
    let progress = Progress::new(src, READING, "Content keys", news.len() + changed.len());
    let mut news: Vec<New> = news
        .into_par_iter()
        .filter_map(|found| {
            let key = ids::content_key(&found.path);
            progress.step();
            match key {
                Ok(key) => Some(New { found, key, full: None }),
                Err(e) => {
                    progress.problem(format!("{}: {e}", found.path.display()));
                    None
                }
            }
        })
        .collect();
    changed.par_iter_mut().for_each(|c| {
        match ids::content_key(&c.found.path) {
            Ok(key) => c.key = Some(key),
            Err(e) => progress.problem(format!("{}: {e}", c.found.path.display())),
        }
        progress.step();
    });
    walked.problems.extend(progress.problems());
    // A changed file that could not be read is kept as it was.
    changed.retain(|c| c.key.is_some());

    let mut pairs = match_moves(&news, &candidates);
    let full_needed = shared_keys(src, def, &news, &changed, &candidates, &seen, &pairs);
    plan.fulls = hash_whole(src, &mut news, &mut changed, &full_needed, &mut walked.problems);
    pairs.extend(match_ambiguous(&news, &candidates, &pairs));
    let reads = settle(&mut plan, changed, news, &candidates, &pairs, &walked);

    // Metadata, for new files and changed ones.
    let progress = Progress::new(src, READING, "Metadata", reads.len());
    let reads_done = AtomicUsize::new(0);
    let read: Vec<(Option<String>, FileRec, bool)> = reads
        .into_par_iter()
        .map(|ToRead { found, rec, ext, invalidate }| {
            let meta = read_meta(&found);
            reads_done.fetch_add(1, Ordering::Relaxed);
            progress.step();
            (ext, FileRec { meta, ..found.onto(rec) }, invalidate)
        })
        .collect();
    plan.outcome.read = reads_done.into_inner();
    plan.puts.extend(read);
    plan.outcome.problems = walked.problems;

    write(src, def, plan)
}

/// Sorts what the walk found against the store, cheaply, under the read guard: records
/// found unchanged need nothing (or only to be shown again), the rest is sorted into
/// changed files, files at new paths, and records whose file was not found where it was.
fn sort_out(src: &FolderSource, walked: &mut Walked, plan: &mut Plan) -> Sorted {
    let mut sorted = Sorted { seen: HashSet::new(), changed: Vec::new(), news: Vec::new(), candidates: Vec::new() };
    let state = src.store().read();
    for found in std::mem::take(&mut walked.found) {
        let Some(ext) = state.at_path(&found.root, &found.relpath) else {
            sorted.news.push(found);
            continue;
        };
        sorted.seen.insert(ext.to_owned());
        let rec = &state.files[ext];
        if !found.same_as(rec) {
            sorted.changed.push(Changed { ext: ext.to_owned(), rec: rec.clone(), found, key: None, full: None });
        } else if rec.state != FileState::Present {
            plan.puts.push((Some(ext.to_owned()), found.onto(rec.clone()), false));
        }
    }
    for (ext, rec) in state.files.iter().filter(|(ext, _)| !sorted.seen.contains(*ext)) {
        match walked.reachable.get(&rec.root) {
            None => {
                if rec.state != FileState::Detached {
                    plan.detach.push((ext.clone(), FileRec { state: FileState::Detached, ..rec.clone() }));
                }
            }
            Some(false) => {
                // Detached too: its root is back in the definition, though out of reach.
                if rec.state != FileState::Offline {
                    plan.offline.push((ext.clone(), FileRec { state: FileState::Offline, ..rec.clone() }));
                }
            }
            Some(true) if walked.kept(rec) => continue,
            // Now excluded: detached like a root taken out, so its edits come back with it.
            Some(true) if walked.excluded(rec) => {
                if rec.state != FileState::Detached {
                    plan.detach.push((ext.clone(), FileRec { state: FileState::Detached, ..rec.clone() }));
                }
            }
            Some(true) => {}
        }
        sorted.candidates.push(Candidate { ext: ext.clone(), rec: rec.clone() });
    }
    sorted
}

/// A file whose metadata is to be read.
struct ToRead {
    found: Found,
    /// Its record so far: what is kept of the one before, or a new one.
    rec: FileRec,
    /// Its id; `None` for a new one.
    ext: Option<String>,
    /// Its pixels changed: the images made of it go.
    invalidate: bool,
}

/// The file's content is not what the record says: its size, time or content key differ
/// (not only its sidecar).
fn pixels_changed(found: &Found, key: &ContentKey, rec: &FileRec) -> bool {
    found.size != rec.size || found.mtime_ns != rec.mtime_ns || *key != rec.content
}

/// Puts the changed files, the new ones and the moves into the plan, with what is removed
/// and counted. Returns the files whose metadata is to be read.
fn settle(
    plan: &mut Plan,
    changed: Vec<Changed>,
    news: Vec<New>,
    candidates: &[Candidate],
    pairs: &[(usize, usize)],
    walked: &Walked,
) -> Vec<ToRead> {
    let taken: HashMap<usize, usize> = pairs.iter().copied().collect();
    let mut reads: Vec<ToRead> = Vec::new();
    for c in changed {
        let key = c.key.expect("kept only with a key");
        // Only its sidecar changed: what the file says is read again, its images stay.
        let invalidate = pixels_changed(&c.found, &key, &c.rec);
        let full = if key == c.rec.content { c.full.or(c.rec.full.clone()) } else { c.full };
        let rec = FileRec { content: key, full, ..c.rec };
        plan.outcome.changed += 1;
        reads.push(ToRead { found: c.found, rec, ext: Some(c.ext), invalidate });
    }
    let mut matched: HashSet<&str> = HashSet::new();
    for (n, new) in news.into_iter().enumerate() {
        let Some(&c) = taken.get(&n) else {
            let rec = FileRec {
                root: String::new(),
                relpath: String::new(),
                kind: new.found.kind,
                size: 0,
                mtime_ns: 0,
                sidecar: None,
                sidecar_mtime_ns: None,
                content: new.key,
                full: new.full,
                meta: FileMeta::default(),
                state: FileState::Present,
                imported_at: 0,
            };
            plan.outcome.added += 1;
            reads.push(ToRead { found: new.found, rec, ext: None, invalidate: false });
            continue;
        };
        let cand = &candidates[c];
        matched.insert(&cand.ext);
        let mut rec = cand.rec.clone();
        rec.full = new.full.or(rec.full);
        plan.outcome.moved += 1;
        // Images that follow the photo's path are seen to when the plan is written.
        if new.found.same_as(&cand.rec) {
            plan.puts.push((Some(cand.ext.clone()), new.found.onto(rec), false));
        } else {
            let invalidate = pixels_changed(&new.found, &new.key, &cand.rec);
            reads.push(ToRead { found: new.found, rec, ext: Some(cand.ext.clone()), invalidate });
        }
    }
    plan.detach.retain(|(ext, _)| !matched.contains(ext.as_str()));
    plan.offline.retain(|(ext, _)| !matched.contains(ext.as_str()));
    for cand in candidates {
        if !matched.contains(cand.ext.as_str())
            && walked.reachable.get(&cand.rec.root) == Some(&true)
            && !walked.excluded(&cand.rec)
        {
            plan.removes.push(cand.ext.clone());
        }
    }
    plan.outcome.removed = plan.removes.len() + plan.detach.len();
    reads
}

/// Lists the photo files under every reachable root.
fn walk(src: &FolderSource, def: &Definition) -> Walked {
    let mut walked = Walked::default();
    let excludes = excludes(def, &mut walked.problems);
    walked.excludes = excludes.clone();
    // Folder → its XMP files, by lower-case name.
    let mut xmps: HashMap<PathBuf, HashMap<String, String>> = HashMap::new();
    let mut listed: Vec<(String, String, PathBuf, FileKind)> = Vec::new();
    for root in &def.roots {
        let base = Path::new(&root.path);
        let reachable = base.is_dir();
        walked.reachable.insert(root.id.clone(), reachable);
        if !reachable {
            continue;
        }
        let before = listed.len();
        let note = Some(root.display_name());
        let progress =
            SourceProgress { phase: SCANNING.into(), done: listed.len() as u64, total: None, note: note.clone() };
        src.set_progress(Some(progress));
        let entries = walkdir::WalkDir::new(base)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| e.depth() == 0 || !excludes.is_match(relpath(base, e.path())));
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    let at = e.path().map(|p| relpath(base, p)).unwrap_or_default();
                    walked.problems.push(format!("{}: {e}", e.path().unwrap_or(base).display()));
                    walked.unreadable.push((root.id.clone(), at));
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
            if ext.eq_ignore_ascii_case("xmp") {
                let name = entry.file_name().to_string_lossy().into_owned();
                let dir = path.parent().unwrap_or(base).to_owned();
                xmps.entry(dir).or_default().insert(name.to_lowercase(), name);
                continue;
            }
            let Some(kind) = FileKind::from_ext(ext) else { continue };
            if src.images.is_generated(src, path) {
                continue;
            }
            listed.push((root.id.clone(), relpath(base, path), entry.into_path(), kind));
            if listed.len().is_multiple_of(256) {
                let progress = SourceProgress {
                    phase: SCANNING.into(),
                    done: listed.len() as u64,
                    total: None,
                    note: note.clone(),
                };
                src.set_progress(Some(progress));
            }
        }
        // An empty root that held photos is most likely a drive not mounted (an empty mount
        // point): its photos are kept, offline, rather than deleted.
        if listed.len() == before && known_under(src, &root.id) {
            walked.reachable.insert(root.id.clone(), false);
            walked.problems.push(format!("{}: no photos found; taken as not mounted", root.path));
        }
    }

    let none = HashMap::new();
    let stats: Vec<std::result::Result<Found, (String, String, String)>> = listed
        .into_par_iter()
        .map(|(root, relpath, path, kind)| {
            let meta = std::fs::metadata(&path)
                .map_err(|e| (root.clone(), relpath.clone(), format!("{}: {e}", path.display())))?;
            let dir = path.parent().and_then(|d| xmps.get(d)).unwrap_or(&none);
            let sidecar = sidecar(&path, dir, def.sidecar);
            Ok(Found { root, relpath, kind, size: meta.len(), mtime_ns: mtime_ns(&meta), sidecar, path })
        })
        .collect();
    for stat in stats {
        match stat {
            Ok(found) => walked.found.push(found),
            // A file that cannot be read now is not a file that is gone: its record stays.
            Err((root, relpath, problem)) => {
                walked.problems.push(problem);
                walked.unreadable.push((root, relpath));
            }
        }
    }
    walked
}

/// The store has photos under root `id`, shown or detached (a root taken out and added
/// back): an empty mount point must not delete either.
fn known_under(src: &FolderSource, id: &str) -> bool {
    src.store().read().files.values().any(|r| r.root == id)
}

/// The excludes as one set. A pattern for everything in a folder (`**/tmp/**`) also
/// matches the folder itself, so it is not entered.
fn excludes(def: &Definition, problems: &mut Vec<String>) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for pattern in def.all_excludes() {
        let pattern = pattern.trim_end_matches('/');
        let folder = pattern.strip_suffix("/**").filter(|f| !f.is_empty());
        for p in std::iter::once(pattern).chain(folder) {
            match GlobBuilder::new(p).case_insensitive(cfg!(windows)).build() {
                Ok(glob) => {
                    builder.add(glob);
                }
                Err(e) => problems.push(format!("exclude {p}: {e}")),
            }
        }
    }
    builder.build().unwrap_or_else(|e| {
        problems.push(format!("excludes: {e}"));
        GlobSet::empty()
    })
}

/// `path` below `base`, `/`-separated.
fn relpath(base: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    let parts: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect();
    parts.join("/")
}

/// A file's modification time, nanoseconds since the epoch, as [`FileRec::mtime_ns`] holds it.
pub(crate) fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    match meta.modified().map(|t| t.duration_since(UNIX_EPOCH)) {
        Ok(Ok(d)) => d.as_nanos() as i64,
        Ok(Err(before)) => -(before.duration().as_nanos() as i64),
        Err(_) => 0,
    }
}

/// The XMP sidecar of `photo` among its folder's XMP files (`xmps`, by lower-case name),
/// the definition's naming first, with its modification time.
fn sidecar(photo: &Path, xmps: &HashMap<String, String>, naming: SidecarNaming) -> Option<(String, i64)> {
    if xmps.is_empty() {
        return None;
    }
    let other = match naming {
        SidecarNaming::Stem => SidecarNaming::Name,
        SidecarNaming::Name => SidecarNaming::Stem,
    };
    [naming, other].into_iter().find_map(|naming| {
        let wanted = naming.sidecar(photo);
        let name = xmps.get(&wanted.file_name()?.to_string_lossy().to_lowercase())?;
        let meta = std::fs::metadata(photo.with_file_name(name)).ok()?;
        Some((name.clone(), mtime_ns(&meta)))
    })
}

fn read_meta(found: &Found) -> FileMeta {
    let sidecar = found.sidecar.as_ref().map(|(name, _)| found.path.with_file_name(name));
    meta::read(&found.path, found.kind, sidecar.as_deref())
}

/// Pairs new files with candidates (index into `news`, index into `candidates`) where the
/// content key alone settles it: the same path below another root (a root picked again),
/// or one new file and one candidate left with a key.
fn match_moves(news: &[New], candidates: &[Candidate]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for (news_of, cands_of) in groups(news, candidates, &[]) {
        let mut free: Vec<usize> = cands_of;
        let mut left = Vec::new();
        for n in news_of {
            match free.iter().position(|&c| candidates[c].rec.relpath == news[n].found.relpath) {
                Some(at) => pairs.push((n, free.remove(at))),
                None => left.push(n),
            }
        }
        if let ([n], [c]) = (left.as_slice(), free.as_slice()) {
            pairs.push((*n, *c));
        }
    }
    pairs
}

/// Pairs the new files [`match_moves`] left where several share a key, by whole-file
/// hash, then the same file name, then the same path.
fn match_ambiguous(news: &[New], candidates: &[Candidate], done: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for (mut news_of, mut free) in groups(news, candidates, done) {
        news_of.sort_by(|a, b| news[*a].found.relpath.cmp(&news[*b].found.relpath));
        for n in news_of {
            let new = &news[n];
            let best = free
                .iter()
                .enumerate()
                .filter(|(_, c)| candidates[**c].rec.full.as_ref().is_none_or(|f| Some(f) == new.full.as_ref()))
                .min_by_key(|(_, c)| {
                    let rec = &candidates[**c].rec;
                    Reverse((rec.full.is_some(), rec.name() == new.found.name(), rec.relpath == new.found.relpath))
                })
                .map(|(at, _)| at);
            if let Some(at) = best {
                pairs.push((n, free.remove(at)));
            }
        }
    }
    pairs
}

/// The new files and candidates not in `done`, grouped by content key, for the keys both
/// have; in the order of the new files.
fn groups(news: &[New], candidates: &[Candidate], done: &[(usize, usize)]) -> Vec<(Vec<usize>, Vec<usize>)> {
    let (taken_n, taken_c): (HashSet<usize>, HashSet<usize>) = done.iter().copied().unzip();
    let mut cands: HashMap<&ContentKey, Vec<usize>> = HashMap::new();
    for (c, cand) in candidates.iter().enumerate().filter(|(c, _)| !taken_c.contains(c)) {
        cands.entry(&cand.rec.content).or_default().push(c);
    }
    let mut order: Vec<&ContentKey> = Vec::new();
    let mut by_key: HashMap<&ContentKey, Vec<usize>> = HashMap::new();
    for (n, new) in news.iter().enumerate().filter(|(n, _)| !taken_n.contains(n)) {
        if !cands.contains_key(&new.key) {
            continue;
        }
        let entry = by_key.entry(&new.key).or_default();
        if entry.is_empty() {
            order.push(&new.key);
        }
        entry.push(n);
    }
    order
        .into_iter()
        .map(|key| (by_key.remove(key).unwrap_or_default(), cands.remove(key).unwrap_or_default()))
        .collect()
}

/// What needs a whole-file hash: the new files [`match_moves`] could not pair, and every
/// file found whose content key another file shown shares (new, changed or known), so the
/// next walk can tell them apart once they move.
struct FullNeeded {
    news: HashSet<usize>,
    changed: HashSet<usize>,
    /// Known records (id, path) that have none yet.
    known: Vec<(String, PathBuf)>,
}

fn shared_keys(
    src: &FolderSource,
    def: &Definition,
    news: &[New],
    changed: &[Changed],
    candidates: &[Candidate],
    seen: &HashSet<String>,
    pairs: &[(usize, usize)],
) -> FullNeeded {
    let mut needed = FullNeeded { news: HashSet::new(), changed: HashSet::new(), known: Vec::new() };
    // Left unpaired with candidates still there for their key.
    for (news_of, _) in groups(news, candidates, pairs) {
        needed.news.extend(news_of);
    }
    let mut count: HashMap<&ContentKey, usize> = HashMap::new();
    for key in news.iter().map(|n| &n.key).chain(changed.iter().filter_map(|c| c.key.as_ref())) {
        *count.entry(key).or_default() += 1;
    }
    if count.is_empty() {
        return needed;
    }
    let skip: HashSet<&str> =
        candidates.iter().map(|c| c.ext.as_str()).chain(changed.iter().map(|c| c.ext.as_str())).collect();
    let state = src.store().read();
    let mut known: Vec<(String, &FileRec)> = Vec::new();
    for (ext, rec) in state.files.iter().filter(|(ext, _)| !skip.contains(ext.as_str())) {
        if let Some(n) = count.get_mut(&rec.content) {
            *n += 1;
            if rec.full.is_none() && seen.contains(ext) {
                known.push((ext.clone(), rec));
            }
        }
    }
    let shared = |key: &ContentKey| count.get(key).is_some_and(|n| *n > 1);
    needed.news.extend(news.iter().enumerate().filter(|(_, n)| shared(&n.key)).map(|(i, _)| i));
    needed
        .changed
        .extend(changed.iter().enumerate().filter(|(_, c)| c.key.as_ref().is_some_and(shared)).map(|(i, _)| i));
    needed.known = known
        .into_iter()
        .filter(|(_, rec)| shared(&rec.content))
        .filter_map(|(ext, rec)| Some((ext, rec.path_under(&def.root(&rec.root)?.path))))
        .collect();
    needed
}

/// Hashes whole the files `needed` names, keeping the hashes on them; the known records'
/// go in as changes of their own.
fn hash_whole(
    src: &FolderSource,
    news: &mut [New],
    changed: &mut [Changed],
    needed: &FullNeeded,
    problems: &mut Vec<String>,
) -> Vec<(String, String)> {
    let total = needed.news.len() + needed.changed.len() + needed.known.len();
    if total == 0 {
        return Vec::new();
    }
    let progress = Progress::new(src, READING, "Whole files with the same content", total);
    let hash = |path: &Path| {
        let full = ids::full_hash(path).map_err(|e| progress.problem(format!("{}: {e}", path.display()))).ok();
        progress.step();
        full
    };
    news.par_iter_mut().enumerate().filter(|(i, _)| needed.news.contains(i)).for_each(|(_, n)| {
        n.full = hash(&n.found.path);
    });
    changed.par_iter_mut().enumerate().filter(|(i, _)| needed.changed.contains(i)).for_each(|(_, c)| {
        c.full = hash(&c.found.path);
    });
    let known = needed.known.par_iter().filter_map(|(ext, path)| Some((ext.clone(), hash(path)?))).collect();
    problems.extend(progress.problems());
    known
}

/// Writes the plan in one transaction.
fn write(src: &FolderSource, def: &Definition, plan: Plan) -> Result<ScanOutcome> {
    let Plan { puts, removes, detach, offline, fulls, mut outcome } = plan;
    let any = !(puts.is_empty() && removes.is_empty() && detach.is_empty() && offline.is_empty() && fulls.is_empty());
    let written = src.store().write(|w| {
        let (roots, mut moves) = roots_changed(w, def)?;
        for ext in &removes {
            // Before the record goes: the images are found through it.
            src.images.invalidate(w, src, ext)?;
            w.remove_file(ext)?;
        }
        for (ext, rec) in detach {
            w.put_file(&ext, rec)?;
            w.tomb(&ext)?;
        }
        for (ext, rec) in offline {
            w.put_file(&ext, rec)?;
        }
        // Photos at a new place, while their records still say where they were: their
        // images go where the patterns now put them.
        for (ext, rec, _) in &puts {
            let Some(ext) = ext else { continue };
            if w.state().files.get(ext).is_some_and(|old| old.root != rec.root || old.relpath != rec.relpath) {
                moves.push(Moved { ext: ext.clone(), root: rec.root.clone(), relpath: rec.relpath.clone() });
            }
        }
        let carried = src.images.relocate(w, src, def, &moves)?;
        let now = chrono::Utc::now().timestamp_millis();
        for (ext, mut rec, invalidate) in puts {
            let ext = match ext {
                Some(ext) => ext,
                // An image carried along with its photo (a pattern that puts images among the
                // photos), found by the walk before it was known: not a photo.
                None if def.root(&rec.root).is_some_and(|root| carried.contains(&rec.path_under(&root.path))) => {
                    outcome.added -= 1;
                    continue;
                }
                None => {
                    rec.imported_at = now;
                    new_id(w.state())
                }
            };
            if invalidate {
                src.images.invalidate(w, src, &ext)?;
            }
            w.put_file(&ext, rec)?;
        }
        for (ext, full) in fulls {
            if let Some(rec) = w.state().files.get(&ext).filter(|r| r.full.is_none()) {
                let rec = FileRec { full: Some(full), ..rec.clone() };
                w.put_file(&ext, rec)?;
            }
        }
        if any || roots {
            w.folders_changed()?;
        }
        Ok(outcome)
    });
    // The images it dropped (none if it failed) are deleted now, outside the store's lock.
    src.images.sweep(src);
    written
}

/// Compares the roots with those of the last walk. A root whose path changed has every
/// photo under it touched (their paths changed); any change in the roots means the folder
/// collections changed. Returns whether the roots changed, and the photos under a root
/// whose path changed.
fn roots_changed(w: &mut Writer, def: &Definition) -> Result<(bool, Vec<Moved>)> {
    let mut before: HashMap<String, RootDef> = HashMap::new();
    {
        let table = w.tx().open_table(ROOTS).map_err(other)?;
        for entry in table.iter().map_err(other)? {
            let (id, root) = entry.map_err(other)?;
            if let Ok(root) = serde_json::from_str::<RootDef>(root.value()) {
                before.insert(id.value().to_owned(), root);
            }
        }
    }
    let now: HashMap<String, RootDef> = def.roots.iter().map(|r| (r.id.clone(), r.clone())).collect();
    if before == now {
        return Ok((false, Vec::new()));
    }
    let moved: HashSet<&str> = def
        .roots
        .iter()
        .filter(|r| before.get(&r.id).is_some_and(|b| b.path != r.path))
        .map(|r| r.id.as_str())
        .collect();
    let touch: Vec<Moved> = w
        .state()
        .files
        .iter()
        .filter(|(_, rec)| moved.contains(rec.root.as_str()) && rec.state != FileState::Detached)
        .map(|(ext, rec)| Moved { ext: ext.clone(), root: rec.root.clone(), relpath: rec.relpath.clone() })
        .collect();
    for Moved { ext, .. } in &touch {
        w.touch(ext)?;
    }
    {
        let mut table = w.tx().open_table(ROOTS).map_err(other)?;
        for id in before.keys() {
            table.remove(id.as_str()).map_err(other)?;
        }
        for (id, root) in &now {
            let json = serde_json::to_string(root).map_err(other)?;
            table.insert(id.as_str(), json.as_str()).map_err(other)?;
        }
    }
    Ok((true, touch))
}

/// A photo id not used before: 16 random hex digits.
fn new_id(state: &State) -> String {
    loop {
        let id = uuid::Uuid::new_v4().simple().to_string()[..16].to_owned();
        if !state.files.contains_key(&id) && !state.tombs.contains_key(&id) && !state.user.contains_key(&id) {
            return id;
        }
    }
}

/// One step of the walk's reading phase, counted from many threads.
struct Progress<'a> {
    src: &'a FolderSource,
    phase: &'static str,
    note: &'static str,
    total: u64,
    done: AtomicU64,
    problems: std::sync::Mutex<Vec<String>>,
}

impl<'a> Progress<'a> {
    fn new(src: &'a FolderSource, phase: &'static str, note: &'static str, total: usize) -> Self {
        let progress = Self {
            src,
            phase,
            note,
            total: total as u64,
            done: AtomicU64::new(0),
            problems: std::sync::Mutex::new(Vec::new()),
        };
        if total > 0 {
            progress.show(0);
        }
        progress
    }

    fn step(&self) {
        let done = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        if done.is_multiple_of(64) || done == self.total {
            self.show(done);
        }
    }

    fn show(&self, done: u64) {
        self.src.set_progress(Some(SourceProgress {
            phase: self.phase.into(),
            done,
            total: Some(self.total),
            note: Some(self.note.into()),
        }));
    }

    fn problem(&self, problem: String) {
        self.problems.lock().unwrap().push(problem);
    }

    fn problems(&self) -> Vec<String> {
        std::mem::take(&mut *self.problems.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::rec;

    #[test]
    fn excludes_match_folders_and_what_is_in_them() {
        let mut def = Definition::new("t", &[]);
        def.excludes = vec!["**/tmp/**".into(), "*.bak.jpg".into(), "old/".into(), "[".into()];
        let mut problems = Vec::new();
        let set = excludes(&def, &mut problems);
        assert_eq!(problems.len(), 1, "the broken pattern is reported: {problems:?}");
        for hit in ["tmp", "a/tmp", "a/tmp/x.jpg", "x.bak.jpg", "a/b/x.bak.jpg", "old", ".c1studio", "r/.previews"] {
            assert!(set.is_match(hit), "{hit}");
        }
        for miss in ["tmpx", "a/tmp.jpg", "x.jpg", "older", ".hidden", "a/.c1studiox"] {
            assert!(!set.is_match(miss), "{miss}");
        }
    }

    #[test]
    fn sidecars_follow_the_naming_then_the_other() {
        let dir = tempfile::tempdir().unwrap();
        let photo = dir.path().join("DSCF1.RAF");
        for name in ["dscf1.XMP", "DSCF1.RAF.xmp"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let xmps: HashMap<String, String> =
            ["dscf1.XMP", "DSCF1.RAF.xmp"].iter().map(|n| (n.to_lowercase(), n.to_string())).collect();
        assert_eq!(sidecar(&photo, &xmps, SidecarNaming::Stem).unwrap().0, "dscf1.XMP");
        assert_eq!(sidecar(&photo, &xmps, SidecarNaming::Name).unwrap().0, "DSCF1.RAF.xmp");
        let only_name: HashMap<String, String> =
            [("dscf1.raf.xmp".to_string(), "DSCF1.RAF.xmp".to_string())].into_iter().collect();
        assert_eq!(sidecar(&photo, &only_name, SidecarNaming::Stem).unwrap().0, "DSCF1.RAF.xmp");
        assert_eq!(sidecar(&dir.path().join("other.jpg"), &xmps, SidecarNaming::Stem), None);
    }

    #[test]
    fn what_could_not_be_read_is_kept() {
        let walked =
            Walked { unreadable: vec![("r".into(), "a/b".into()), ("s".into(), String::new())], ..Walked::default() };
        assert!(walked.kept(&rec("r", "a/b/x.jpg")));
        assert!(walked.kept(&rec("r", "a/b")));
        assert!(!walked.kept(&rec("r", "a/bc/x.jpg")));
        assert!(!walked.kept(&rec("q", "a/b/x.jpg")));
        assert!(walked.kept(&rec("s", "anything.jpg")));
    }

    #[test]
    fn a_detached_root_back_but_out_of_reach_shows_its_photos_offline() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.jpg"), b"\xFF\xD8 a photo").unwrap();
        let location = dir.path().join("t.c1folders");
        let def = Definition::new("t", std::slice::from_ref(&root));
        def.save(&location).unwrap();
        let src = FolderSource::open(&location, &dir.path().join("work")).unwrap();
        scan(&src).unwrap();
        let ext = src.store().read().at_path(&def.roots[0].id, "a.jpg").unwrap().to_owned();

        // Taken out of the definition: detached and deleted from the library.
        let mut without = def.clone();
        without.roots.clear();
        without.save(&location).unwrap();
        src.reload_definition(None).unwrap();
        scan(&src).unwrap();
        assert_eq!(src.store().read().files[&ext].state, FileState::Detached);
        assert!(src.store().read().tombs.contains_key(&ext));

        // Back, while its folder cannot be reached: kept, shown again, offline.
        std::fs::rename(&root, dir.path().join("away")).unwrap();
        def.save(&location).unwrap();
        src.reload_definition(None).unwrap();
        let outcome = scan(&src).unwrap();
        assert_eq!(outcome.offline_roots.len(), 1);
        let state = src.store().read();
        assert_eq!(state.files[&ext].state, FileState::Offline);
        assert!(!state.tombs.contains_key(&ext), "no longer deleted");
        assert_eq!(state.changed[&ext], state.generation, "sent again");
    }

    #[test]
    fn a_new_exclude_detaches_instead_of_deleting() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        std::fs::create_dir_all(root.join("skip")).unwrap();
        std::fs::write(root.join("skip").join("a.jpg"), b"\xFF\xD8 a photo").unwrap();
        // So the root is not taken for an empty mount point.
        std::fs::write(root.join("b.jpg"), b"\xFF\xD8 another photo").unwrap();
        let location = dir.path().join("t.c1folders");
        let def = Definition::new("t", std::slice::from_ref(&root));
        def.save(&location).unwrap();
        let src = FolderSource::open(&location, &dir.path().join("work")).unwrap();
        scan(&src).unwrap();
        let ext = src.store().read().at_path(&def.roots[0].id, "skip/a.jpg").unwrap().to_owned();

        let mut excluding = def.clone();
        excluding.excludes.push("skip/**".into());
        excluding.save(&location).unwrap();
        src.reload_definition(None).unwrap();
        scan(&src).unwrap();
        assert_eq!(src.store().read().files[&ext].state, FileState::Detached, "kept, not removed");

        def.save(&location).unwrap();
        src.reload_definition(None).unwrap();
        scan(&src).unwrap();
        let state = src.store().read();
        assert_eq!(state.at_path(&def.roots[0].id, "skip/a.jpg"), Some(ext.as_str()), "same id back");
        assert_eq!(state.files[&ext].state, FileState::Present);
    }

    #[test]
    fn relpaths_use_slashes() {
        let base = Path::new("/r");
        assert_eq!(relpath(base, &base.join("a").join("b.jpg")), "a/b.jpg");
        assert_eq!(relpath(base, base), "");
    }
}
