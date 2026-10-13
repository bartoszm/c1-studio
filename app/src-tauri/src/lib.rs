//! The desktop app: the library behind Tauri commands, and pixels behind the `c1img:`
//! protocol (`c1img://localhost/thumb/<row>`, `c1img://localhost/preview/<row>`).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use c1index::thumbs::{Failure, Kind, Progress, Thumbs};
use c1index::{Filter, Library, PendingEdit, QueryOptions, QueryResult, SyncReport};
use c1model::provider::data_root;
use c1model::{
    BackupInfo, Change, Collection, CollectionChange, CollectionId, ColorLabel, Compatibility, GeoOrigin, GeoPoint,
    ImageTier, Iptc, KeywordPath, KeywordRemap, Op, Photo, PhotoSource, Pick, Place, SourceSpec, WriteStatus,
};
use events::Log;
use serde::{Deserialize, Serialize};
use sources::Recent;
use tauri::http::{Response, StatusCode, header};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

mod events;
mod sources;

struct Session {
    /// What is open: which provider, and where.
    spec: SourceSpec,
    source: Box<dyn PhotoSource>,
    library: Library,
    /// One sync or write to Capture One at a time.
    syncing: Mutex<()>,
    /// Why the last sync failed, until one succeeds: shown however late the window asks.
    sync_error: Mutex<Option<String>>,
    /// `settings.json` beside the library.
    settings_path: PathBuf,
    /// Write the queue to Capture One by itself (off unless the user turns it on).
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
    /// Thumbnails made here for photos Capture One has none for, and the run that makes
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

/// The app's own settings, kept per catalog.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    autosave: bool,
}

/// The source asked for and its session, or why there is none (nothing chosen yet, the
/// catalog is gone, the library is unreadable). Another source can be opened at any time.
struct Current {
    spec: Option<SourceSpec>,
    session: Result<Arc<Session>, String>,
}

struct App(RwLock<Current>);

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
    let settings: Settings =
        std::fs::read(&settings_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let session = Session {
        spec: spec.clone(),
        source,
        library,
        syncing: Mutex::new(()),
        sync_error: Mutex::new(None),
        settings_path,
        autosave: AtomicBool::new(settings.autosave),
        last_edit: Mutex::new(Instant::now()),
        autosave_state: Mutex::new(AutosaveState::Off),
        analysing: AtomicBool::new(false),
        analysis_stop: AtomicBool::new(false),
        analysis: Mutex::new(AnalysisState::default()),
        thumbs: Thumbs::new(dir.join("thumbs")),
        filling: AtomicBool::new(false),
        fill_stop: AtomicBool::new(false),
        fill: Mutex::new(FillState::default()),
        counting: AtomicBool::new(false),
        count_again: AtomicBool::new(false),
        log: Log::new(dir.join("events.log")),
    };
    session.log.event(&format!("Opened {}", spec.location), &[]);
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
    let s = app.state::<App>().session()?;
    if s.filling.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
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
    s.log.event(&format!("Making thumbnails for {} photos Capture One has none for", missing.len()), &[]);
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
    Ok(Some(LookView {
        shape: a.shape(),
        hues: a.hues(),
        light: a.light(),
        focus: a.focus(),
        duplicates: size(look.duplicates.clone())?,
        series: size(look.series.clone())?,
        analysis: look.analysis,
    }))
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
    s.library.edit(&changes).map_err(text_of)
}

/// Duplicate groups (or series) among the photos `filter` selects, biggest first.
#[tauri::command]
async fn groups(app: State<'_, App>, filter: Filter, series: bool) -> Result<Vec<c1index::Group>, String> {
    app.session()?.library.groups(&filter, series, 300).map_err(text)
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
    photos: u64,
    writes: String,
    sync_error: Option<String>,
    /// What made the source and how far the app is checked against it.
    compatibility: Compatibility,
}

