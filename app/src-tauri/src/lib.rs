//! The desktop app: the library behind Tauri commands, and pixels behind the `c1img:`
//! protocol (`c1img://localhost/thumb/<row>`, `c1img://localhost/preview/<row>`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, TryLockError};
use std::time::{Duration, Instant};

use c1index::thumbs::{Failure, Kind, Progress, Thumbs};
use c1index::{Filter, Library, Origin, PendingEdit, QueryOptions, QueryResult, SyncReport, SyncStep};
use c1model::provider::data_root;
use c1model::{
    BackupInfo, Change, Collection, CollectionChange, CollectionId, ColorLabel, Compatibility, GeoOrigin, GeoPoint,
    ImageTier, Iptc, KeywordPath, KeywordRemap, Op, Photo, PhotoId, PhotoSource, Pick, Place, Removal, SourceProgress,
    SourceSpec, Wording, WriteStatus,
};
use events::Log;
use serde::{Deserialize, Serialize};
use sources::{Offered, Recent};
use tauri::http::{Response, StatusCode, header};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

mod ai;
mod events;
mod folders;
mod sources;

struct Session {
    /// What is open: which provider, and where.
    spec: SourceSpec,
    source: Box<dyn PhotoSource>,
    library: Library,
    /// One sync or write to the source at a time.
    syncing: Mutex<()>,
    /// When the source was last looked at for changes by [`refresh`].
    last_refresh: Mutex<Instant>,
    /// Why the last sync failed, until one succeeds: shown however late the window asks.
    sync_error: Mutex<Option<String>>,
    /// The source has been read once in this session.
    synced: AtomicBool,
    /// `settings.json` beside the library.
    settings_path: PathBuf,
    /// Why `settings.json` could not be read when the catalog was opened, for the window.
    settings_problem: Option<String>,
    /// Write the queue to the source by itself (off unless the user turns it on).
    autosave: AtomicBool,
    /// When the queue last changed, so autosave waits until the user pauses.
    last_edit: Mutex<Instant>,
    /// What autosave last reported, sent to the window only when it changes.
    autosave_state: Mutex<AutosaveState>,
    /// Measuring photos in the background (stage 3): one run at a time, stopped when
    /// another source opens.
    analysing: AtomicBool,
    analysis_stop: AtomicBool,
    analysis: Mutex<AnalysisState>,
    /// Describing photos with a vision model (stage 5): one run at a time, stopped when
    /// asked or when another source opens.
    describing: AtomicBool,
    /// Shared with the run's provider, so a call under way ends on Stop.
    describe_stop: Arc<AtomicBool>,
    describe: Mutex<ai::DescribeState>,
    /// Thumbnails made here for photos the source has none for, and the run that makes
    /// them all (one at a time, stopped when another source opens).
    thumbs: Thumbs,
    filling: AtomicBool,
    fill_stop: AtomicBool,
    fill: Mutex<FillState>,
    /// Counting the photos without a thumbnail, apart from making them; `count_again`
    /// when asked while a count was under way.
    counting: AtomicBool,
    count_again: AtomicBool,
    /// `events.log` beside the library: what happened in the background, and what failed.
    log: Log,
}

/// Photos without a thumbnail, and how far making theirs has got, for the footer.
#[derive(Clone, Debug, Default, Serialize)]
struct FillState {
    running: bool,
    done: usize,
    total: usize,
    made: usize,
    failed: usize,
    /// Photos that show no thumbnail now, once counted.
    missing: Option<usize>,
    /// Why the last run could not make some, most common first, with how many each.
    reasons: Vec<(String, usize)>,
}

impl FillState {
    /// Under way, as far as `p`.
    fn under_way(p: Progress) -> Self {
        FillState {
            running: true,
            done: p.done,
            total: p.total,
            made: p.made,
            failed: p.failed,
            missing: Some(p.total - p.made),
            reasons: Vec::new(),
        }
    }

    /// Done (or stopped) at `p`.
    fn finished(p: Progress, reasons: Vec<(String, usize)>) -> Self {
        FillState { running: false, reasons, ..FillState::under_way(p) }
    }

    /// `missing` photos counted without a thumbnail. A run under way counts for itself.
    /// The last run's failures stand while the photos left are no more than it failed
    /// on; with new photos to make (just imported), it is "Make them" again.
    fn counted(self, missing: usize) -> Self {
        if self.running {
            return self;
        }
        if self.failed == 0 || missing > self.failed {
            return FillState { missing: Some(missing), ..FillState::default() };
        }
        FillState { missing: Some(missing), failed: self.failed.min(missing), ..self }
    }

    /// One made for a tile that asked: one fewer missing. A run under way counts for
    /// itself.
    fn made_one(self) -> Self {
        if self.running {
            return self;
        }
        let missing = self.missing.map(|n| n.saturating_sub(1));
        FillState { missing, failed: self.failed.min(missing.unwrap_or(0)), ..self }
    }
}

/// Sets a flag back however the work it marks ends, a panic included.
struct Lowers<'a>(&'a AtomicBool);

impl Drop for Lowers<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// How far the background analysis has got, for the footer.
#[derive(Clone, Debug, Default, Serialize)]
struct AnalysisState {
    running: bool,
    done: usize,
    total: usize,
    failed: usize,
}

impl Session {
    fn touched(&self) {
        *self.last_edit.lock().unwrap() = Instant::now();
    }
}

/// The app's own settings, kept per catalog. The History tab is not one of them: it is
/// the library's own ([`Library::set_history`]), so `c1cat` follows it too.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    autosave: bool,
    /// Where the first builds with the History tab kept it: carried over into the library
    /// once, and not written again.
    #[serde(skip_serializing_if = "Option::is_none")]
    history: Option<bool>,
}

/// The settings in `path`, and what went wrong reading them. None there yet is the
/// defaults. One that does not parse is set aside as `settings.json.bad` rather than taken
/// for the user's choice: the defaults (autosave off) apply, and the window is told.
fn read_settings(path: &Path) -> (Settings, Option<String>) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Settings::default(), None),
        Err(e) => return (Settings::default(), Some(format!("could not read {}: {e}", path.display()))),
    };
    match serde_json::from_slice(&bytes) {
        Ok(settings) => (settings, None),
        Err(e) => {
            let bad = path.with_extension("json.bad");
            let kept = match std::fs::rename(path, &bad) {
                Ok(()) => format!("set aside as {}", bad.display()),
                Err(e) => format!("could not set it aside: {e}"),
            };
            (Settings::default(), Some(format!("{} is unreadable ({e}); {kept}. Autosave is off.", path.display())))
        }
    }
}

/// Writes the settings whole or not at all: into a file beside it, then over it.
fn write_settings(path: &Path, settings: &Settings) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(settings)?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut file, &bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

impl Session {
    /// Writes the settings with autosave `autosave`.
    fn save_settings(&self, autosave: bool) -> Result<(), String> {
        write_settings(&self.settings_path, &Settings { autosave, history: None })
            .map_err(|e| format!("could not save {}: {e}", self.settings_path.display()))
    }
}

/// The source asked for and its session, or why there is none (nothing chosen yet, the
/// catalog is gone, the library is unreadable). Another source can be opened at any time.
struct Current {
    spec: Option<SourceSpec>,
    session: Result<Arc<Session>, String>,
}

struct App(RwLock<Current>);

/// What `Current::session` says while the first source is still being opened.
const STARTING: &str = "starting";

/// How far opening a source has got, for the window's start-up dialog.
#[derive(Clone, Serialize)]
struct Startup {
    /// "opening", "reading", "indexing" or "saving".
    step: &'static str,
    done: u64,
    total: u64,
}

/// The start-up dialog's state: set from when a source starts opening until it is ready
/// to use (for a library with no photos yet, until its first fill is in). Syncs outside
/// that, such as the periodic look at Capture One, are not reported.
#[derive(Default)]
struct Boot {
    shown: Mutex<Option<Startup>>,
    active: AtomicBool,
    last_emit: Mutex<Option<Instant>>,
}

impl Boot {
    fn begin(&self, app: &AppHandle) {
        self.active.store(true, Ordering::SeqCst);
        self.set(app, Some(Startup { step: "opening", done: 0, total: 0 }), true);
    }

    fn finish(&self, app: &AppHandle) {
        self.active.store(false, Ordering::SeqCst);
        self.set(app, None, true);
    }

    /// The sync reached `step`. Counts come often, so they reach the window a few times a second.
    fn sync_step(&self, app: &AppHandle, step: SyncStep) {
        if !self.active.load(Ordering::SeqCst) {
            return;
        }
        let (shown, counting) = match step {
            SyncStep::Reading => (Startup { step: "reading", done: 0, total: 0 }, false),
            SyncStep::Indexing { done, total } => {
                (Startup { step: "indexing", done: done as u64, total: total as u64 }, true)
            }
            SyncStep::Saving => (Startup { step: "saving", done: 0, total: 0 }, false),
        };
        self.set(app, Some(shown), !counting);
    }

    fn set(&self, app: &AppHandle, shown: Option<Startup>, always: bool) {
        *self.shown.lock().unwrap() = shown.clone();
        let mut last = self.last_emit.lock().unwrap();
        if always || last.is_none_or(|t| t.elapsed() >= Duration::from_millis(100)) {
            *last = Some(Instant::now());
            let _ = app.emit("startup-progress", shown);
        }
    }
}

/// The start-up dialog's state, for a window that asks after the events went by.
#[tauri::command]
async fn startup_progress(boot: State<'_, Boot>) -> Result<Option<Startup>, String> {
    Ok(boot.shown.lock().unwrap().clone())
}

impl App {
    fn session(&self) -> Result<Arc<Session>, String> {
        self.0.read().unwrap().session.clone()
    }
}

/// Opens a source through its provider, with the app's data about it (library,
/// settings, backups) in a folder of its own.
fn open(spec: &SourceSpec) -> anyhow::Result<Session> {
    let provider = sources::provider(&spec.provider).map_err(anyhow::Error::msg)?;
    let dir = data_root().join(provider.data_key(&spec.location));
    let source = provider.open(&spec.location, &dir.join("work"))?;
    let library = Library::open(dir.join("library"))?;
    let settings_path = dir.join("settings.json");
    let (settings, settings_problem) = read_settings(&settings_path);
    // History turned off where the first builds kept it moves into the library, before
    // the first sync, so what that logs follows it; the file then drops it.
    let mut carried = None;
    if let Some(history) = settings.history {
        if !history && library.history_on() {
            library.set_history(false)?;
        }
        carried = Some(write_settings(&settings_path, &Settings { history: None, ..settings }));
    }
    let session = Session {
        spec: spec.clone(),
        source,
        library,
        syncing: Mutex::new(()),
        last_refresh: Mutex::new(Instant::now()),
        sync_error: Mutex::new(None),
        synced: AtomicBool::new(false),
        settings_path,
        settings_problem,
        autosave: AtomicBool::new(settings.autosave),
        last_edit: Mutex::new(Instant::now()),
        autosave_state: Mutex::new(AutosaveState::Off),
        analysing: AtomicBool::new(false),
        analysis_stop: AtomicBool::new(false),
        analysis: Mutex::new(AnalysisState::default()),
        describing: AtomicBool::new(false),
        describe_stop: Arc::default(),
        describe: Mutex::new(ai::DescribeState::default()),
        thumbs: Thumbs::new(dir.join("thumbs")),
        filling: AtomicBool::new(false),
        fill_stop: AtomicBool::new(false),
        fill: Mutex::new(FillState::default()),
        counting: AtomicBool::new(false),
        count_again: AtomicBool::new(false),
        log: Log::new(dir.join("events.log")),
    };
    session.log.event(&format!("Opened {}", spec.location), &[]);
    if let Some(problem) = &session.settings_problem {
        session.log.event(&format!("Settings: {problem}"), &[]);
    }
    if let Some(Err(e)) = carried {
        session
            .log
            .event(&format!("Settings: History moved into the library, but settings.json still has it: {e}"), &[]);
    }
    Ok(session)
}

/// Whether `s` is still the session open. Work left over from one closed since (its
/// run stops after the photo it is on) tells the window nothing, nor starts more.
fn is_current(app: &AppHandle, s: &Arc<Session>) -> bool {
    app.state::<App>().session().is_ok_and(|now| Arc::ptr_eq(&now, s))
}

/// Changes the footer's thumbnail state, and tells the window.
fn report_fill(app: &AppHandle, s: &Arc<Session>, change: impl FnOnce(FillState) -> FillState) {
    let state = {
        let mut fill = s.fill.lock().unwrap_or_else(|e| e.into_inner());
        *fill = change(std::mem::take(&mut *fill));
        fill.clone()
    };
    if is_current(app, s) {
        let _ = app.emit("thumbs", state);
    }
}

