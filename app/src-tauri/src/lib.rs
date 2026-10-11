//! The desktop app: the library behind Tauri commands, and pixels behind the `c1img:`
//! protocol (`c1img://localhost/thumb/<row>`, `c1img://localhost/preview/<row>`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use c1index::{Filter, Library, PendingEdit, QueryOptions, QueryResult, SyncReport};
use c1model::{
    Change, Collection, ColorLabel, GeoOrigin, GeoPoint, ImageTier, Iptc, KeywordPath, KeywordRemap, Op, Photo,
    PhotoSource, Place, WriteStatus,
};
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
    /// Location fields by name (`city`, `state`, `country`, `country_code`, `location`);
    /// empty clears one. Fields not named keep each photo's own value.
    Place(BTreeMap<String, String>),
    /// Caption fields by name (`description`, `headline`, `title`, `creator`,
    /// `copyright`); empty clears one.
    Iptc(BTreeMap<String, String>),
    /// Latitude and longitude in degrees; `None` clears.
    Geo(Option<[f64; 2]>),
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
        _ => return None,
    })
}

/// A typed field: trimmed (but a caption keeps its lines), blank is nothing, and a
/// country code is upper case.
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
    })
}

/// What pasting `fields` copied from `from` does to `to`. Keywords are added
/// (`keywords`) or made the same (`keywords_exact`: the ones `from` lacks go first).
fn paste_ops(from: &Photo, fields: &[String], to: &Photo) -> Result<Vec<Op>, String> {
    let mut removes = Vec::new();
    let mut ops = Vec::new();
    let mut place = to.place.clone();
    let mut iptc = to.iptc.clone();
    for name in fields {
        match name.as_str() {
            "rating" => ops.push(Op::SetRating { base: to.rating, value: from.rating }),
            "color" => ops.push(Op::SetColor { base: to.color, value: from.color }),
            "keywords" | "keywords_exact" => {
                if name == "keywords_exact" {
                    let gone: Vec<KeywordPath> =
                        to.keywords.iter().filter(|k| !from.keywords.contains(k)).cloned().collect();
                    if !gone.is_empty() {
                        removes.push(Op::RemoveKeywords { paths: gone });
                    }
                }
                if !from.keywords.is_empty() {
                    ops.push(Op::AddKeywords { paths: from.keywords.clone() });
                }
            }
            "geo" => ops.push(Op::SetGeo { base: to.geo, value: from.geo }),
            other => {
                let mut source_place = from.place.clone();
                let mut source_iptc = from.iptc.clone();
                if let Some(slot) = place_slot(&mut place, other) {
                    *slot = place_slot(&mut source_place, other).and_then(|v| v.take());
                } else if let Some(slot) = iptc_slot(&mut iptc, other) {
                    *slot = iptc_slot(&mut source_iptc, other).and_then(|v| v.take());
                } else {
                    return Err(format!("cannot copy {other:?}"));
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
    let mut changes = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(photo) = s.library.photo(row).map_err(text)? {
            changes.push((row, op_for(&edit, &photo)?));
        }
    }
    s.touched();
    s.library.edit(&changes).map_err(|e| format!("{e:#}"))
}

/// Pastes fields copied from one photo (as it was when copied) onto others, as one
/// edit. Returns the ops that undo it.
#[tauri::command]
async fn paste(
    app: State<'_, App>,
    rows: Vec<u64>,
    from: Photo,
    fields: Vec<String>,
) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    let mut changes = Vec::new();
    for row in rows {
        if let Some(photo) = s.library.photo(row).map_err(text)? {
            changes.extend(paste_ops(&from, &fields, &photo)?.into_iter().map(|op| (row, op)));
        }
    }
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
}

/// The plan, what it shows, and the numbers of the keyword tree changes it carries.
fn plan_writes(s: &Session) -> Result<(WritePlan, c1model::ApplyPlan, Vec<u64>), String> {
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
    let plan = s.source.plan_after(&ok, &changes).map_err(text)?;
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
        remaps: remaps.iter().map(|(seq, r)| (*seq, r.label.clone())).collect(),
        blocked_remaps,
    };
    let seqs = remaps.iter().map(|(seq, _)| *seq).collect();
    Ok((out, plan, seqs))
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
        let (summary, plan, remaps) = plan_writes(&s)?;
        if !summary.conflicts.is_empty() {
            return Ok(Outcome::Attention { conflicts: summary.conflicts.len(), blocked: summary.blocked.len() });
        }
        let report = s.source.apply(&plan).map_err(text)?;
        let done: Vec<Change> = plan.apply.iter().chain(&plan.no_ops).cloned().collect();
        s.library.settle_remaps(&remaps).map_err(|e| format!("{e:#}"))?;
        s.library.settle(&done).map_err(|e| format!("{e:#}"))?;
        let blocked = summary.blocked.len() + summary.blocked_remaps.len();
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
        let queued =
            s.library.pending().map(|p| p.len()).unwrap_or(0) + s.library.remaps().map(|r| r.len()).unwrap_or(0);
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
            paste,
            keywords,
            remap_keywords,
            undo_remap,
            remaps,
            keyword_users,
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
        let ops = paste_ops(&from, &fields(&["rating", "city"]), &to).unwrap();
        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0], Op::SetRating { value: Some(4), .. }));
        let Op::SetPlace { value, .. } = &ops[1] else { panic!("{ops:?}") };
        // The city comes over; their country stays.
        assert_eq!((value.city.as_deref(), value.country.as_deref()), (Some("Kraków"), Some("Japan")));
        // Copying an empty field clears it.
        let ops = paste_ops(&photo(4, &[], None), &fields(&["city"]), &photo(1, &[], Some("Oslo"))).unwrap();
        let Op::SetPlace { value, .. } = &ops[0] else { panic!("{ops:?}") };
        assert_eq!(value.city, None);
    }

    #[test]
    fn keywords_are_added_or_made_the_same() {
        let from = photo(1, &["street", "city|night"], None);
        let to = photo(2, &["street", "b&w"], None);
        let add = paste_ops(&from, &fields(&["keywords"]), &to).unwrap();
        assert!(matches!(&add[..], [Op::AddKeywords { .. }]));
        let exact = paste_ops(&from, &fields(&["keywords_exact"]), &to).unwrap();
        // The removal first: the adds after it put back anything it takes under it.
        let [Op::RemoveKeywords { paths: gone }, Op::AddKeywords { paths: added }] = &exact[..] else {
            panic!("{exact:?}")
        };
        assert_eq!(gone, &[KeywordPath::from_pipe("b&w")]);
        assert_eq!(added.len(), 2);
        assert!(paste_ops(&from, &fields(&["albums"]), &to).is_err());
    }

    #[test]
    fn typed_place_fields_are_cleaned() {
        let edit = Edit::Place(BTreeMap::from([("country_code".into(), " pl ".into()), ("city".into(), "  ".into())]));
        let Op::SetPlace { value, .. } = op_for(&edit, &photo(1, &[], Some("Oslo"))).unwrap() else { panic!() };
        assert_eq!((value.country_code.as_deref(), value.city), (Some("PL"), None));
        assert!(op_for(&Edit::Geo(Some([91.0, 0.0])), &photo(1, &[], None)).is_err());
    }
}
