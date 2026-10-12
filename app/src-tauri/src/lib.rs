//! The desktop app: the library behind Tauri commands, and pixels behind the `c1img:`
//! protocol (`c1img://localhost/thumb/<row>`, `c1img://localhost/preview/<row>`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use c1index::{Filter, Library, PendingEdit, QueryOptions, QueryResult, SyncReport};
use c1model::provider::data_root;
use c1model::{
    BackupInfo, Change, Collection, CollectionChange, CollectionId, ColorLabel, Compatibility, GeoOrigin, GeoPoint,
    ImageTier, Iptc, KeywordPath, KeywordRemap, Op, Photo, PhotoSource, Pick, Place, SourceSpec, WriteStatus,
};
use serde::{Deserialize, Serialize};
use sources::Recent;
use tauri::http::{Response, StatusCode, header};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

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
    Ok(Session {
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
    })
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
        let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).max(1));
        let report = |state: AnalysisState| {
            *s.analysis.lock().unwrap() = state.clone();
            let _ = handle.emit("analysis", state);
        };
        let result = c1index::analyse::run(&s.library, s.source.as_ref(), threads, &s.analysis_stop, |p| {
            report(AnalysisState { running: true, done: p.done, total: p.total, failed: p.failed });
        });
        let last = s.analysis.lock().unwrap().clone();
        report(AnalysisState { running: false, ..last });
        if let Err(e) = result {
            let _ = handle.emit("sync-failed", format!("analysis: {e:#}"));
        }
        let _ = handle.emit("analysis-done", ());
        s.analysing.store(false, Ordering::SeqCst);
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
    *s.sync_error.lock().unwrap() = report.as_ref().err().cloned();
    let report = report?;
    if report.full || report.added + report.updated + report.deleted > 0 {
        let _ = app.emit("library-changed", &report);
    }
    start_analysis(app);
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
    let mut changes = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(photo) = s.library.photo(row).map_err(text)? {
            changes.push((row, op_for(&edit, &photo)?));
        }
    }
    s.touched();
    s.library.edit(&changes).map_err(|e| format!("{e:#}"))
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
    let mut changes = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(photo) = s.library.photo(row).map_err(text)? {
            let value = moved_place(&photo.place, &target, code.as_deref());
            changes.push((row, Op::SetPlace { base: photo.place, value }));
        }
    }
    s.touched();
    s.library.edit(&changes).map_err(|e| format!("{e:#}"))
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
    let mut changes = Vec::new();
    for (rows, code) in items {
        let code = clean_text("country_code", &code).ok_or("an empty country code")?;
        for row in rows {
            if let Some(photo) = s.library.photo(row).map_err(text)? {
                let value = Place { country_code: Some(code.clone()), ..photo.place.clone() };
                changes.push((row, Op::SetPlace { base: photo.place, value }));
            }
        }
    }
    s.touched();
    s.library.edit(&changes).map_err(|e| format!("{e:#}"))
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
    let mut changes = Vec::new();
    for row in rows {
        if let Some(photo) = s.library.photo(row).map_err(text)? {
            changes.extend(paste_ops(&from, &fields, &photo, fill.unwrap_or(false))?.into_iter().map(|op| (row, op)));
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
        let report = s.source.apply(&plan).map_err(text)?;
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
            change_collection,
            undo_collection,
            collection_changes,
            backups,
            restore_backup,
            look,
            analysis_state,
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
    fn typed_place_fields_are_cleaned() {
        let edit = Edit::Place(BTreeMap::from([("country_code".into(), " pl ".into()), ("city".into(), "  ".into())]));
        let Op::SetPlace { value, .. } = op_for(&edit, &photo(1, &[], Some("Oslo"))).unwrap() else { panic!() };
        assert_eq!((value.country_code.as_deref(), value.city), (Some("PL"), None));
        assert!(op_for(&Edit::Geo(Some([91.0, 0.0])), &photo(1, &[], None)).is_err());
    }
}