/// Counts the photos without a thumbnail, in the background. Asked while counting, it
/// counts once more after.
fn count_missing(app: &AppHandle) {
    let Ok(s) = app.state::<App>().session() else { return };
    s.count_again.store(true, Ordering::SeqCst);
    if s.counting.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        loop {
            while s.count_again.swap(false, Ordering::SeqCst) && is_current(&handle, &s) {
                let counted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    s.thumbs.missing(&s.library, s.source.as_ref())
                }));
                match counted {
                    Ok(Ok(missing)) => report_fill(&handle, &s, |state| state.counted(missing.len())),
                    Ok(Err(e)) => s.log.event(&format!("Counting the photos without a thumbnail failed: {e:#}"), &[]),
                    Err(_) => s.log.event("Counting the photos without a thumbnail crashed", &[]),
                }
            }
            s.counting.store(false, Ordering::SeqCst);
            // Asked again just as this count ended: count once more, unless another
            // count has started meanwhile.
            if !s.count_again.load(Ordering::SeqCst) || s.counting.swap(true, Ordering::SeqCst) {
                break;
            }
        }
    });
}

/// Makes a thumbnail for every photo without one, from its preview or else its original,
/// then measures the new ones. Photos that failed before are tried again.
#[tauri::command]
async fn fill_thumbnails(app: AppHandle) -> Result<(), String> {
    start_fill(&app)
}

/// Starts [`fill_thumbnails`]' run in the background, unless one is under way.
fn start_fill(app: &AppHandle) -> Result<(), String> {
    let s = app.state::<App>().session()?;
    if s.filling.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let _done = Lowers(&s.filling);
        s.thumbs.forget();
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_fill(&app, &s)));
        if run.is_err() {
            s.log.event("Making thumbnails stopped: it crashed", &[]);
            report_fill(&app, &s, |state| FillState { running: false, ..state });
        }
        if is_current(&app, &s) {
            let _ = app.emit("thumbs-done", ());
            start_analysis(&app);
        }
    });
    Ok(())
}

fn run_fill(app: &AppHandle, s: &Arc<Session>) {
    let missing = match s.thumbs.missing(&s.library, s.source.as_ref()) {
        Ok(missing) => missing,
        Err(e) => {
            s.log.event(&format!("Making thumbnails failed: {e:#}"), &[]);
            if is_current(app, s) {
                let _ = app.emit("sync-failed", format!("thumbnails: {e:#}"));
            }
            return;
        }
    };
    s.log.event(&format!("Making thumbnails for {} photos the source has none for", missing.len()), &[]);
    // One place of the few is left for the window's tiles.
    let threads = (c1index::thumbs::MAKERS - 1).max(1);
    let (done, failures) = s.thumbs.fill(s.source.as_ref(), &missing, threads, &s.fill_stop, |p| {
        report_fill(app, s, |_| FillState::under_way(p));
    });
    let reasons = log_fill(s, &done, &failures);
    report_fill(app, s, |_| FillState::finished(done, reasons));
}

/// Writes how a thumbnail run went to the events log: a summary, why photos failed and
/// where their originals were looked for, then each photo. Returns the reasons counted.
fn log_fill(s: &Session, done: &Progress, failures: &[Failure]) -> Vec<(String, usize)> {
    let stopped = if done.done < done.total { " (stopped early)" } else { "" };
    let summary =
        format!("Thumbnails: made {} of {}, {} could not be made{stopped}", done.made, done.total, done.failed);
    if failures.is_empty() {
        s.log.event(&summary, &[]);
        return Vec::new();
    }
    let mut reasons = BTreeMap::<Kind, usize>::new();
    // Folders whose originals were not found: usually one drive not connected.
    let mut folders = BTreeMap::<String, usize>::new();
    let mut lines = Vec::new();
    for f in failures {
        *reasons.entry(f.why.kind).or_default() += 1;
        let photo = s.library.photo(f.row).ok().flatten();
        let name = photo.as_ref().map_or_else(
            || format!("row {}", f.row),
            |p| match p.variant_index {
                0 => p.file.name.clone(),
                n => format!("{} (variant {})", p.file.name, n + 1),
            },
        );
        let path = photo.as_ref().and_then(|p| p.file.path.clone());
        if f.why.kind == Kind::Unreachable {
            let folder = path.as_deref().and_then(|p| std::path::Path::new(p).parent());
            *folders
                .entry(folder.map_or("(no path in the catalog)".into(), |d| d.display().to_string()))
                .or_default() += 1;
        }
        lines.push(format!(
            "{name}  —  {}  —  {}",
            f.why.detail,
            path.as_deref().unwrap_or("(no path in the catalog)")
        ));
    }
    let mut reasons: Vec<(String, usize)> = reasons.into_iter().map(|(k, n)| (k.reason().to_string(), n)).collect();
    reasons.sort_by_key(|r| std::cmp::Reverse(r.1));
    let mut folders: Vec<_> = folders.into_iter().collect();
    folders.sort_by_key(|f| std::cmp::Reverse(f.1));
    let mut details = vec!["Why:".to_string()];
    details.extend(reasons.iter().map(|(r, n)| format!("  {n:>6}  {r}")));
    if !folders.is_empty() {
        details.push("Originals not found, by folder (is the drive connected?):".into());
        details.extend(folders.iter().take(30).map(|(d, n)| format!("  {n:>6}  {d}")));
        if folders.len() > 30 {
            details.push(format!("  … and {} more folders", folders.len() - 30));
        }
    }
    details.push("Photos:".into());
    details.extend(lines.into_iter().map(|l| format!("  {l}")));
    s.log.event(&summary, &details);
    reasons
}

/// Opens the events log in the system's text viewer.
#[tauri::command]
async fn open_log(app: State<'_, App>) -> Result<(), String> {
    let s = app.session()?;
    s.log.open().map_err(|e| format!("Could not open {}: {e}", s.log.path().display()))
}

#[tauri::command]
async fn thumbs_state(app: State<'_, App>) -> Result<FillState, String> {
    Ok(app.session()?.fill.lock().unwrap().clone())
}

/// Measures the photos not measured yet, in the background, unless that is under way.
/// Uses half the cores, so the window stays quick.
fn start_analysis(app: &AppHandle) {
    let Ok(s) = app.state::<App>().session() else { return };
    if s.analysing.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        let _done = Lowers(&s.analysing);
        let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).max(1));
        let report = |state: AnalysisState| {
            *s.analysis.lock().unwrap() = state.clone();
            if is_current(&handle, &s) {
                let _ = handle.emit("analysis", state);
            }
        };
        let result = c1index::analyse::run(&s.library, s.source.as_ref(), &s.thumbs, threads, &s.analysis_stop, |p| {
            report(AnalysisState { running: true, done: p.done, total: p.total, failed: p.failed });
        });
        let last = s.analysis.lock().unwrap().clone();
        report(AnalysisState { running: false, ..last });
        if let Err(e) = &result {
            s.log.event(&format!("Measuring photos failed: {e:#}"), &[]);
        }
        if is_current(&handle, &s) {
            if let Err(e) = result {
                let _ = handle.emit("sync-failed", format!("analysis: {e:#}"));
            }
            let _ = handle.emit("analysis-done", ());
        }
    });
}

/// What the analysis found for a photo: its measures, classes, and the duplicate group
/// and series it is in, with their sizes.
#[derive(Serialize)]
struct LookView {
    analysis: c1analysis::Analysis,
    /// The lines the classes below were drawn with, for saying why.
    lines: c1analysis::ClassSettings,
    shape: &'static str,
    hues: Vec<&'static str>,
    light: Vec<&'static str>,
    focus: &'static str,
    duplicates: Option<(String, usize)>,
    series: Option<(String, usize)>,
}

#[tauri::command]
async fn look(app: State<'_, App>, row: u64) -> Result<Option<LookView>, String> {
    let s = app.session()?;
    let Some(look) = s.library.look(row) else { return Ok(None) };
    let size = |group: Option<String>| -> Result<Option<(String, usize)>, String> {
        let Some(group) = group else { return Ok(None) };
        let filter = Filter { group: Some(group.clone()), ..Filter::default() };
        let options = QueryOptions { limit: Some(0), ..QueryOptions::default() };
        Ok(Some((group, s.library.query(&filter, &options).map_err(text)?.total)))
    };
    let a = &look.analysis;
    let classes = s.library.class_settings();
    Ok(Some(LookView {
        shape: a.shape(&classes),
        hues: a.hues(&classes),
        light: a.light(&classes),
        focus: a.focus(&classes),
        duplicates: size(look.duplicates.clone())?,
        series: size(look.series.clone())?,
        lines: classes.clone(),
        analysis: look.analysis,
    }))
}

/// Where this library draws the lines between analysis classes.
#[tauri::command]
async fn class_settings(app: State<'_, App>) -> Result<c1analysis::ClassSettings, String> {
    Ok(app.session()?.library.class_settings())
}

/// The lines the app draws until the user moves them.
#[tauri::command]
fn default_class_settings() -> c1analysis::ClassSettings {
    c1analysis::ClassSettings::default()
}

/// Classes the measured photos again by these settings; returns what was kept.
#[tauri::command]
async fn set_class_settings(
    app: State<'_, App>,
    settings: c1analysis::ClassSettings,
) -> Result<c1analysis::ClassSettings, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || s.library.set_class_settings(&settings))
        .await
        .map_err(text)?
        .map_err(|e| format!("{e:#}"))
}

/// How many photos these settings would put in each class, without applying them.
#[tauri::command]
async fn preview_classes(
    app: State<'_, App>,
    settings: c1analysis::ClassSettings,
) -> Result<c1index::ClassPreview, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || s.library.preview_classes(&settings))
        .await
        .map_err(text)?
        .map_err(|e| format!("{e:#}"))
}

/// The system dialog for a GPX track; None if cancelled.
#[tauri::command]
async fn pick_gpx(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app.dialog().file().set_title("GPX track").add_filter("GPX track", &["gpx"]).blocking_pick_file();
        picked.map(|p| p.into_path().map(|p| p.display().to_string())).transpose().map_err(text)
    })
    .await
    .map_err(text)?
}

#[derive(Deserialize)]
struct GpxSettings {
    path: String,
    /// The camera clock's distance east of UTC, in minutes: Capture One keeps the time on
    /// the camera's clock.
    offset_minutes: i64,
    /// Fixes further apart than this are a break in the recording.
    max_gap_minutes: i64,
    /// Also move photos that already have a position.
    overwrite: bool,
}

#[derive(Serialize)]
struct GpxPlan {
    fixes: usize,
    /// The track's first and last fix, UTC milliseconds.
    from: i64,
    to: i64,
    /// Photos in view taken during the track.
    candidates: usize,
    /// Of them, the ones the track places.
    matched: usize,
    /// A few of them, to show: row, latitude, longitude.
    sample: Vec<(u64, f64, f64)>,
}

/// How many photos were considered, the track, and the photos it places with where.
type GpxMatches = (usize, Vec<c1index::gpx::Fix>, Vec<(u64, Photo, (f64, f64))>);

/// The photos `filter` selects that the track places: row, the photo's position then (for
/// undo), and the new one.
fn gpx_matches(s: &Session, filter: &Filter, set: &GpxSettings) -> Result<GpxMatches, String> {
    let file = std::fs::read_to_string(&set.path).map_err(|e| format!("{}: {e}", set.path))?;
    let fixes = c1index::gpx::parse(&file).map_err(text_of)?;
    let offset = set.offset_minutes * 60_000;
    let (first, last) = (fixes[0].at + offset, fixes[fixes.len() - 1].at + offset);
    let window = c1index::TimeRange {
        from: Some(filter.captured.and_then(|c| c.from).map_or(first, |f| f.max(first))),
        to: Some(filter.captured.and_then(|c| c.to).map_or(last, |t| t.min(last))),
    };
    let narrowed = Filter {
        captured: Some(window),
        gps: if set.overwrite { filter.gps } else { Some(false) },
        within: if set.overwrite { filter.within } else { None },
        ..filter.clone()
    };
    let rows = s.library.query(&narrowed, &QueryOptions::default()).map_err(text)?.rows;
    let mut found = Vec::new();
    for row in &rows {
        let Some(photo) = s.library.photo(*row).map_err(text)? else { continue };
        let Some(taken) = photo.captured_at else { continue };
        if let Some(at) = c1index::gpx::locate(&fixes, taken - offset, set.max_gap_minutes * 60_000) {
            found.push((*row, photo, at));
        }
    }
    Ok((rows.len(), fixes, found))
}

