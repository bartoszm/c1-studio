//! The desktop app: the library behind Tauri commands, and pixels behind the `c1img:`
//! protocol (`c1img://localhost/thumb/<row>`, `c1img://localhost/preview/<row>`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use c1index::{Filter, Library, PendingEdit, QueryOptions, QueryResult, SyncReport};
use c1model::{Change, Collection, ColorLabel, ImageTier, KeywordPath, Op, Photo, PhotoSource, WriteStatus};
use serde::{Deserialize, Serialize};
use source_c1::{CaptureOne, data_dir, find_catalog};
use tauri::http::{Response, StatusCode, header};
use tauri::{AppHandle, Emitter, Manager, State};

struct Session {
    source: CaptureOne,
    library: Library,
    /// One sync or write to Capture One at a time.
    syncing: Mutex<()>,
    /// `settings.json` beside the library.
    settings_path: PathBuf,
    /// Write the queue to Capture One by itself (off unless the user turns it on).
    autosave: AtomicBool,
    /// When the queue last changed, so autosave waits until the user pauses.
    last_edit: Mutex<Instant>,
    /// What autosave last reported, sent to the window only when it changes.
    autosave_state: Mutex<AutosaveState>,
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

/// The open session, or why there is none (no catalog found, library unreadable).
struct App(Result<Arc<Session>, String>);

impl App {
    fn session(&self) -> Result<Arc<Session>, String> {
        self.0.clone()
    }
}

fn open() -> anyhow::Result<Session> {
    let catalog = find_catalog(None)?;
    let dir = data_dir(&catalog);
    let source = CaptureOne::open(&catalog, dir.join("work"))?;
    let library = Library::open(dir.join("library"))?;
    let settings_path = dir.join("settings.json");
    let settings: Settings =
        std::fs::read(&settings_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    Ok(Session {
        source,
        library,
        syncing: Mutex::new(()),
        settings_path,
        autosave: AtomicBool::new(settings.autosave),
        last_edit: Mutex::new(Instant::now()),
        autosave_state: Mutex::new(AutosaveState::Off),
    })
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
}

#[tauri::command]
async fn status(app: State<'_, App>) -> Result<Status, String> {
    let s = app.session()?;
    Ok(Status {
        catalog: s.source.catalog_path().display().to_string(),
        source: s.source.info().name,
        photos: s.library.len(),
        writes: format!("{:?}", s.source.write_status()),
    })
}

#[tauri::command]
async fn query(app: State<'_, App>, filter: Filter, options: QueryOptions) -> Result<QueryResult, String> {
    app.session()?.library.query(&filter, &options).map_err(|e| format!("{e:#}"))
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
    let report = s.library.sync(&s.source).map_err(|e| format!("{e:#}"))?;
    if report.full || report.added + report.updated + report.deleted > 0 {
        let _ = app.emit("library-changed", &report);
    }
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
    // The library takes each base from the photo; these are placeholders.
    let op = match edit {
        Edit::Rating(value) => Op::SetRating { base: None, value: value.filter(|&v| v > 0) },
        Edit::Color(value) => Op::SetColor { base: None, value },
        Edit::AddKeywords(paths) => Op::AddKeywords { paths: keyword_paths(&paths)? },
        Edit::RemoveKeywords(paths) => Op::RemoveKeywords { paths: keyword_paths(&paths)? },
    };
    let changes: Vec<(u64, Op)> = rows.into_iter().map(|row| (row, op.clone())).collect();
    let s = app.session()?;
    s.touched();
    s.library.edit(&changes).map_err(|e| format!("{e:#}"))
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
    /// Edits that would be written.
    apply: Vec<PendingEdit>,
    /// Edits Capture One already matches; writing drops them from the queue.
    unchanged: Vec<PendingEdit>,
    /// Fields changed in Capture One too: the user picks a side first.
    conflicts: Vec<PendingEdit>,
    /// Edits Capture One can no longer take (the photo is gone), with why.
    blocked: Vec<(PendingEdit, String)>,
}

fn plan_writes(s: &Session) -> Result<(WritePlan, c1model::ApplyPlan), String> {
    let pending = s.library.pending().map_err(|e| format!("{e:#}"))?;
    let (conflicts, ready): (Vec<_>, Vec<_>) = pending.into_iter().partition(|p| p.pending.theirs.is_some());
    let changes: Vec<Change> = ready.iter().map(|p| p.pending.change.clone()).collect();
    let plan = s.source.plan(&changes).map_err(text)?;
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
        apply: pick(&plan.apply),
        unchanged: pick(&plan.no_ops),
        conflicts,
        blocked,
    };
    Ok((out, plan))
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
        let (summary, plan) = plan_writes(&s)?;
        if !summary.conflicts.is_empty() {
            return Ok(Outcome::Attention { conflicts: summary.conflicts.len(), blocked: summary.blocked.len() });
        }
        let report = s.source.apply(&plan).map_err(text)?;
        let done: Vec<Change> = plan.apply.iter().chain(&plan.no_ops).cloned().collect();
        s.library.settle(&done).map_err(|e| format!("{e:#}"))?;
        if report.applied == 0 && !summary.blocked.is_empty() {
            return Ok(Outcome::Attention { conflicts: 0, blocked: summary.blocked.len() });
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
        let Ok(s) = app.state::<App>().session() else { return };
        if !s.autosave.load(Ordering::Relaxed) {
            report_autosave(&app, &s, AutosaveState::Off);
            continue;
        }
        let queued = s.library.pending().map(|p| p.len()).unwrap_or(0);
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
                let error = "this catalog's format is not one the app was checked against".to_owned();
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

/// `/thumb/<row>` serves Capture One's own thumbnail as stored: a JPEG with every edit.
/// `/preview/<row>` renders the large preview (~250 ms) and sends it as JPEG.
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
    let Some(source) = s.source.image(&id, tier)? else { return Ok(None) };
    if tier == ImageTier::Thumb {
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
        // Rows are stable and pixels change only when Capture One re-renders.
        .header(header::CACHE_CONTROL, "max-age=600")
        .body(body)
        .expect("a static response always builds")
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let session = open().map(Arc::new).map_err(|e| format!("{e:#}"));
            let ready = session.is_ok();
            app.manage(App(session));
            if ready {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(e) = run_sync(&handle) {
                        let _ = handle.emit("sync-failed", e);
                    }
                });
                let handle = app.handle().clone();
                std::thread::spawn(move || autosave_loop(handle));
            }
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("c1img", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_owned();
            tauri::async_runtime::spawn_blocking(move || responder.respond(image_response(&app, &path)));
        })
        .invoke_handler(tauri::generate_handler![
            status,
            query,
            smart_counts,
            photo,
            collections,
            sync,
            edit,
            edit_ops,
            keywords,
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