#[tauri::command]
async fn status(app: State<'_, App>) -> Result<Status, String> {
    let s = app.session()?;
    Ok(Status {
        catalog: s.spec.location.clone(),
        source: s.source.info().name,
        photos: s.library.len(),
        writes: format!("{:?}", s.source.write_status()),
        sync_error: s.sync_error.lock().unwrap().clone(),
        compatibility: s.source.info().compatibility,
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

/// Smart album id → how many photos it holds under `filter` (its album pick aside).
#[tauri::command]
async fn smart_counts(app: State<'_, App>, filter: Filter) -> Result<Vec<(String, u64)>, String> {
    app.session()?.library.smart_counts(&filter).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn photo(app: State<'_, App>, row: u64) -> Result<Option<Photo>, String> {
    app.session()?.library.photo(row).map_err(text)
}

#[tauri::command]
async fn collections(app: State<'_, App>) -> Result<Vec<Collection>, String> {
    app.session()?.library.collections().map_err(text)
}

#[tauri::command]
async fn sync(app: AppHandle) -> Result<SyncReport, String> {
    tauri::async_runtime::spawn_blocking(move || run_sync(&app)).await.map_err(text)?
}

/// Pulls Capture One's changes in, and tells the window when anything moved.
fn run_sync(app: &AppHandle) -> Result<SyncReport, String> {
    let s = app.state::<App>().session()?;
    let _one = s.syncing.lock().unwrap_or_else(|e| e.into_inner());
    let report = s.library.sync(s.source.as_ref()).map_err(|e| format!("{e:#}"));
    let failed = report.as_ref().err().cloned();
    if let Some(e) = &failed
        && s.sync_error.lock().unwrap().as_ref() != Some(e)
    {
        s.log.event(&format!("Reading Capture One failed: {e}"), &[]);
    }
    *s.sync_error.lock().unwrap() = failed;
    let report = report?;
    if report.full || report.added + report.updated + report.deleted > 0 {
        s.log.event(
            &format!(
                "Read Capture One{}: {} added, {} updated, {} deleted",
                if report.full { " in full" } else { "" },
                report.added,
                report.updated,
                report.deleted
            ),
            &[],
        );
        let _ = app.emit("library-changed", &report);
    }
    // Previews may have been rendered, or crops changed: made thumbnails are checked
    // against their sources again, and photos that failed may succeed now.
    s.thumbs.forget();
    start_analysis(app);
    count_missing(app);
    Ok(report)
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

/// Edits photos in the library and queues the edits for Capture One. Returns the ops
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
    let filter = Filter { place_under: Some(from), trashed: c1index::Trashed::Include, ..Filter::default() };
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
    let mut codes = HashMap::new();
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

/// Drops a queued keyword tree change; the photos it touched are read from Capture One
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

/// The keywords on the photos at `rows`, with how many of them carry each: the
/// selection's keywords, on all of them or only on some.
#[tauri::command]
async fn keyword_counts(app: State<'_, App>, rows: Vec<u64>) -> Result<Vec<(String, u64)>, String> {
    let s = app.session()?;
    let mut counts = BTreeMap::<String, u64>::new();
    for row in rows {
        if let Some(photo) = s.library.photo(row).map_err(text)? {
            for path in &photo.keywords {
                *counts.entry(path.to_pipe()).or_default() += 1;
            }
        }
    }
    Ok(counts.into_iter().collect())
}

/// The places of the photos at `rows`, each with the photos that have it, the most used
/// first; photos with no location fields at all share the empty place.
#[tauri::command]
async fn place_counts(app: State<'_, App>, rows: Vec<u64>) -> Result<Vec<(Place, Vec<u64>)>, String> {
    let s = app.session()?;
    let mut places = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(photo) = s.library.photo(row).map_err(text)? {
            places.push((row, photo.place));
        }
    }
    Ok(group_places(places))
}

fn group_places(places: Vec<(u64, Place)>) -> Vec<(Place, Vec<u64>)> {
    type Key = [Option<String>; 5];
    let key = |p: &Place| -> Key {
        [p.country.clone(), p.country_code.clone(), p.state.clone(), p.city.clone(), p.location.clone()]
    };
    let mut groups = BTreeMap::<Key, (Place, Vec<u64>)>::new();
    for (row, place) in places {
        groups.entry(key(&place)).or_insert_with(|| (place, vec![])).1.push(row);
    }
    let mut out: Vec<_> = groups.into_values().collect();
    out.sort_by_key(|(_, rows)| std::cmp::Reverse(rows.len()));
    out
}

/// The albums the photos at `rows` are in, with how many of them each holds.
#[tauri::command]
async fn album_counts(app: State<'_, App>, rows: Vec<u64>) -> Result<Vec<(String, u64)>, String> {
    let s = app.session()?;
    let mut counts = BTreeMap::<String, u64>::new();
    for row in rows {
        if let Some(photo) = s.library.photo(row).map_err(text)? {
            for id in &photo.collections {
                *counts.entry(id.0.clone()).or_default() += 1;
            }
        }
    }
    Ok(counts.into_iter().collect())
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
    let filter = Filter { keywords_under: paths, trashed: c1index::Trashed::Include, ..Filter::default() };
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

/// What writing the queue to Capture One would do, after pulling its latest state.
#[derive(Serialize)]
struct WritePlan {
    status: WriteStatus,
    /// Why a read-only source is read-only.
    compatibility: Compatibility,
    /// Edits that would be written.
    apply: Vec<PendingEdit>,
    /// Edits Capture One already matches; writing drops them from the queue.
    unchanged: Vec<PendingEdit>,
    /// Fields changed in Capture One too: the user picks a side first.
    conflicts: Vec<PendingEdit>,
    /// Edits Capture One can no longer take (the photo is gone), with why.
    blocked: Vec<(PendingEdit, String)>,
    /// Keyword tree changes that would be written first: number and what they do.
    remaps: Vec<(u64, String)>,
    /// Keyword tree changes Capture One can no longer take, with why.
    blocked_remaps: Vec<(u64, String, String)>,
    /// Album changes, written after the keyword tree and before photo edits.
    collections: Vec<(u64, String)>,
    /// Album changes Capture One can no longer take, with why.
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
    for (seq, queued) in s.library.remaps().map_err(|e| format!("{e:#}"))? {
        let earlier: Vec<KeywordRemap> = remaps.iter().map(|(_, r): &(u64, KeywordRemap)| r.clone()).collect();
        match s.source.check_remap(&earlier, &queued.remap).map_err(text)? {
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
}

enum Outcome {
    Written(WriteReport),
    /// Nothing written: these need the user first.
    Attention {
        conflicts: usize,
        blocked: usize,
    },
}

/// Writes the queue to Capture One (refused while it runs), unless some edit conflicts
/// with a change made there. Capture One's state is pulled in before and after.
fn write_queue(app: &AppHandle) -> Result<Outcome, String> {
    run_sync(app)?;
    let s = app.state::<App>().session()?;
    let report = {
        let _one = s.syncing.lock().unwrap_or_else(|e| e.into_inner());
        let (summary, plan, carried) = plan_writes(&s)?;
        if !summary.conflicts.is_empty() {
            return Ok(Outcome::Attention { conflicts: summary.conflicts.len(), blocked: summary.blocked.len() });
        }
        let report = s.source.apply(&plan).map_err(|e| {
            s.log.event(&format!("Writing to Capture One failed: {e:#}"), &[]);
            text(e)
        })?;
        s.log.event(
            &format!("Wrote {} edit(s) to Capture One", report.applied),
            &report.backup.iter().map(|b| format!("backup first: {b}")).collect::<Vec<_>>(),
        );
        let done: Vec<Change> = plan.apply.iter().chain(&plan.no_ops).cloned().collect();
        s.library.settle_remaps(&carried.remaps).map_err(|e| format!("{e:#}"))?;
        s.library.settle_collections(&carried.collections).map_err(|e| format!("{e:#}"))?;
        s.library.settle(&done).map_err(|e| format!("{e:#}"))?;
        let blocked = summary.blocked.len() + summary.blocked_remaps.len() + summary.blocked_collections.len();
        if report.applied == 0 && blocked > 0 {
            return Ok(Outcome::Attention { conflicts: 0, blocked });
        }
        WriteReport { written: report.applied, backup: report.backup }
    };
    run_sync(app)?;
    Ok(Outcome::Written(report))
}

#[tauri::command]
async fn write_apply(app: AppHandle) -> Result<WriteReport, String> {
    app.state::<App>().session()?.touched();
    tauri::async_runtime::spawn_blocking(move || match write_queue(&app)? {
        Outcome::Written(report) => Ok(report),
        Outcome::Attention { conflicts, .. } => {
            Err(format!("{conflicts} edit(s) conflict with Capture One; choose a side first"))
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
        let before = {
            let _one = s.syncing.lock().unwrap_or_else(|e| e.into_inner());
            s.source.restore(&id).map_err(text)?
        };
        run_sync(&app)?;
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
    /// Capture One (or another program) has the catalog; it writes once it is closed.
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
    s.autosave.store(on, Ordering::Relaxed);
    // Turning it on waits for the usual pause, so the first write is never a surprise.
    s.touched();
    let bytes = serde_json::to_vec_pretty(&Settings { autosave: on }).map_err(text)?;
    std::fs::write(&s.settings_path, bytes).map_err(text)
}

/// `/thumb/<row>` serves Capture One's own thumbnail: a JPEG with every edit, cut to the crop;
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
        _ => s.source.image(&id, tier)?,
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
fn sync_in_background(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = run_sync(&handle) {
            let _ = handle.emit("sync-failed", e);
        }
    });
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
    let name = path.file_stem().or(path.file_name()).map_or(spec.location.clone(), |n| n.to_string_lossy().into());
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

#[derive(Serialize)]
struct SourcesView {
    current: Option<SourceEntry>,
    /// The open source's, when one is open.
    compatibility: Option<Compatibility>,
    /// Why nothing is open, when nothing is.
    error: Option<String>,
    recent: Vec<SourceEntry>,
    providers: Vec<ProviderView>,
}

/// What is open, what was open before, and what kinds of source can be opened.
#[tauri::command]
async fn sources(app: State<'_, App>) -> Result<SourcesView, String> {
    let current = app.0.read().unwrap();
    Ok(SourcesView {
        current: current.spec.as_ref().and_then(entry),
        compatibility: current.session.as_ref().ok().map(|s| s.source.info().compatibility),
        error: current.session.as_ref().err().cloned(),
        recent: Recent::load().list.iter().filter(|s| Some(*s) != current.spec.as_ref()).filter_map(entry).collect(),
        providers: sources::providers()
            .iter()
            .map(|p| ProviderView { id: p.id(), name: p.name(), pick: p.pick() })
            .collect(),
    })
}

/// Asks for a source of a provider's kind with the system's file or folder dialog.
#[tauri::command]
async fn pick_source(app: AppHandle, provider: String) -> Result<Option<String>, String> {
    let provider = sources::provider(&provider)?;
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
    let spec = sources::spec_for(&spec.location, named)?;
    {
        let state = app.state::<App>();
        let current = state.0.read().unwrap();
        if current.spec.as_ref() == Some(&spec) && current.session.is_ok() {
            return Ok(());
        }
    }
    let opened = {
        let spec = spec.clone();
        tauri::async_runtime::spawn_blocking(move || open(&spec).map_err(|e| format!("{e:#}")))
            .await
            .map_err(text)??
    };
    // The old session closes once the last task using it is done with it; its analysis
    // stops after the batch it is on.
    if let Ok(old) = &app.state::<App>().0.read().unwrap().session {
        old.analysis_stop.store(true, Ordering::SeqCst);
        old.fill_stop.store(true, Ordering::SeqCst);
    }
    *app.state::<App>().0.write().unwrap() = Current { spec: Some(spec.clone()), session: Ok(Arc::new(opened)) };
    Recent::remember(&spec)?;
    sync_in_background(&app);
    Ok(())
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
            let current = match sources::startup(&args) {
                Ok((spec, remember)) => {
                    let session = open(&spec).map(Arc::new).map_err(|e| format!("{e:#}"));
                    if session.is_ok() && remember {
                        let _ = Recent::remember(&spec);
                    }
                    Current { spec: Some(spec), session }
                }
                Err(e) => Current { spec: None, session: Err(e) },
            };
            let ready = current.session.is_ok();
            app.manage(App(RwLock::new(current)));
            if ready {
                sync_in_background(app.handle());
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || autosave_loop(handle));
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("c1img", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_owned();
            tauri::async_runtime::spawn_blocking(move || responder.respond(image_response(&app, &path)));
        })
        .invoke_handler(tauri::generate_handler![
            sources,
            pick_source,
            open_source,
            forget_source,
            status,
            query,
            smart_counts,
            map,
            pick_gpx,
            gpx_plan,
            gpx_apply,
            groups,
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
            album_counts,
            place_counts,
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
            keyword_counts,
            pending,
            discard,
            resolve,
            write_plan,
            write_apply,
            autosave,
            set_autosave
        ])
        .run(tauri::generate_context!())
        .expect("C1 Studio failed to start");
}

#[cfg(test)]
mod tests {
    use super::*;
    use c1model::{AssetId, EditInfo, Exif, FileInfo, PhotoId, SourceId};

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
        ]);
        let shape: Vec<_> =
            groups.iter().map(|(p, rows)| (p.city.as_deref(), p.country.as_deref(), rows.clone())).collect();
        assert_eq!(shape[0], (Some("Kraków"), Some("Poland"), vec![1, 3]));
        assert_eq!(groups.len(), 3);
        assert!(groups.iter().any(|(p, rows)| *p == Place::default() && rows == &[2]));
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