fn text_of(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// What importing the track would do to the photos `filter` selects.
#[tauri::command]
async fn gpx_plan(app: State<'_, App>, filter: Filter, settings: GpxSettings) -> Result<GpxPlan, String> {
    let s = app.session()?;
    let (candidates, fixes, found) = gpx_matches(&s, &filter, &settings)?;
    Ok(GpxPlan {
        fixes: fixes.len(),
        from: fixes[0].at,
        to: fixes[fixes.len() - 1].at,
        candidates,
        matched: found.len(),
        sample: found.iter().take(5).map(|(row, _, (lat, lon))| (*row, *lat, *lon)).collect(),
    })
}

/// Gives the photos the track places their positions, as one edit. Returns the ops that undo it.
#[tauri::command]
async fn gpx_apply(app: State<'_, App>, filter: Filter, settings: GpxSettings) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    let (_, _, found) = gpx_matches(&s, &filter, &settings)?;
    let changes: Vec<(u64, Op)> = found
        .into_iter()
        .map(|(row, photo, (lat, lon))| {
            let value = GeoPoint { lat, lon, alt: None, origin: GeoOrigin::Gpx };
            (row, Op::SetGeo { base: photo.geo, value: Some(value) })
        })
        .collect();
    s.touched();
    s.library.edit_as(&changes, Origin::Gpx).map_err(text_of)
}

/// A page of the duplicate groups (or series) among the photos `filter` selects, biggest
/// first, with how many there are in all.
#[tauri::command]
async fn groups(
    app: State<'_, App>,
    filter: Filter,
    series: bool,
    sort: Option<c1index::GroupSort>,
    reverse: Option<bool>,
    offset: usize,
    limit: usize,
) -> Result<c1index::GroupPage, String> {
    let (sort, reverse) = (sort.unwrap_or_default(), reverse.unwrap_or(false));
    app.session()?.library.groups_sorted(&filter, series, sort, reverse, offset, limit).map_err(text)
}

#[tauri::command]
async fn analysis_state(app: State<'_, App>) -> Result<AnalysisState, String> {
    Ok(app.session()?.analysis.lock().unwrap().clone())
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
struct Status {
    catalog: String,
    source: String,
    /// How the UI names the source in its text.
    wording: Wording,
    photos: u64,
    writes: WriteStatus,
    sync_error: Option<String>,
    /// What made the source and how far the app is checked against it.
    compatibility: Compatibility,
    /// The provider's id: `capture-one`, `folders`.
    provider: String,
    /// The source can be asked to look at its files again ([`rescan`]).
    can_rescan: bool,
    /// Places of the source that could not be reached at its last read (a folder source's roots).
    offline_roots: Vec<String>,
    /// How far a scan has got, while one runs.
    progress: Option<SourceProgress>,
    /// What emptying the trash does at the source.
    removes: Removal,
    /// A position belongs to the image, so moving one variant moves the others (Capture One).
    geo_per_asset: bool,
    /// Photos in the app's trash.
    binned: usize,
}

#[tauri::command]
async fn status(app: State<'_, App>) -> Result<Status, String> {
    let s = app.session()?;
    Ok(Status {
        catalog: s.spec.location.clone(),
        source: s.source.info().name,
        wording: s.source.wording(),
        photos: s.library.len(),
        writes: s.source.write_status(),
        sync_error: s.sync_error.lock().unwrap().clone(),
        compatibility: s.source.info().compatibility,
        provider: s.spec.provider.clone(),
        can_rescan: s.source.capabilities().rescan,
        offline_roots: s.source.offline_places(),
        progress: s.source.progress(),
        removes: s.source.capabilities().removes,
        geo_per_asset: s.source.capabilities().geo_per_asset,
        binned: s.library.bin_count(),
    })
}

#[tauri::command]
async fn query(app: State<'_, App>, filter: Filter, options: QueryOptions) -> Result<QueryResult, String> {
    app.session()?.library.query(&filter, &options).map_err(|e| format!("{e:#}"))
}

/// The map's markers for the photos `filter` selects, clustered for what the map shows.
#[tauri::command]
async fn map(app: State<'_, App>, filter: Filter, view: c1index::MapView) -> Result<c1index::MapResult, String> {
    app.session()?.library.map(&filter, &view).map_err(|e| format!("{e:#}"))
}

/// The places `filter`'s photos name that the gazetteer does not know at that level and
/// that were not looked up online yet.
#[tauri::command]
async fn unresolved_places(app: State<'_, App>, filter: Filter) -> Result<Vec<c1index::Unresolved>, String> {
    app.session()?.library.unresolved_places(&filter).map_err(|e| format!("{e:#}"))
}

/// What online lookups found for places (or that they found nothing).
#[tauri::command]
async fn set_place_points(app: State<'_, App>, items: Vec<c1index::PlaceLookup>) -> Result<(), String> {
    app.session()?.library.set_place_points(&items).map_err(|e| format!("{e:#}"))
}

/// Gives the photos at each place (of those `filter` selects, with no position) the
/// place's position. Returns the ops that undo it.
#[tauri::command]
async fn promote_places(app: State<'_, App>, filter: Filter, paths: Vec<String>) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    s.touched();
    s.library.promote(&filter, &paths).map_err(|e| format!("{e:#}"))
}

/// Gives the photos among `rows` with no position their place's position.
#[tauri::command]
async fn promote_rows(app: State<'_, App>, rows: Vec<u64>) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    s.touched();
    s.library.promote_rows(&rows).map_err(|e| format!("{e:#}"))
}

/// For the photos among `rows` with a position, the place fields it would fill.
#[tauri::command]
async fn fill_plan(app: State<'_, App>, rows: Vec<u64>) -> Result<Vec<c1index::FillGroup>, String> {
    app.session()?.library.fill_plan(&rows).map_err(|e| format!("{e:#}"))
}

/// Fills the empty place fields of the photos among `rows` from their positions.
#[tauri::command]
async fn fill_places(app: State<'_, App>, rows: Vec<u64>) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    s.touched();
    s.library.fill_places(&rows).map_err(|e| format!("{e:#}"))
}

/// What placing `filter`'s photos from the photos taken just before and after would do.
#[tauri::command]
async fn neighbour_plan(
    app: State<'_, App>,
    filter: Filter,
    settings: c1index::NeighbourSettings,
) -> Result<c1index::NeighbourPlan, String> {
    app.session()?.library.neighbour_plan(&filter, &settings).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn neighbour_apply(
    app: State<'_, App>,
    filter: Filter,
    settings: c1index::NeighbourSettings,
) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    s.touched();
    s.library.neighbour_apply(&filter, &settings).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn refine_plan(
    app: State<'_, App>,
    filter: Filter,
    settings: c1index::RefineSettings,
) -> Result<c1index::RefinePlan, String> {
    app.session()?.library.refine_plan(&filter, &settings).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn refine_apply(
    app: State<'_, App>,
    filter: Filter,
    settings: c1index::RefineSettings,
) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    s.touched();
    s.library.refine_apply(&filter, &settings).map_err(|e| format!("{e:#}"))
}

/// Smart album id → how many photos it holds under `filter` (its album pick aside).
#[tauri::command]
async fn smart_counts(app: State<'_, App>, filter: Filter) -> Result<Vec<(String, u64)>, String> {
    app.session()?.library.smart_counts(&filter).map_err(|e| format!("{e:#}"))
}

/// The trips, each counted under `filter` (its own trip aside), and the trip settings.
#[tauri::command]
async fn trips(app: State<'_, App>, filter: Filter) -> Result<c1index::TripList, String> {
    app.session()?.library.trips(&filter).map_err(|e| format!("{e:#}"))
}

/// Photos per day under `filter`: (local midnight in ms, count), in date order.
#[tauri::command]
async fn timeline(app: State<'_, App>, filter: Filter) -> Result<Vec<(i64, usize)>, String> {
    app.session()?.library.timeline(&filter).map_err(|e| format!("{e:#}"))
}

/// The photos `filter` selects that have a position, in the order they were taken.
#[tauri::command]
async fn route(app: State<'_, App>, filter: Filter) -> Result<Vec<c1index::RoutePoint>, String> {
    app.session()?.library.route(&filter).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn set_trip_settings(app: State<'_, App>, settings: c1index::trips::TripSettings) -> Result<(), String> {
    app.session()?.library.set_trip_settings(&settings).map_err(|e| format!("{e:#}"))
}

/// Renames, keeps, merges, splits, dismisses or makes a trip; returns the trip to select.
#[tauri::command]
async fn edit_trips(app: State<'_, App>, edit: c1index::trips::TripEdit) -> Result<Option<u64>, String> {
    app.session()?.library.edit_trips(edit).map_err(|e| format!("{e:#}"))
}

/// A photo as the details panel shows it: what its source holds, and whether it is hidden
/// or in the trash here.
#[derive(Serialize)]
struct PhotoView {
    #[serde(flatten)]
    photo: Photo,
    hidden: bool,
    binned: bool,
}

#[tauri::command]
async fn photo(app: State<'_, App>, row: u64) -> Result<Option<PhotoView>, String> {
    let s = app.session()?;
    let photo = s.library.photo(row).map_err(text)?;
    Ok(photo.map(|photo| PhotoView { photo, hidden: s.library.is_hidden(row), binned: s.library.is_binned(row) }))
}

/// Hides photos from every view (`hidden`), or shows them again; returns the rows whose
/// mark changed, for undo. A mark of C1 Studio's own: nothing is written to the source.
#[tauri::command]
async fn set_hidden(app: State<'_, App>, rows: Vec<u64>, hidden: bool) -> Result<Vec<u64>, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || s.library.set_hidden(&rows, hidden))
        .await
        .map_err(text)?
        .map_err(|e| format!("{e:#}"))
}

/// Of these photos, the ones that are hidden: what `h` and the Hide button go by.
#[tauri::command]
async fn hidden_of(app: State<'_, App>, rows: Vec<u64>) -> Result<Vec<u64>, String> {
    Ok(app.session()?.library.hidden_of(&rows))
}

/// Of the photos `filter` selects, those that are hidden: for marking their tiles.
#[tauri::command]
async fn hidden_rows(app: State<'_, App>, filter: Filter) -> Result<Vec<u64>, String> {
    app.session()?.library.hidden_rows(&filter).map_err(text)
}

/// Moves photos to the app's trash (`binned`), or restores them; returns the rows whose
/// mark changed, for undo. Nothing reaches the source until the trash is emptied. A source
/// that deletes whole assets (Capture One) takes their other variants along.
#[tauri::command]
async fn set_binned(app: State<'_, App>, rows: Vec<u64>, binned: bool) -> Result<Vec<u64>, String> {
    let s = app.session()?;
    let whole_assets = s.source.capabilities().removes == Removal::MarksTrashed;
    tauri::async_runtime::spawn_blocking(move || s.library.set_binned(&rows, binned, whole_assets))
        .await
        .map_err(text)?
        .map_err(|e| format!("{e:#}"))
}

/// What emptying the trash did.
#[derive(Default, Serialize)]
struct EmptyReport {
    removed: usize,
    /// Photos left in the trash, by name, with why.
    failed: Vec<(String, String)>,
    backup: Option<String>,
    warnings: Vec<String>,
}

/// Deletes the photos in the trash for good (only those at `rows`, if given): the source
/// deletes their files, or marks them deleted (refused while Capture One runs). Not undone.
#[tauri::command]
async fn empty_bin(app: AppHandle, rows: Option<Vec<u64>>) -> Result<EmptyReport, String> {
    app.state::<App>().session()?.touched();
    tauri::async_runtime::spawn_blocking(move || empty_trash(&app, rows)).await.map_err(text)?
}

fn empty_trash(app: &AppHandle, rows: Option<Vec<u64>>) -> Result<EmptyReport, String> {
    let s = app.state::<App>().session()?;
    // Held until the source is read back, so no refresh comes in between.
    let _one = s.syncing.lock().unwrap_or_else(|e| e.into_inner());
    let mut going = s.library.bin_photos().map_err(|e| format!("{e:#}"))?;
    if let Some(rows) = &rows {
        going.retain(|(row, _)| rows.contains(row));
    }
    if going.is_empty() {
        return Ok(EmptyReport::default());
    }
    let ids: Vec<PhotoId> = going.iter().map(|(_, id)| id.clone()).collect();
    let name = s.source.info().name;
    let report = s.source.remove(&ids).map_err(|e| {
        s.log.event(&format!("Deleting {} photo(s) in {name} failed: {e:#}", ids.len()), &[]);
        text(e)
    })?;
    s.log.event(
        &format!("Deleted {} photo(s) for good in {name}", report.removed.len()),
        &report.backup.iter().map(|b| format!("backup first: {b}")).chain(report.warnings.clone()).collect::<Vec<_>>(),
    );
    s.library.emptied(&report.removed).map_err(|e| format!("{e:#}"))?;
    let named = |id: &PhotoId| {
        let row = going.iter().find(|(_, i)| i == id).map(|(row, _)| *row);
        row.and_then(|row| s.library.photo(row).ok().flatten()).map_or_else(|| id.to_string(), |p| p.file.name)
    };
    let failed = report.failed.iter().map(|(id, why)| (named(id), why.clone())).collect();
    sync_held(app, &s, true, Origin::Source)?;
    Ok(EmptyReport { removed: report.removed.len(), failed, backup: report.backup, warnings: report.warnings })
}

#[tauri::command]
async fn collections(app: State<'_, App>) -> Result<Vec<Collection>, String> {
    app.session()?.library.collections().map_err(text)
}

#[tauri::command]
async fn sync(app: AppHandle) -> Result<SyncReport, String> {
    tauri::async_runtime::spawn_blocking(move || run_sync(&app)).await.map_err(text)?
}

/// Pulls the source's changes in, and tells the window when anything moved.
fn run_sync(app: &AppHandle) -> Result<SyncReport, String> {
    let s = app.state::<App>().session()?;
    let _one = s.syncing.lock().unwrap_or_else(|e| e.into_inner());
    sync_held(app, &s, true, Origin::Source)
}

/// [`run_sync`] for a caller holding `s.syncing`, with what changed logged as coming from
/// `origin`. With `recheck` false, thumbnails and analysis are looked at again only when
/// the catalog changed.
fn sync_held(app: &AppHandle, s: &Session, recheck: bool, origin: Origin) -> Result<SyncReport, String> {
    let boot = app.state::<Boot>();
    let done = Finish::default();
    let report = std::thread::scope(|scope| {
        scope.spawn(|| report_progress(app, s, &done));
        // Ends the poller also if the sync panics, or the scope would wait for it for ever.
        let _ends = FinishOnDrop(&done);
        s.library
            .sync_with(s.source.as_ref(), origin, &mut |step| boot.sync_step(app, step))
            .map_err(|e| format!("{e:#}"))
    });
    let failed = report.as_ref().err().cloned();
    if let Some(e) = &failed
        && s.sync_error.lock().unwrap().as_ref() != Some(e)
    {
        s.log.event(&format!("Reading the source failed: {e}"), &[]);
    }
    *s.sync_error.lock().unwrap() = failed;
    let report = report?;
    let changed = report.changed();
    // A source that makes its own images has none yet for what it just found; the first
    // read of a session also looks for those left from before.
    let first = !s.synced.swap(true, Ordering::SeqCst);
    let fill = (changed || first) && s.source.capabilities().makes_images;
    if changed {
        s.log.event(
            &format!(
                "Read the source{}: {} added, {} updated, {} deleted",
                if report.full { " in full" } else { "" },
                report.added,
                report.updated,
                report.deleted
            ),
            &[],
        );
        let _ = app.emit("library-changed", &report);
    }
    if !recheck && !changed {
        if fill {
            let _ = start_fill(app);
        }
        return Ok(report);
    }
    // Previews may have been rendered, or crops changed: made thumbnails are checked
    // against their sources again, and photos that failed may succeed now.
    s.thumbs.forget();
    start_analysis(app);
    count_missing(app);
    if fill {
        let _ = start_fill(app);
    }
    Ok(report)
}

/// How often the source is looked at while the app runs, besides when the app's window
/// comes back to the front: an unchanged Capture One catalog costs a file stat.
const REFRESH_EVERY: Duration = Duration::from_secs(300);
/// Focus can come and go quickly (dialogs): no more than one look this often.
const REFRESH_AT_MOST: Duration = Duration::from_secs(10);

/// Reads the source when it changed, so its changes are seen (and logged) near when
/// they were made. Skipped while another read or a write is under way.
fn refresh(app: &AppHandle) {
    let Ok(s) = app.state::<App>().session() else { return };
    {
        let mut last = s.last_refresh.lock().unwrap();
        if last.elapsed() < REFRESH_AT_MOST {
            return;
        }
        *last = Instant::now();
    }
    let _one = match s.syncing.try_lock() {
        Ok(one) => one,
        Err(TryLockError::WouldBlock) => return,
        Err(TryLockError::Poisoned(e)) => e.into_inner(),
    };
    if let Err(e) = sync_held(app, &s, false, Origin::Source) {
        let _ = app.emit("sync-failed", e);
    }
}

fn refresh_loop(app: AppHandle) {
    loop {
        std::thread::sleep(REFRESH_EVERY);
        refresh(&app);
    }
}

/// One photo's entries in the change log, newest first, with what the albums they name
/// were called when last seen (for albums since deleted, or made here and since written).
#[derive(Serialize)]
struct PhotoHistory {
    events: Vec<c1index::EventView>,
    albums: BTreeMap<String, String>,
    /// When History was off: entries from then kept nothing of what they replaced, and a
    /// keyword tree change or album delete from then is not in a photo's history.
    history_off: Vec<c1index::HistoryOff>,
}

#[tauri::command]
async fn photo_history(app: State<'_, App>, row: u64) -> Result<PhotoHistory, String> {
    let s = app.session()?;
    let Some(id) = s.library.photo_id(row) else {
        return Ok(PhotoHistory { events: Vec::new(), albums: BTreeMap::new(), history_off: Vec::new() });
    };
    let events = s.library.photo_events(&id).map_err(text)?;
    let albums = s.library.album_names(&events).map_err(text)?.into_iter().map(|(id, name)| (id.0, name)).collect();
    let history_off = s.library.history_off().map_err(text)?;
    Ok(PhotoHistory { events: events.iter().map(c1index::EventView::of).collect(), albums, history_off })
}

/// The newest `limit` entries of the change log.
#[tauri::command]
async fn change_log(app: State<'_, App>, limit: usize) -> Result<Vec<c1index::EventView>, String> {
    let events = app.session()?.library.events(limit).map_err(text)?;
    Ok(events.iter().map(c1index::EventView::of).collect())
}

/// A page of the change log, newest first, in groups of one batch each.
#[tauri::command]
async fn change_log_page(app: State<'_, App>, query: c1index::LogQuery) -> Result<c1index::LogPage, String> {
    app.session()?.library.log_page(&query).map_err(text)
}

/// What changed between two times (inclusive, in ms; `to` of null is now).
#[tauri::command]
async fn history_compare(app: State<'_, App>, from: i64, to: Option<i64>) -> Result<c1index::Compare, String> {
    app.session()?.library.compare(from, to).map_err(text)
}

/// Photos per import day under `filter`: (local midnight in ms, count), in date order.
#[tauri::command]
async fn growth(app: State<'_, App>, filter: Filter) -> Result<Vec<(i64, usize)>, String> {
    app.session()?.library.growth(&filter).map_err(text)
}

/// Entries logged per day, and when the log starts.
#[tauri::command]
async fn log_days(app: State<'_, App>) -> Result<c1index::LogDays, String> {
    app.session()?.library.log_days().map_err(text)
}

/// An edit from the UI, applied to every photo it names.
#[derive(Deserialize)]
enum Edit {
    Rating(Option<u8>),
    Color(Option<ColorLabel>),
    /// Keyword paths, levels joined by `|`.
    AddKeywords(Vec<String>),
    RemoveKeywords(Vec<String>),
    /// Location fields by name (`city`, `state`, `country`, `country_code`, `location`);
    /// empty clears one. Fields not named keep each photo's own value.
    Place(BTreeMap<String, String>),
    /// IPTC text fields by name (`description`, `headline`, `title`, `creator`,
    /// `copyright`, `job_id`, `instructions`, `source`); empty clears one.
    Iptc(BTreeMap<String, String>),
    /// Latitude and longitude in degrees; `None` clears.
    Geo(Option<[f64; 2]>),
    /// Into an album, or out of it, by collection id.
    AddToAlbum(String),
    RemoveFromAlbum(String),
}

fn place_slot<'a>(place: &'a mut Place, name: &str) -> Option<&'a mut Option<String>> {
    Some(match name {
        "city" => &mut place.city,
        "state" => &mut place.state,
        "country" => &mut place.country,
        "country_code" => &mut place.country_code,
        "location" => &mut place.location,
        _ => return None,
    })
}

fn iptc_slot<'a>(iptc: &'a mut Iptc, name: &str) -> Option<&'a mut Option<String>> {
    Some(match name {
        "description" => &mut iptc.description,
        "headline" => &mut iptc.headline,
        "title" => &mut iptc.title,
        "creator" => &mut iptc.creator,
        "copyright" => &mut iptc.copyright,
        "job_id" => &mut iptc.job_id,
        "instructions" => &mut iptc.instructions,
        "source" => &mut iptc.source,
        _ => return None,
    })
}

/// A typed field: trimmed at both ends (a description keeps its inner lines), blank is
/// nothing, and a country code is upper case.
fn clean_text(name: &str, text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(if name == "country_code" { text.to_uppercase() } else { text.to_owned() })
}

fn geo_point(lat: f64, lon: f64) -> Result<GeoPoint, String> {
    if !(lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)) {
        return Err(format!("{lat}, {lon}: latitude is -90 to 90 and longitude -180 to 180"));
    }
    Ok(GeoPoint { lat, lon, alt: None, origin: GeoOrigin::Manual })
}

/// The op `edit` makes on one photo. The library takes each base from the photo; the
/// bases here are placeholders.
fn op_for(edit: &Edit, photo: &Photo) -> Result<Op, String> {
    Ok(match edit {
        Edit::Rating(value) => Op::SetRating { base: None, value: value.filter(|&v| v > 0) },
        Edit::Color(value) => Op::SetColor { base: None, value: *value },
        Edit::AddKeywords(paths) => Op::AddKeywords { paths: keyword_paths(paths)? },
        Edit::RemoveKeywords(paths) => Op::RemoveKeywords { paths: keyword_paths(paths)? },
        Edit::Place(fields) => {
            let mut value = photo.place.clone();
            for (name, text) in fields {
                *place_slot(&mut value, name).ok_or_else(|| format!("no location field {name:?}"))? =
                    clean_text(name, text);
            }
            Op::SetPlace { base: photo.place.clone(), value }
        }
        Edit::Iptc(fields) => {
            let mut value = photo.iptc.clone();
            for (name, text) in fields {
                *iptc_slot(&mut value, name).ok_or_else(|| format!("no caption field {name:?}"))? =
                    clean_text(name, text);
            }
            Op::SetIptc { base: photo.iptc.clone(), value }
        }
        Edit::Geo(value) => {
            Op::SetGeo { base: photo.geo, value: value.map(|[lat, lon]| geo_point(lat, lon)).transpose()? }
        }
        Edit::AddToAlbum(id) => Op::AddToCollection { collection: CollectionId(id.clone()) },
        Edit::RemoveFromAlbum(id) => Op::RemoveFromCollection { collection: CollectionId(id.clone()) },
    })
}

/// What pasting `fields` copied from `from` does to `to`. Keywords are added
/// (`keywords`) or made the same (`keywords_exact`: the ones `from` lacks go first).
/// `fill` only fills what `to` leaves empty: its own values stay, and keywords are added.
fn paste_ops(from: &Photo, fields: &[String], to: &Photo, fill: bool) -> Result<Vec<Op>, String> {
    let mut removes = Vec::new();
    let mut ops = Vec::new();
    let mut place = to.place.clone();
    let mut iptc = to.iptc.clone();
    for name in fields {
        match name.as_str() {
            "rating" if fill => {
                if to.rating.is_none() && from.rating.is_some() {
                    ops.push(Op::SetRating { base: to.rating, value: from.rating })
                }
            }
            "rating" => ops.push(Op::SetRating { base: to.rating, value: from.rating }),
            "color" if fill => {
                if to.color.is_none() && from.color.is_some() {
                    ops.push(Op::SetColor { base: to.color, value: from.color })
                }
            }
            "color" => ops.push(Op::SetColor { base: to.color, value: from.color }),
            "keywords" | "keywords_exact" => {
                if name == "keywords_exact" && !fill {
                    let gone: Vec<KeywordPath> =
                        to.keywords.iter().filter(|k| !from.keywords.contains(k)).cloned().collect();
                    if !gone.is_empty() {
                        removes.push(Op::RemoveKeywords { paths: gone });
                    }
                }
                let new: Vec<KeywordPath> = if fill {
                    from.keywords.iter().filter(|k| !to.keywords.contains(k)).cloned().collect()
                } else {
                    from.keywords.clone()
                };
                if !new.is_empty() {
                    ops.push(Op::AddKeywords { paths: new });
                }
            }
            "geo" if fill => {
                if to.geo.is_none() && from.geo.is_some() {
                    ops.push(Op::SetGeo { base: to.geo, value: from.geo })
                }
            }
            "geo" => ops.push(Op::SetGeo { base: to.geo, value: from.geo }),
            other => {
                let mut source_place = from.place.clone();
                let mut source_iptc = from.iptc.clone();
                let (slot, value) = if let Some(slot) = place_slot(&mut place, other) {
                    (slot, place_slot(&mut source_place, other).and_then(|v| v.take()))
                } else if let Some(slot) = iptc_slot(&mut iptc, other) {
                    (slot, iptc_slot(&mut source_iptc, other).and_then(|v| v.take()))
                } else {
                    return Err(format!("cannot copy {other:?}"));
                };
                if !fill || slot.is_none() {
                    *slot = value;
                }
            }
        }
    }
    if place != to.place {
        ops.push(Op::SetPlace { base: to.place.clone(), value: place });
    }
    if iptc != to.iptc {
        ops.push(Op::SetIptc { base: to.iptc.clone(), value: iptc });
    }
    // A removal takes the keywords under it too; the adds after it put back any that
    // `from` has.
    removes.extend(ops);
    Ok(removes)
}

/// `best | best2018 ` → `best|best2018`. Refuses names Capture One cannot store.
fn keyword_paths(paths: &[String]) -> Result<Vec<KeywordPath>, String> {
    paths
        .iter()
        .map(|path| {
            let levels: Vec<String> = path.split('|').map(|l| l.trim().to_owned()).collect();
            if levels.iter().any(|l| l.is_empty()) {
                return Err(format!("{path:?}: a keyword level is empty"));
            }
            if levels.iter().any(|l| l.contains(',')) {
                return Err(format!("{path:?}: keyword names cannot contain a comma"));
            }
            Ok(KeywordPath(levels))
        })
        .collect()
}

/// Edits photos in the library and queues the edits for the source. Returns the ops
/// that undo it.
#[tauri::command]
async fn edit(app: State<'_, App>, rows: Vec<u64>, edit: Edit) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    s.touched();
    s.library
        .edit_photos(&rows, |_, photo| Ok(vec![op_for(&edit, photo).map_err(anyhow::Error::msg)?]))
        .map_err(|e| format!("{e:#}"))
}

/// A place path's levels, `Poland|—|Kraków` → country, no state, city: a level the
/// index writes as `—` is no value.
fn place_levels(path: &str) -> Vec<Option<String>> {
    path.split('|').map(|l| Some(l.trim().to_owned()).filter(|l| !l.is_empty() && l != c1index::NO_PLACE)).collect()
}

/// `place` moved to `to`: its first `to.len()` levels (country, state, city, location)
/// become `to`'s; the levels below stay. `code` sets the ISO country code too.
fn moved_place(place: &Place, to: &[Option<String>], code: Option<&str>) -> Place {
    let mut out = place.clone();
    let levels = [&mut out.country, &mut out.state, &mut out.city, &mut out.location];
    for (level, value) in levels.into_iter().zip(to) {
        *level = value.clone();
    }
    if let Some(code) = code {
        out.country_code = clean_text("country_code", code);
    }
    out
}

/// Renames or joins a place: every photo at or under `from` moves to `to`, a path of the
/// same depth (an existing place joins them, a new one renames), as one edit. `code`,
/// when given, is the ISO code to set with a new country. Returns the ops that undo it.
#[tauri::command]
async fn move_place(
    app: State<'_, App>,
    from: String,
    to: String,
    code: Option<String>,
) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    let target = place_levels(&to);
    if target.len() != from.split('|').count() || target.len() > 4 {
        return Err(format!("{to:?} is not at the level of {from:?}"));
    }
    if target.last().is_none_or(|l| l.is_none()) {
        return Err("the new name cannot be empty".into());
    }
    let filter = Filter {
        place_under: Some(from),
        trashed: c1index::Trashed::Include,
        hidden: c1index::Hidden::Include,
        bin: c1index::Bin::Include,
        ..Filter::default()
    };
    let options = QueryOptions { sort: c1index::Sort::Row, ..QueryOptions::default() };
    let rows = s.library.query(&filter, &options).map_err(|e| format!("{e:#}"))?.rows;
    s.touched();
    s.library
        .edit_photos(&rows, |_, photo| {
            let value = moved_place(&photo.place, &target, code.as_deref());
            Ok(vec![Op::SetPlace { base: photo.place.clone(), value }])
        })
        .map_err(|e| format!("{e:#}"))
}

/// Photos in view with a country but no ISO country code, by country.
#[tauri::command]
async fn missing_country_codes(app: State<'_, App>, filter: Filter) -> Result<Vec<(String, Vec<u64>)>, String> {
    let s = app.session()?;
    let filter = Filter { no_country_code: true, ..filter };
    let options = QueryOptions { sort: c1index::Sort::Row, ..QueryOptions::default() };
    let rows = s.library.query(&filter, &options).map_err(|e| format!("{e:#}"))?.rows;
    let mut by_country: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for row in rows {
        if let Some(country) = s.library.photo(row).map_err(text)?.and_then(|p| p.place.country) {
            by_country.entry(country).or_default().push(row);
        }
    }
    Ok(by_country.into_iter().collect())
}

/// Sets the ISO country code on photos, as one edit (one undo). Each item: the photos
/// and their code.
#[tauri::command]
async fn set_country_codes(app: State<'_, App>, items: Vec<(Vec<u64>, String)>) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    let mut codes = BTreeMap::new();
    for (rows, code) in items {
        let code = clean_text("country_code", &code).ok_or("an empty country code")?;
        codes.extend(rows.into_iter().map(|row| (row, code.clone())));
    }
    let rows: Vec<u64> = codes.keys().copied().collect();
    s.touched();
    s.library
        .edit_photos(&rows, |row, photo| {
            let Some(code) = codes.get(&row) else { return Ok(vec![]) };
            let value = Place { country_code: Some(code.clone()), ..photo.place.clone() };
            Ok(vec![Op::SetPlace { base: photo.place.clone(), value }])
        })
        .map_err(|e| format!("{e:#}"))
}

/// Pastes fields copied from one photo (as it was when copied) onto others, as one
/// edit; `fill` only fills fields they leave empty. Returns the ops that undo it.
#[tauri::command]
async fn paste(
    app: State<'_, App>,
    rows: Vec<u64>,
    from: Photo,
    fields: Vec<String>,
    fill: Option<bool>,
) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    let fill = fill.unwrap_or(false);
    s.touched();
    s.library
        .edit_photos(&rows, |_, photo| paste_ops(&from, &fields, photo, fill).map_err(anyhow::Error::msg))
        .map_err(|e| format!("{e:#}"))
}

/// Applies ops returned by [`edit`] (undo and redo).
#[tauri::command]
async fn edit_ops(app: State<'_, App>, changes: Vec<(u64, Op)>) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    s.touched();
    s.library.edit(&changes).map_err(|e| format!("{e:#}"))
}

/// Every keyword in the catalog's tree with how many photos carry it, for completion.
#[tauri::command]
async fn keywords(app: State<'_, App>) -> Result<Vec<(String, u32)>, String> {
    let nodes = app.session()?.library.keywords().map_err(|e| format!("{e:#}"))?;
    Ok(nodes.into_iter().map(|n| (n.path.to_pipe(), n.uses)).collect())
}

/// A keyword tree change from the UI. Paths join levels with `|`.
#[derive(Deserialize)]
enum RemapRequest {
    Rename {
        path: String,
        name: String,
    },
    /// Several keywords into one, existing or new; with one, a replace.
    Join {
        from: Vec<String>,
        into: String,
    },
    /// Under a parent, existing or new; `None` is the top level.
    Group {
        paths: Vec<String>,
        under: Option<String>,
    },
    Ungroup {
        path: String,
    },
}

fn one_path(path: &str) -> Result<KeywordPath, String> {
    Ok(keyword_paths(&[path.to_owned()])?.remove(0))
}

/// Queues a keyword tree change; the library shows it at once. Returns its number, for
/// undo.
#[tauri::command]
async fn remap_keywords(app: State<'_, App>, request: RemapRequest) -> Result<u64, String> {
    let s = app.session()?;
    let remap = match request {
        RemapRequest::Rename { path, name } => {
            let name = name.trim();
            if name.is_empty() || name.contains(['|', ',']) {
                return Err(format!("{name:?}: a name cannot be empty or contain | or ,"));
            }
            KeywordRemap::rename(one_path(&path)?, name)
        }
        RemapRequest::Join { from, into } => KeywordRemap::join(keyword_paths(&from)?, one_path(&into)?),
        RemapRequest::Group { paths, under } => {
            let under = under.as_deref().map(one_path).transpose()?;
            let paths = keyword_paths(&paths)?;
            if let Some(u) = &under
                && paths.iter().any(|p| u.0.starts_with(&p.0))
            {
                return Err("a keyword cannot go under itself or under one of its own keywords".into());
            }
            KeywordRemap::group(paths, under)
        }
        RemapRequest::Ungroup { path } => {
            let group = one_path(&path)?;
            let children: Vec<KeywordPath> = s
                .library
                .keywords()
                .map_err(|e| format!("{e:#}"))?
                .into_iter()
                .map(|n| n.path)
                .filter(|p| p.0.len() == group.0.len() + 1 && p.0.starts_with(&group.0))
                .collect();
            if children.is_empty() {
                return Err(format!("{path:?} has no keywords under it"));
            }
            KeywordRemap::ungroup(group, children)
        }
    };
    s.touched();
    s.library.remap_keywords(remap).map_err(|e| format!("{e:#}"))
}

/// Drops a queued keyword tree change; the photos it touched are read from the source
/// again.
#[tauri::command]
async fn undo_remap(app: AppHandle, seq: u64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = app.state::<App>().session()?;
        s.touched();
        s.library.undo_remap(seq).map_err(|e| format!("{e:#}"))?;
        run_sync(&app).map(|_| ())
    })
    .await
    .map_err(text)?
}

/// Queued keyword tree changes: number and what they do.
#[tauri::command]
async fn remaps(app: State<'_, App>) -> Result<Vec<(u64, String)>, String> {
    let list = app.session()?.library.remaps().map_err(|e| format!("{e:#}"))?;
    Ok(list.into_iter().map(|(seq, q)| (seq, q.remap.label)).collect())
}

/// What the photos at `rows` have, for the details panel's batch editing: their keywords,
/// albums and places with how many carry each, and each editable field merged.
#[tauri::command]
async fn selection_summary(app: State<'_, App>, rows: Vec<u64>) -> Result<c1index::Selection, String> {
    app.session()?.library.selection(&rows).map_err(text)
}

/// Queues an album change; a new collection with an empty id gets one. Returns its
/// number (for undo) and the change as queued (for redo, which keeps the id).
#[tauri::command]
async fn change_collection(
    app: State<'_, App>,
    mut change: CollectionChange,
) -> Result<(u64, CollectionChange), String> {
    if let CollectionChange::Create { id, .. } = &mut change
        && id.0.is_empty()
    {
        *id = c1index::new_collection_id();
    }
    if let CollectionChange::Create { id, .. } = &change
        && id.local_uuid().is_none()
    {
        return Err(format!("{} is not an id for a new album", id.0));
    }
    let s = app.session()?;
    s.touched();
    let seq = s.library.change_collection(change.clone()).map_err(|e| format!("{e:#}"))?;
    Ok((seq, change))
}

#[tauri::command]
async fn undo_collection(app: State<'_, App>, seq: u64) -> Result<(), String> {
    let s = app.session()?;
    s.touched();
    s.library.undo_collection(seq).map_err(|e| format!("{e:#}"))
}

/// Queued album changes: number, what it does, and the collection it is about.
#[tauri::command]
async fn collection_changes(app: State<'_, App>) -> Result<Vec<(u64, String, String)>, String> {
    let list = app.session()?.library.collection_changes().map_err(|e| format!("{e:#}"))?;
    Ok(list.into_iter().map(|(seq, q)| (seq, q.label, q.change.id().0.clone())).collect())
}

/// How many photos carry any of these keywords (or keywords under them).
#[tauri::command]
async fn keyword_users(app: State<'_, App>, paths: Vec<String>) -> Result<usize, String> {
    let filter = Filter {
        keywords_under: paths,
        trashed: c1index::Trashed::Include,
        hidden: c1index::Hidden::Include,
        bin: c1index::Bin::Include,
        ..Filter::default()
    };
    let options = QueryOptions { limit: Some(0), ..QueryOptions::default() };
    Ok(app.session()?.library.query(&filter, &options).map_err(|e| format!("{e:#}"))?.total)
}

#[tauri::command]
async fn pending(app: State<'_, App>) -> Result<Vec<PendingEdit>, String> {
    app.session()?.library.pending().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn discard(app: State<'_, App>, keys: Vec<String>) -> Result<(), String> {
    let s = app.session()?;
    s.touched();
    s.library.discard(&keys).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn resolve(app: State<'_, App>, keys: Vec<String>, keep_mine: bool) -> Result<(), String> {
    let s = app.session()?;
    s.touched();
    s.library.resolve(&keys, keep_mine).map_err(|e| format!("{e:#}"))
}

/// What writing the queue to the source would do, after pulling its latest state.
#[derive(Serialize)]
struct WritePlan {
    status: WriteStatus,
    /// Why a read-only source is read-only.
    compatibility: Compatibility,
    /// Edits that would be written.
    apply: Vec<PendingEdit>,
    /// Edits the source already matches; writing drops them from the queue.
    unchanged: Vec<PendingEdit>,
    /// Fields changed in the source too: the user picks a side first.
    conflicts: Vec<PendingEdit>,
    /// Edits the source can no longer take (the photo is gone), with why.
    blocked: Vec<(PendingEdit, String)>,
    /// Keyword tree changes that would be written first: number and what they do.
    remaps: Vec<(u64, String)>,
    /// Keyword tree changes the source can no longer take, with why.
    blocked_remaps: Vec<(u64, String, String)>,
    /// Album changes, written after the keyword tree and before photo edits.
    collections: Vec<(u64, String)>,
    /// Album changes the source can no longer take, with why.
    blocked_collections: Vec<(u64, String, String)>,
}

/// The numbers of the queued structural changes a plan carries, settled once written.
struct Carried {
    remaps: Vec<u64>,
    collections: Vec<u64>,
}

/// The plan, what it shows, and the queued keyword tree and album changes it carries.
fn plan_writes(s: &Session) -> Result<(WritePlan, c1model::ApplyPlan, Carried), String> {
    let pending = s.library.pending().map_err(|e| format!("{e:#}"))?;
    let (conflicts, ready): (Vec<_>, Vec<_>) = pending.into_iter().partition(|p| p.pending.theirs.is_some());
    let changes: Vec<Change> = ready.iter().map(|p| p.pending.change.clone()).collect();
    let mut remaps = Vec::new();
    let mut blocked_remaps = Vec::new();
    let queued = s.library.remaps().map_err(|e| format!("{e:#}"))?;
    let whys = s.source.check_remaps(&queued.iter().map(|(_, q)| q.remap.clone()).collect::<Vec<_>>()).map_err(text)?;
    for ((seq, queued), why) in queued.into_iter().zip(whys) {
        match why {
            None => remaps.push((seq, queued.remap)),
            Some(why) => blocked_remaps.push((seq, queued.remap.label, why)),
        }
    }
    let ok: Vec<KeywordRemap> = remaps.iter().map(|(_, r)| r.clone()).collect();
    let mut collections: Vec<(u64, String, CollectionChange)> = Vec::new();
    let mut blocked_collections = Vec::new();
    for (seq, queued) in s.library.collection_changes().map_err(|e| format!("{e:#}"))? {
        let earlier: Vec<CollectionChange> = collections.iter().map(|(_, _, c)| c.clone()).collect();
        match s.source.check_collection(&earlier, &queued.change).map_err(text)? {
            None => collections.push((seq, queued.label, queued.change)),
            Some(why) => blocked_collections.push((seq, queued.label, why)),
        }
    }
    let albums: Vec<CollectionChange> = collections.iter().map(|(_, _, c)| c.clone()).collect();
    let plan = s.source.plan_after(&ok, &albums, &changes).map_err(text)?;
    let pick = |list: &[Change]| -> Vec<PendingEdit> {
        ready.iter().filter(|p| list.contains(&p.pending.change)).cloned().collect()
    };
    let blocked = plan
        .conflicts
        .iter()
        .filter_map(|c| {
            let p = ready.iter().find(|p| p.pending.change == c.change)?;
            Some((p.clone(), c.current.clone()))
        })
        .collect();
    let out = WritePlan {
        status: s.source.write_status(),
        compatibility: s.source.info().compatibility,
        apply: pick(&plan.apply),
        unchanged: pick(&plan.no_ops),
        conflicts,
        blocked,
        remaps: remaps.iter().map(|(seq, r)| (*seq, r.label.clone())).collect(),
        blocked_remaps,
        collections: collections.iter().map(|(seq, label, _)| (*seq, label.clone())).collect(),
        blocked_collections,
    };
    let carried = Carried {
        remaps: remaps.iter().map(|(seq, _)| *seq).collect(),
        collections: collections.iter().map(|(seq, _, _)| *seq).collect(),
    };
    Ok((out, plan, carried))
}

#[tauri::command]
async fn write_plan(app: AppHandle) -> Result<WritePlan, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_sync(&app)?;
        let s = app.state::<App>().session()?;
        let _one = s.syncing.lock().unwrap_or_else(|e| e.into_inner());
        Ok(plan_writes(&s)?.0)
    })
    .await
    .map_err(text)?
}

#[derive(Serialize)]
struct WriteReport {
    written: usize,
    backup: Option<String>,
    /// What went wrong without undoing the write (an XMP export that failed).
    warnings: Vec<String>,
}

enum Outcome {
    Written(WriteReport),
    /// Nothing written: these need the user first.
    Attention {
        conflicts: usize,
        blocked: usize,
    },
}

/// Writes the queue to the source (refused while Capture One runs), unless some edit
/// conflicts with a change made there. The source's state is pulled in before and after.
fn write_queue(app: &AppHandle) -> Result<Outcome, String> {
    run_sync(app)?;
    let s = app.state::<App>().session()?;
    // Held until the write is read back, so no refresh comes in between.
    let _one = s.syncing.lock().unwrap_or_else(|e| e.into_inner());
    let report = {
        let (summary, plan, carried) = plan_writes(&s)?;
        if !summary.conflicts.is_empty() {
            return Ok(Outcome::Attention { conflicts: summary.conflicts.len(), blocked: summary.blocked.len() });
        }
        let report = s.source.apply(&plan).map_err(|e| {
            s.log.event(&format!("Writing to {} failed: {e:#}", s.source.info().name), &[]);
            text(e)
        })?;
        s.log.event(
            &format!("Wrote {} edit(s) to {}", report.applied, s.source.info().name),
            &report.backup.iter().map(|b| format!("backup first: {b}")).collect::<Vec<_>>(),
        );
        for warning in &report.warnings {
            s.log.event(&format!("Warning: {warning}"), &[]);
        }
        let done: Vec<Change> = plan.apply.iter().chain(&plan.no_ops).cloned().collect();
        s.library.settle_remaps(&carried.remaps).map_err(|e| format!("{e:#}"))?;
        s.library.settle_collections(&carried.collections).map_err(|e| format!("{e:#}"))?;
        s.library.settle(&done).map_err(|e| format!("{e:#}"))?;
        s.library
            .log_written(&plan.apply, carried.collections.len(), carried.remaps.len())
            .map_err(|e| format!("{e:#}"))?;
        let blocked = summary.blocked.len() + summary.blocked_remaps.len() + summary.blocked_collections.len();
        if report.applied == 0 && blocked > 0 {
            return Ok(Outcome::Attention { conflicts: 0, blocked });
        }
        WriteReport { written: report.applied, backup: report.backup, warnings: report.warnings }
    };
    sync_held(app, &s, true, Origin::Source)?;
    Ok(Outcome::Written(report))
}

#[tauri::command]
async fn write_apply(app: AppHandle) -> Result<WriteReport, String> {
    app.state::<App>().session()?.touched();
    tauri::async_runtime::spawn_blocking(move || match write_queue(&app)? {
        Outcome::Written(report) => Ok(report),
        Outcome::Attention { conflicts, .. } => {
            Err(format!("{conflicts} edit(s) conflict with changes in the source; choose a side first"))
        }
    })
    .await
    .map_err(text)?
}

/// The backups made before writes, and whether the source can be written now (a
/// restore needs that as much as a write does).
#[derive(Serialize)]
struct Backups {
    list: Vec<BackupInfo>,
    status: WriteStatus,
}

#[tauri::command]
async fn backups(app: State<'_, App>) -> Result<Backups, String> {
    let s = app.session()?;
    Ok(Backups { list: s.source.backups().map_err(text)?, status: s.source.write_status() })
}

/// Puts a backup back as the catalog, then reads it in. Queued edits stay queued.
/// Returns the backup of the catalog it replaced.
#[tauri::command]
async fn restore_backup(app: AppHandle, id: String) -> Result<BackupInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = app.state::<App>().session()?;
        // The source's own changes read first, and the restored catalog read back before
        // anything else reads it: what the restore put back is logged as such.
        let _one = s.syncing.lock().unwrap_or_else(|e| e.into_inner());
        // A catalog that cannot be read is no reason not to restore one: the failure is
        // logged, and the restore goes ahead.
        let _ = sync_held(&app, &s, false, Origin::Source);
        let before = s.source.restore(&id).map_err(text)?;
        sync_held(&app, &s, true, Origin::Restore)?;
        Ok(before)
    })
    .await
    .map_err(text)?
}

/// What autosave is doing, for the footer.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
enum AutosaveState {
    Off,
    /// The queue is empty.
    Idle,
    /// Edits are queued; it writes once the user pauses.
    Pending,
    /// Another program has the source open; it writes once that program lets go.
    Waiting {
        by: String,
    },
    /// Some edits need a choice in the Write dialog first.
    Attention {
        conflicts: usize,
        blocked: usize,
    },
    Saved {
        written: usize,
        at: i64,
    },
    Failed {
        error: String,
    },
}

/// Edits wait this long after the last change before autosave writes them.
const AUTOSAVE_QUIET: Duration = Duration::from_secs(8);
const AUTOSAVE_TICK: Duration = Duration::from_secs(4);

fn report_autosave(app: &AppHandle, s: &Session, state: AutosaveState) {
    let mut last = s.autosave_state.lock().unwrap();
    // A save stays on show until there is something new to say.
    let keep_saved = matches!(*last, AutosaveState::Saved { .. }) && state == AutosaveState::Idle;
    if *last != state && !keep_saved {
        *last = state.clone();
        let _ = app.emit("autosave", &state);
    }
}

/// Runs for the life of the app: when autosave is on, writes the queue once the user
/// has paused and Capture One is closed.
fn autosave_loop(app: AppHandle) {
    loop {
        std::thread::sleep(AUTOSAVE_TICK);
        // Nothing open (yet): another source may be opened at any time.
        let Ok(s) = app.state::<App>().session() else { continue };
        if !s.autosave.load(Ordering::Relaxed) {
            report_autosave(&app, &s, AutosaveState::Off);
            continue;
        }
        let queued = s.library.pending().map(|p| p.len()).unwrap_or(0)
            + s.library.remaps().map(|r| r.len()).unwrap_or(0)
            + s.library.collection_changes().map(|c| c.len()).unwrap_or(0);
        if queued == 0 {
            report_autosave(&app, &s, AutosaveState::Idle);
            continue;
        }
        if s.last_edit.lock().unwrap().elapsed() < AUTOSAVE_QUIET {
            report_autosave(&app, &s, AutosaveState::Pending);
            continue;
        }
        match s.source.write_status() {
            WriteStatus::Ready => {}
            WriteStatus::Locked { by } => {
                report_autosave(&app, &s, AutosaveState::Waiting { by });
                continue;
            }
            WriteStatus::ReadOnly => {
                let compat = s.source.info().compatibility;
                let error =
                    format!("not written to this catalog ({}): {}", compat.format, compat.note.unwrap_or_default());
                report_autosave(&app, &s, AutosaveState::Failed { error });
                continue;
            }
        }
        let state = match write_queue(&app) {
            Ok(Outcome::Written(r)) => {
                let _ = app.emit("queue-written", &r);
                AutosaveState::Saved { written: r.written, at: c1index::edits::now_ms() }
            }
            Ok(Outcome::Attention { conflicts, blocked }) => AutosaveState::Attention { conflicts, blocked },
            Err(error) => AutosaveState::Failed { error },
        };
        // A failure is not retried until the queue changes, so it is not repeated every tick.
        if matches!(state, AutosaveState::Failed { .. } | AutosaveState::Attention { .. }) {
            report_autosave(&app, &s, state);
            let since = *s.last_edit.lock().unwrap();
            while *s.last_edit.lock().unwrap() == since && s.autosave.load(Ordering::Relaxed) {
                std::thread::sleep(AUTOSAVE_TICK);
            }
        } else {
            report_autosave(&app, &s, state);
        }
    }
}

#[tauri::command]
async fn autosave(app: State<'_, App>) -> Result<(bool, AutosaveState), String> {
    let s = app.session()?;
    Ok((s.autosave.load(Ordering::Relaxed), s.autosave_state.lock().unwrap().clone()))
}

#[tauri::command]
async fn set_autosave(app: State<'_, App>, on: bool) -> Result<(), String> {
    let s = app.session()?;
    // Saved first: a setting that could not be saved is not taken up either.
    s.save_settings(on)?;
    s.autosave.store(on, Ordering::Relaxed);
    // Turning it on waits for the usual pause, so the first write is never a surprise.
    s.touched();
    Ok(())
}

/// Why the open catalog's `settings.json` could not be read, if it could not.
#[tauri::command]
async fn settings_problem(app: State<'_, App>) -> Result<Option<String>, String> {
    Ok(app.session()?.settings_problem.clone())
}

/// Whether the History tab is on for the open catalog.
#[tauri::command]
async fn history_enabled(app: State<'_, App>) -> Result<bool, String> {
    Ok(app.session()?.library.history_on())
}

/// Turns the History tab on or off for the open catalog. Off, the change log goes on as
/// 0.5 kept it (without what changes replaced), and nothing reads it as history. Kept in
/// the library, so `c1cat` follows it too; nothing changes when it cannot be saved.
#[tauri::command]
async fn set_history(app: State<'_, App>, on: bool) -> Result<(), String> {
    app.session()?.library.set_history(on).map_err(text)
}

/// `/thumb/<row>` serves the source's own thumbnail (Capture One's): a JPEG with every edit, cut to the crop;
/// for a photo without one, one made here (from its preview or original) the first time.
/// `/preview/<row>` renders the large preview (~300 ms), framed alike, and sends it as JPEG.
fn serve(app: &AppHandle, path: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let mut parts = path.trim_start_matches('/').split('/');
    let tier = match parts.next() {
        Some("thumb") => ImageTier::Thumb,
        Some("preview") => ImageTier::Preview,
        _ => return Ok(None),
    };
    let Some(row) = parts.next().and_then(|r| r.parse::<u64>().ok()) else { return Ok(None) };
    let s = app.state::<App>().session().map_err(anyhow::Error::msg)?;
    let Some(id) = s.library.photo_id(row) else { return Ok(None) };
    let image = match tier {
        ImageTier::Thumb => s.thumbs.get_or_make(s.source.as_ref(), &id)?.map(|made| {
            if made.new {
                report_fill(app, &s, FillState::made_one);
            }
            made.image
        }),
        _ => s.source.image_or_make(&id, tier)?,
    };
    let Some(source) = image else { return Ok(None) };
    // A thumbnail without a crop is sent as stored; cutting one costs ~5 ms.
    if tier == ImageTier::Thumb && source.crop.is_none() {
        return Ok(Some(std::fs::read(&source.path)?));
    }
    let pixels = c1image::load(&source)?;
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&pixels)?;
    Ok(Some(out))
}

fn image_response(app: &AppHandle, path: &str) -> Response<Vec<u8>> {
    let (status, mime, body) = match serve(app, path) {
        Ok(Some(bytes)) => (StatusCode::OK, "image/jpeg", bytes),
        Ok(None) => (StatusCode::NOT_FOUND, "text/plain", b"no such image".to_vec()),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, "text/plain", format!("{e:#}").into_bytes()),
    };
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        // Rows are stable and pixels change only when Capture One re-renders. A missing
        // image is not kept: one may be made for it any moment.
        .header(header::CACHE_CONTROL, if status == StatusCode::OK { "max-age=600" } else { "no-store" })
        .body(body)
        .expect("a static response always builds")
}

/// Reads the source in the background, telling the window if that fails.
/// Opens the source the app was started with, then [`finish_opening`].
fn open_in_background(app: &AppHandle, spec: SourceSpec, remember: bool) {
    let app = app.clone();
    std::thread::spawn(move || {
        let session = std::panic::catch_unwind(|| open(&spec).map(Arc::new).map_err(|e| format!("{e:#}")))
            .unwrap_or_else(|_| Err("opening the library crashed".into()));
        let ready = session.is_ok();
        if ready && remember {
            let _ = Recent::remember(&spec);
        }
        *app.state::<App>().0.write().unwrap() = Current { spec: Some(spec), session };
        if ready {
            finish_opening(&app);
        } else {
            app.state::<Boot>().finish(&app);
        }
    });
}

/// Reads the source just opened. A library with no photos has nothing to use until its
/// first fill is in, so that is waited for under the start-up dialog; any other is
/// usable at once, and reads in the background.
fn finish_opening(app: &AppHandle) {
    let boot = app.state::<Boot>();
    if app.state::<App>().session().is_ok_and(|s| s.library.is_empty()) {
        if let Err(e) = run_sync(app) {
            let _ = app.emit("sync-failed", e);
        }
        boot.finish(app);
    } else {
        boot.finish(app);
        sync_in_background(app);
    }
}

fn sync_in_background(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = run_sync(&handle) {
            let _ = handle.emit("sync-failed", e);
        }
    });
}

/// A flag that a thread can sleep on until it is set.
#[derive(Default)]
struct Finish {
    set: Mutex<bool>,
    changed: Condvar,
}

impl Finish {
    fn set(&self) {
        *self.set.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.changed.notify_all();
    }

    /// Waits up to `time` for the flag; whether it is set.
    fn wait(&self, time: Duration) -> bool {
        let set = self.set.lock().unwrap_or_else(|e| e.into_inner());
        let (set, _) = self.changed.wait_timeout_while(set, time, |set| !*set).unwrap_or_else(|e| e.into_inner());
        *set
    }
}

/// Sets the flag when dropped.
struct FinishOnDrop<'a>(&'a Finish);

impl Drop for FinishOnDrop<'_> {
    fn drop(&mut self) {
        self.0.set();
    }
}

/// How often a running sync is asked how far the source has got.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Until `done`, tells the window how far the source's work has got (`source-progress`),
/// then that there is none.
fn report_progress(app: &AppHandle, s: &Session, done: &Finish) {
    let mut last = None;
    loop {
        let now = s.source.progress();
        if now != last {
            let _ = app.emit("source-progress", &now);
            last = now;
        }
        if done.wait(PROGRESS_EVERY) {
            break;
        }
    }
    if last.is_some() {
        let _ = app.emit("source-progress", None::<SourceProgress>);
    }
}

/// Asks the source to look at its files again, and reads it.
#[tauri::command]
async fn rescan(app: AppHandle) -> Result<(), String> {
    let s = app.state::<App>().session()?;
    s.source.rescan().map_err(text)?;
    s.log.event("Rescan asked for", &[]);
    sync_in_background(&app);
    Ok(())
}

/// Asks for a folder with the system's dialog.
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app.dialog().file().set_title("Photo folder").blocking_pick_folder();
        picked.map(|p| p.into_path().map(|p| p.display().to_string())).transpose().map_err(text)
    })
    .await
    .map_err(text)?
}

/// A source as the window lists it.
#[derive(Serialize)]
struct SourceEntry {
    #[serde(flatten)]
    spec: SourceSpec,
    /// The provider's name for its kind: "Capture One catalog".
    kind: &'static str,
    /// The file or folder name, for lists.
    name: String,
    /// Still there to be opened.
    available: bool,
}

fn entry(spec: &SourceSpec) -> Option<SourceEntry> {
    let provider = sources::provider(&spec.provider).ok()?;
    let path = std::path::Path::new(&spec.location);
    // Photo folders are named in their definition; its file is named for the app.
    let named = (spec.provider == source_folders::PROVIDER_ID)
        .then(|| source_folders::Definition::load(path).ok())
        .flatten()
        .map(|d| d.name.trim().to_owned())
        .filter(|n| !n.is_empty());
    let name = named.unwrap_or_else(|| {
        path.file_stem().or(path.file_name()).map_or(spec.location.clone(), |n| n.to_string_lossy().into())
    });
    Some(SourceEntry {
        spec: spec.clone(),
        kind: provider.name(),
        name,
        available: provider.recognise(&spec.location).is_some(),
    })
}

#[derive(Serialize)]
struct ProviderView {
    id: &'static str,
    name: &'static str,
    pick: Pick,
}

/// A kind of source, and whether the app offers it.
#[derive(Serialize)]
struct KindView {
    id: &'static str,
    name: &'static str,
    offered: bool,
}

#[derive(Serialize)]
struct SourcesView {
    current: Option<SourceEntry>,
    /// The open source's, when one is open.
    compatibility: Option<Compatibility>,
    /// Why nothing is open, when nothing is.
    error: Option<String>,
    recent: Vec<SourceEntry>,
    /// The kinds offered, to open.
    providers: Vec<ProviderView>,
    /// Every kind there is, to turn on or off.
    kinds: Vec<KindView>,
}

/// What is open, what was open before, and what kinds of source can be opened.
#[tauri::command]
async fn sources(app: State<'_, App>) -> Result<SourcesView, String> {
    let current = app.0.read().unwrap();
    let offered = Offered::load();
    Ok(SourcesView {
        current: current.spec.as_ref().and_then(entry),
        compatibility: current.session.as_ref().ok().map(|s| s.source.info().compatibility),
        error: current.session.as_ref().err().cloned(),
        recent: Recent::load()
            .list
            .iter()
            .filter(|s| Some(*s) != current.spec.as_ref() && offered.allows(&s.provider))
            .filter_map(entry)
            .collect(),
        providers: sources::providers()
            .iter()
            .filter(|p| offered.allows(p.id()))
            .map(|p| ProviderView { id: p.id(), name: p.name(), pick: p.pick() })
            .collect(),
        kinds: sources::providers()
            .iter()
            .map(|p| KindView { id: p.id(), name: p.name(), offered: offered.allows(p.id()) })
            .collect(),
    })
}

/// Offers these kinds of source from now on (at least one); the source open stays open.
#[tauri::command]
async fn set_providers(ids: Vec<String>) -> Result<(), String> {
    Offered::save(&ids)
}

/// Asks for a source of a provider's kind with the system's file or folder dialog.
#[tauri::command]
async fn pick_source(app: AppHandle, provider: String) -> Result<Option<String>, String> {
    let provider = sources::offered_providers()
        .into_iter()
        .find(|p| p.id() == provider)
        .ok_or_else(|| format!("{provider:?} sources are turned off"))?;
    let pick = provider.pick();
    tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file().set_title(provider.name());
        let picked = match pick {
            Pick::File { label, extensions } => dialog.add_filter(label, extensions).blocking_pick_file(),
            Pick::Folder => dialog.blocking_pick_folder(),
        };
        picked.map(|p| p.into_path().map(|p| p.display().to_string())).transpose().map_err(text)
    })
    .await
    .map_err(text)?
}

/// Opens another source in place of the current one: the window reloads after.
#[tauri::command]
async fn open_source(app: AppHandle, spec: SourceSpec) -> Result<(), String> {
    let named = Some(spec.provider.as_str()).filter(|p| !p.is_empty());
    // The source that is open (or failed to) opens again whatever kinds are offered: Try
    // again after a lock, say. Another one must be of a kind offered.
    let again = app.state::<App>().0.read().unwrap().spec.as_ref() == Some(&spec);
    let among = if again { sources::providers() } else { sources::offered_providers() };
    let spec = sources::spec_among(&among, &spec.location, named)?;
    {
        let state = app.state::<App>();
        let current = state.0.read().unwrap();
        if current.spec.as_ref() == Some(&spec) && current.session.is_ok() {
            return Ok(());
        }
    }
    let boot = app.state::<Boot>();
    boot.begin(&app);
    let opened = {
        let spec = spec.clone();
        tauri::async_runtime::spawn_blocking(move || open(&spec).map_err(|e| format!("{e:#}")))
            .await
            .map_err(text)
            .and_then(|r| r)
    };
    let opened = match opened {
        Ok(opened) => opened,
        Err(e) => {
            boot.finish(&app);
            return Err(e);
        }
    };
    // The old session closes once the last task using it is done with it; its analysis
    // stops after the batch it is on.
    if let Ok(old) = &app.state::<App>().0.read().unwrap().session {
        old.analysis_stop.store(true, Ordering::SeqCst);
        old.fill_stop.store(true, Ordering::SeqCst);
    }
    *app.state::<App>().0.write().unwrap() = Current { spec: Some(spec.clone()), session: Ok(Arc::new(opened)) };
    let remembered = Recent::remember(&spec);
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || finish_opening(&handle));
    remembered
}

/// Takes a source off the recent list.
#[tauri::command]
async fn forget_source(spec: SourceSpec) -> Result<(), String> {
    Recent::forget(&spec)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let args: Vec<String> = std::env::args().skip(1).collect();
            app.manage(Boot::default());
            // Opening can take a while (an index to rebuild, a first fill to read): the
            // window is up meanwhile, and says how far it is.
            match sources::startup(&args) {
                Ok((spec, remember)) => {
                    app.manage(App(RwLock::new(Current { spec: Some(spec.clone()), session: Err(STARTING.into()) })));
                    app.state::<Boot>().begin(app.handle());
                    open_in_background(app.handle(), spec, remember);
                }
                Err(e) => {
                    app.manage(App(RwLock::new(Current { spec: None, session: Err(e) })));
                }
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || autosave_loop(handle));
            let handle = app.handle().clone();
            std::thread::spawn(move || refresh_loop(handle));
            Ok(())
        })
        // Back from another program (Capture One): show what changed there.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(true) = event {
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn_blocking(move || refresh(&app));
            }
        })
        .register_asynchronous_uri_scheme_protocol("c1img", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_owned();
            tauri::async_runtime::spawn_blocking(move || responder.respond(image_response(&app, &path)));
        })
        .invoke_handler(tauri::generate_handler![
            sources,
            pick_source,
            pick_folder,
            folders::folders_new,
            folders::folders_load,
            folders::folders_save,
            folders::folders_check_store,
            rescan,
            open_source,
            forget_source,
            set_providers,
            status,
            startup_progress,
            query,
            smart_counts,
            trips,
            timeline,
            route,
            unresolved_places,
            set_place_points,
            promote_places,
            promote_rows,
            fill_plan,
            fill_places,
            neighbour_plan,
            neighbour_apply,
            refine_plan,
            refine_apply,
            set_trip_settings,
            class_settings,
            default_class_settings,
            set_class_settings,
            preview_classes,
            edit_trips,
            map,
            pick_gpx,
            gpx_plan,
            gpx_apply,
            groups,
            set_hidden,
            hidden_rows,
            hidden_of,
            set_binned,
            empty_bin,
            photo,
            collections,
            sync,
            edit,
            edit_ops,
            paste,
            missing_country_codes,
            move_place,
            set_country_codes,
            keywords,
            remap_keywords,
            undo_remap,
            remaps,
            keyword_users,
            change_collection,
            undo_collection,
            collection_changes,
            backups,
            restore_backup,
            look,
            analysis_state,
            fill_thumbnails,
            thumbs_state,
            open_log,
            pending,
            discard,
            resolve,
            write_plan,
            write_apply,
            autosave,
            set_autosave,
            history_enabled,
            set_history,
            settings_problem,
            selection_summary,
            photo_history,
            change_log,
            change_log_page,
            history_compare,
            growth,
            log_days,
            ai::ai_settings,
            ai::set_ai_settings,
            ai::set_ai_key,
            ai::ai_prompt,
            ai::ai_estimate,
            ai::start_describe,
            ai::stop_describe,
            ai::describe_state,
            ai::suggestions,
            ai::suggestion,
            ai::accept_suggestions,
            ai::set_suggestion_status
        ])
        .build(tauri::generate_context!())
        .expect("C1 Studio failed to start")
        .run(|app, event| {
            // The process ends without dropping the session: what the source writes lazily
            // (a folder source's image records) is made durable here, or it is lost and its
            // thumbnails made again at the next start.
            if let tauri::RunEvent::Exit = event
                && let Ok(s) = app.state::<App>().session()
            {
                let _ = s.source.flush();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use c1index::group_places;
    use c1model::{AssetId, EditInfo, Exif, FileInfo, PhotoId, SourceId};

    #[test]
    fn settings_read_whatever_build_wrote_them() {
        let old: Settings = serde_json::from_str(r#"{"autosave":true}"#).unwrap();
        assert!(old.autosave && old.history.is_none());
        let none: Settings = serde_json::from_str("{}").unwrap();
        assert!(!none.autosave && none.history.is_none());
        let off: Settings = serde_json::from_str(r#"{"autosave":false,"history":false}"#).unwrap();
        assert_eq!(off.history, Some(false));
        // History is no longer written: the library keeps it.
        let json = serde_json::to_string(&Settings { autosave: true, history: None }).unwrap();
        assert_eq!(json, r#"{"autosave":true}"#);
    }

    #[test]
    fn settings_are_written_whole_and_an_unreadable_file_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(read_settings(&path).1.is_none(), "none yet is no problem");
        write_settings(&path, &Settings { autosave: true, history: None }).unwrap();
        assert!(!dir.path().join("settings.json.tmp").exists());
        let (read, problem) = read_settings(&path);
        assert!(read.autosave && problem.is_none());

        std::fs::write(&path, br#"{"autosave": tr"#).unwrap();
        let (read, problem) = read_settings(&path);
        assert!(!read.autosave);
        assert!(problem.is_some_and(|p| p.contains("unreadable")));
        assert_eq!(std::fs::read(dir.path().join("settings.json.bad")).unwrap(), br#"{"autosave": tr"#);
        assert!(!path.exists());
    }

    fn photo(n: u32, keywords: &[&str], city: Option<&str>) -> Photo {
        Photo {
            id: PhotoId::new(&SourceId("t".into()), n.to_string()),
            asset: AssetId(n.to_string()),
            variant_index: 0,
            file: FileInfo::default(),
            captured_at: None,
            imported_at: None,
            exif: Exif::default(),
            rating: Some(n as u8),
            color: None,
            keywords: keywords.iter().map(|k| KeywordPath::from_pipe(k)).collect(),
            iptc: Iptc::default(),
            place: Place { city: city.map(str::to_owned), country: Some("Poland".into()), ..Place::default() },
            geo: None,
            collections: vec![],
            edit: EditInfo::default(),
            trashed: false,
        }
    }

    fn fields(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn pasting_takes_only_the_chosen_fields() {
        let from = photo(4, &["street"], Some("Kraków"));
        let to = Photo { place: Place { country: Some("Japan".into()), ..Place::default() }, ..photo(1, &[], None) };
        let ops = paste_ops(&from, &fields(&["rating", "city"]), &to, false).unwrap();
        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0], Op::SetRating { value: Some(4), .. }));
        let Op::SetPlace { value, .. } = &ops[1] else { panic!("{ops:?}") };
        // The city comes over; their country stays.
        assert_eq!((value.city.as_deref(), value.country.as_deref()), (Some("Kraków"), Some("Japan")));
        // Copying an empty field clears it.
        let ops = paste_ops(&photo(4, &[], None), &fields(&["city"]), &photo(1, &[], Some("Oslo")), false).unwrap();
        let Op::SetPlace { value, .. } = &ops[0] else { panic!("{ops:?}") };
        assert_eq!(value.city, None);
    }

    #[test]
    fn pasting_a_location_onto_its_own_source_changes_nothing() {
        let all = ["country", "country_code", "state", "city", "location", "geo"];
        let from = Photo { geo: Some(geo_point(50.06, 19.94).unwrap()), ..photo(1, &[], Some("Kraków")) };
        // The source among the targets: no place change (the library skips the same GPS).
        let own = paste_ops(&from, &fields(&all), &from, false).unwrap();
        assert!(own.iter().all(|op| matches!(op, Op::SetGeo { base, value } if base == value)));
        // Another photo gets the place and the GPS.
        let ops = paste_ops(&from, &fields(&all), &photo(2, &[], None), false).unwrap();
        assert!(ops.iter().any(|op| matches!(op, Op::SetGeo { value: Some(_), .. })));
        assert!(
            ops.iter().any(|op| matches!(op, Op::SetPlace { value, .. } if value.city.as_deref() == Some("Kraków")))
        );
    }

    #[test]
    fn places_group_whole_and_most_used_first() {
        let city = |c: Option<&str>| photo(1, &[], c).place;
        let groups = group_places(vec![
            (1, city(Some("Kraków"))),
            (2, Place::default()),
            (3, city(Some("Kraków"))),
            (4, city(None)),
            // Only spaces: no location, as the index has it.
            (5, Place { city: Some("  ".into()), ..Place::default() }),
            (6, city(Some("Kraków"))),
        ]);
        let shape: Vec<_> =
            groups.iter().map(|(p, rows)| (p.city.as_deref(), p.country.as_deref(), rows.clone())).collect();
        assert_eq!(shape[0], (Some("Kraków"), Some("Poland"), vec![1, 3, 6]));
        assert_eq!(groups.len(), 3);
        assert!(groups.iter().any(|(p, rows)| *p == Place::default() && rows == &[2, 5]));
    }

    #[test]
    fn keywords_are_added_or_made_the_same() {
        let from = photo(1, &["street", "city|night"], None);
        let to = photo(2, &["street", "b&w"], None);
        let add = paste_ops(&from, &fields(&["keywords"]), &to, false).unwrap();
        assert!(matches!(&add[..], [Op::AddKeywords { .. }]));
        let exact = paste_ops(&from, &fields(&["keywords_exact"]), &to, false).unwrap();
        // The removal first: the adds after it put back anything it takes under it.
        let [Op::RemoveKeywords { paths: gone }, Op::AddKeywords { paths: added }] = &exact[..] else {
            panic!("{exact:?}")
        };
        assert_eq!(gone, &[KeywordPath::from_pipe("b&w")]);
        assert_eq!(added.len(), 2);
        assert!(paste_ops(&from, &fields(&["albums"]), &to, false).is_err());
    }

    #[test]
    fn filling_keeps_what_the_target_has() {
        let from =
            Photo { geo: Some(geo_point(50.0, 19.9).unwrap()), ..photo(4, &["street", "night"], Some("Kraków")) };
        let to =
            Photo { place: Place { city: Some("Oslo".into()), ..Place::default() }, ..photo(1, &["street"], None) };
        let all = fields(&["rating", "color", "keywords_exact", "city", "country", "geo"]);
        let ops = paste_ops(&from, &all, &to, true).unwrap();
        // Their rating stays; the colour is empty on both; keywords only gain.
        let [Op::AddKeywords { paths }, Op::SetGeo { value: Some(_), .. }, Op::SetPlace { value, .. }] = &ops[..]
        else {
            panic!("{ops:?}")
        };
        assert_eq!(paths, &[KeywordPath::from_pipe("night")]);
        assert_eq!((value.city.as_deref(), value.country.as_deref()), (Some("Oslo"), Some("Poland")));
        // An unrated target takes the rating; an empty source never clears.
        let unrated = Photo { rating: None, ..to.clone() };
        assert!(matches!(
            paste_ops(&from, &fields(&["rating"]), &unrated, true).unwrap()[..],
            [Op::SetRating { value: Some(4), .. }]
        ));
        assert!(paste_ops(&photo(4, &[], None), &fields(&["city", "geo"]), &to, true).unwrap().is_empty());
    }

    #[test]
    fn a_moved_place_takes_the_new_levels_and_keeps_the_ones_below() {
        let krakow = Place {
            country: Some("Poland".into()),
            city: Some("Krakow".into()),
            location: Some("Rynek".into()),
            country_code: Some("PL".into()),
            ..Place::default()
        };
        // Poland|—|Krakow joins Poland|Malopolska|Kraków: the state is set, Rynek stays.
        let moved = moved_place(&krakow, &place_levels("Poland|Malopolska|Kraków"), None);
        assert_eq!(moved.state.as_deref(), Some("Malopolska"));
        assert_eq!(moved.city.as_deref(), Some("Kraków"));
        assert_eq!(moved.location.as_deref(), Some("Rynek"));
        assert_eq!(moved.country_code.as_deref(), Some("PL"));
        // A country renamed with its code.
        let usa = Place { country: Some("USA".into()), city: Some("New York".into()), ..Place::default() };
        let renamed = moved_place(&usa, &place_levels("United States"), Some("us"));
        assert_eq!((renamed.country.as_deref(), renamed.country_code.as_deref()), (Some("United States"), Some("US")));
        assert_eq!(renamed.city.as_deref(), Some("New York"));
        // Into a place with no state: the state goes.
        let into = moved_place(&moved, &place_levels("Poland|—|Kraków"), None);
        assert_eq!(into.state, None);
    }

    #[test]
    fn the_thumbnail_footer_follows_runs_counts_and_tiles() {
        let progress = |done, total, made, failed| Progress { done, total, made, failed };
        let run = FillState::under_way(progress(10, 40, 8, 2));
        assert_eq!((run.running, run.missing), (true, Some(32)));
        // A run under way counts for itself.
        assert_eq!(run.clone().counted(5).missing, Some(32));
        assert_eq!(run.made_one().missing, Some(32));
        let done = FillState::finished(progress(40, 40, 30, 10), vec![("drive".into(), 10)]);
        assert_eq!((done.running, done.missing, done.failed), (false, Some(10), 10));
        // Counted again with the same photos left: still "Try again", with the reasons.
        let same = done.clone().counted(10);
        assert_eq!((same.total, same.failed, same.reasons.len()), (40, 10, 1));
        // New photos imported: "Make them" again, the last run forgotten.
        let more = done.clone().counted(25);
        assert_eq!((more.missing, more.total, more.failed), (Some(25), 0, 0));
        assert!(more.reasons.is_empty());
        // A tile made one of the failed ones (its drive is back).
        let tile = done.made_one();
        assert_eq!((tile.missing, tile.failed), (Some(9), 9));
        // Nothing counted yet: a tile changes nothing.
        assert_eq!(FillState::default().made_one().missing, None);
    }

    #[test]
    fn a_flag_is_lowered_even_when_its_work_panics() {
        let flag = AtomicBool::new(true);
        let result = std::panic::catch_unwind(|| {
            let _done = Lowers(&flag);
            panic!("work fell over");
        });
        assert!(result.is_err());
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[test]
    fn typed_place_fields_are_cleaned() {
        let edit = Edit::Place(BTreeMap::from([("country_code".into(), " pl ".into()), ("city".into(), "  ".into())]));
        let Op::SetPlace { value, .. } = op_for(&edit, &photo(1, &[], Some("Oslo"))).unwrap() else { panic!() };
        assert_eq!((value.country_code.as_deref(), value.city), (Some("PL"), None));
        assert!(op_for(&Edit::Geo(Some([91.0, 0.0])), &photo(1, &[], None)).is_err());
    }
}
