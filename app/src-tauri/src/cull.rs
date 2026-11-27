//! Choosing the best of alike photos: one duplicate group or series at a time, its photos
//! in order of which to keep, and the pick itself — the photos kept, and what becomes of
//! the rest — as one step that can be taken back.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Weak};

use c1ai::aesthetic::{self, Aesthetic, Prompts};
use c1ai::claude;
use c1ai::models::SIGLIP2;
use c1ai::pick::{self, Frames, PickAsk};
use c1ai::siglip::Embedder;
use c1ai::{AiError, RunRecord};
use c1analysis::{Candidate, Framing, Judge};
use c1index::cull::NewView;
use c1index::{Filter, Library, QueryOptions, Sort};
use c1model::{ColorLabel, Op, Photo, Removal};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::ai::{Access, AiImage, Asker, asker, load_settings, models_root, pixels, pixels_at_hand};
use crate::{App, Session, is_current, text};

/// The long edge of the picture the local model is given: it squashes it to 224 itself.
const LOOK_EDGE: u32 = 448;
/// A run asking about every group ends after this many groups in a row got no answer.
const ASK_FAILURES: usize = 3;

/// A photo of a group, with what the order goes by.
#[derive(Serialize)]
pub(crate) struct CullPhoto {
    row: u64,
    name: String,
    rating: Option<u8>,
    color: Option<ColorLabel>,
    edited: bool,
    /// Picked before as the one to keep.
    kept: bool,
    sharpness: f32,
    focus: &'static str,
    /// How the local model saw it, once it has looked.
    aesthetic: Option<Aesthetic>,
    /// How it is framed, measured from its pixels, once it has been.
    framing: Option<Framing>,
    /// How well composed, 0 to 1, next to the others of its group: its framing, with the
    /// local model's view once it has looked at them all. None until every photo is framed.
    composition: Option<f32>,
    /// 0 to 1, next to the others of its group.
    score: f32,
    reasons: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct CullGroupView {
    id: String,
    /// Its photos the filter selects, oldest first.
    photos: Vec<CullPhoto>,
    /// Their rows, the one to keep first: a photo picked before comes before the others.
    order: Vec<u64>,
    /// What the order rests on: what was measured, and the local model's view of the
    /// photos once it has looked at them all. Nothing gives a candidate a `verdict` yet.
    judge: Judge,
    /// The two best are too close to call.
    close: bool,
    /// The local model is downloaded: it can be asked to look.
    model: bool,
    /// How many of `photos` the local model has no view of: while there is one, its
    /// views of the others do not count in the order.
    unseen: usize,
    /// What a model that compared the frames said, if one was asked.
    verdict: Option<VerdictView>,
    /// The photos worth keeping, the one to keep first: it, and the few as good that are
    /// other pictures. The ones picked before, where some were.
    suggested: Vec<u64>,
    /// Photos picked by hand (a selection, not a group the analysis found).
    selection: bool,
    /// Of the photos asked for, how many could not be measured and are left out.
    left_out: usize,
}

/// A model's word on a group, for the bar.
#[derive(Serialize)]
pub(crate) struct VerdictView {
    /// `claude`, `claude-code`, `gemini`.
    provider: String,
    model: String,
    /// How clear the choice is, in its words.
    note: String,
    /// How many of the photos shown it was shown.
    sent: usize,
}

/// What a group of photos picked by hand is called: `pick:` and their rows, sorted, with
/// commas between.
const SELECTION: &str = "pick:";
/// The most photos picked by hand to choose among.
pub(crate) const SELECTION_MAX: usize = 200;

/// The rows of a group of photos picked by hand, sorted; none for a duplicate group or a
/// series.
fn selection(id: &str) -> Option<Vec<u64>> {
    let rows = id.strip_prefix(SELECTION)?;
    let mut out: Vec<u64> = rows.split(',').filter_map(|r| r.trim().parse().ok()).collect();
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// The rows of a group among the photos `filter` selects, oldest first: every frame that
/// matches, stacked or not. Of photos picked by hand, those of them the filter shows.
fn group_rows(library: &Library, filter: &Filter, id: &str) -> Result<Vec<u64>, String> {
    let narrowed = match selection(id) {
        Some(picked) => {
            if picked.len() > SELECTION_MAX {
                return Err(format!("Best of takes up to {SELECTION_MAX} photos at once: pick fewer"));
            }
            let rows = match &filter.rows {
                Some(shown) => picked.into_iter().filter(|r| shown.contains(r)).collect(),
                None => picked,
            };
            Filter { rows: Some(rows), group: None, stacked: false, ..filter.clone() }
        }
        None => Filter { group: Some(id.to_owned()), stacked: false, ..filter.clone() },
    };
    Ok(library.query(&narrowed, &QueryOptions { sort: Sort::Date, ..QueryOptions::default() }).map_err(text)?.rows)
}

/// Gets a group's `rows` ready to be shown: how each is framed is measured where it is not
/// kept for its render, and a photo not measured yet (one picked by hand) is measured on the
/// way. A photo that cannot be read is left as it is: the view counts it.
fn prepare(s: &Session, rows: &[u64]) {
    let threads = c1index::analyse::default_threads();
    match c1index::analyse::frame(&s.library, s.source.as_ref(), &s.thumbs, rows, threads) {
        Ok(0) => {}
        Ok(failed) => {
            s.log.event(&format!("Best of: {failed} photo(s) could not be read to measure their framing"), &[])
        }
        Err(e) => s.log.event(&format!("Best of: measuring how photos are framed failed: {e:#}"), &[]),
    }
}

/// What ranking calls the frame the model likes best.
const BEST_PICTURE: &str = "best picture";

/// A score this far above the group's mean sets a frame apart on that axis. The four
/// scores are noisy, and the model reads a small picture: under this they are too alike
/// to name one, and the frame is the best picture, no more.
const LEADS_BY: f32 = 0.05;

/// The way `seen` stands out from the group's `views`: the axis on which it is furthest
/// above their mean, where that is [`LEADS_BY`] at least.
fn leading(seen: &Aesthetic, views: &[Aesthetic]) -> Option<&'static str> {
    let n = views.len().max(1) as f32;
    let mean = |axis: usize| views.iter().map(|v| v.as_array()[axis]).sum::<f32>() / n;
    seen.named()
        .enumerate()
        .map(|(axis, (name, score))| (name, score - mean(axis)))
        .filter(|(_, above)| *above >= LEADS_BY)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(name, _)| if name == "technical" { "technically" } else { name })
}

fn group_view(library: &Library, filter: &Filter, id: &str) -> Result<CullGroupView, String> {
    view_of(library, id, group_rows(library, filter, id)?)
}

/// [`group_view`] of the group `id` with `rows` as its photos.
fn view_of(library: &Library, id: &str, rows: Vec<u64>) -> Result<CullGroupView, String> {
    let chosen = selection(id).is_some();
    // Picks are made of the groups the analysis finds: photos picked by hand are chosen
    // among afresh, whichever of them was kept of its own group.
    let kept: HashSet<u64> = if chosen { HashSet::new() } else { library.kept_of(&rows).into_iter().collect() };
    let framings = library.framings(&rows).map_err(text)?;
    let asked = rows.len();
    // Only views scored the way photos are scored now: older ones wait to be scored again.
    let views = library.views(&rows).map_err(text)?;
    let seen =
        |row: u64| views.get(&row).filter(|v| v.version == aesthetic::VERSION).map(|v| Aesthetic::from(v.scores));
    // A model's answer that still holds for the photos shown: it saw every one of them.
    let (verdict, places) = library.verdict(&rows).map_err(text)?.unzip();
    let places = places.unwrap_or_default();
    let classes = library.class_settings();
    let mut photos = Vec::with_capacity(rows.len());
    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(photo) = library.photo(row).map_err(text)? else { continue };
        let Some(look) = library.look(row) else { continue };
        let a = &look.analysis;
        let framing = framings.get(&row).copied();
        candidates.push(Candidate {
            sharpness: a.sharpness,
            highlights: a.highlights,
            shadows: a.shadows,
            rating: photo.rating,
            edited: photo.edit.edited,
            aesthetic: seen(row).map(|s| s.overall()),
            verdict: places.get(&row).cloned(),
            hash: a.hash,
            framing,
            seen_composition: seen(row).map(|s| s.composition),
        });
        photos.push(CullPhoto {
            row,
            name: photo.file.name,
            rating: photo.rating,
            color: photo.color,
            edited: photo.edit.edited,
            kept: kept.contains(&row),
            sharpness: a.sharpness,
            focus: a.focus(&classes),
            aesthetic: seen(row),
            framing,
            composition: None,
            score: 0.0,
            reasons: Vec::new(),
        });
    }
    let left_out = asked - photos.len();
    let ranked = c1analysis::rank(&candidates, &classes);
    for (photo, entry) in photos.iter_mut().zip(ranked.entries) {
        photo.score = entry.score;
        photo.composition = entry.composition;
        photo.reasons = entry.reasons;
    }
    // The best picture by the model's view is said by what sets it apart, where something does.
    let views: Vec<Aesthetic> = photos.iter().filter_map(|p| p.aesthetic).collect();
    for photo in &mut photos {
        if let Some(seen) = photo.aesthetic
            && let Some(reason) = photo.reasons.iter_mut().find(|r| *r == BEST_PICTURE)
            && let Some(way) = leading(&seen, &views)
        {
            *reason = format!("best {way}");
        }
        // Measured and seen alike: said once.
        let mut said = HashSet::new();
        photo.reasons.retain(|r| said.insert(r.clone()));
    }
    // The user's own choice, made before, outranks any measure.
    let picked: Vec<bool> = photos.iter().map(|p| p.kept).collect();
    let order = picks_first(ranked.order, &picked);
    for photo in photos.iter_mut().filter(|p| p.kept) {
        photo.reasons.insert(0, PICKED_BEFORE.to_owned());
    }
    // The photos picked before are what is suggested again, where there are some.
    let suggested: Vec<u64> = if picked.contains(&true) {
        order.iter().filter(|&&i| picked[i]).map(|&i| photos[i].row).collect()
    } else {
        ranked.suggest.iter().map(|&i| photos[i].row).collect()
    };
    Ok(CullGroupView {
        suggested,
        selection: chosen,
        left_out,
        id: id.to_owned(),
        close: ranked.close && !picked.contains(&true),
        order: order.iter().map(|&i| photos[i].row).collect(),
        unseen: photos.iter().filter(|p| p.aesthetic.is_none()).count(),
        photos,
        judge: ranked.judge,
        model: SIGLIP2.installed(&models_root()),
        verdict: verdict.map(|v| VerdictView {
            sent: places.len(),
            provider: v.provider,
            model: v.model,
            note: v.note,
        }),
    })
}

/// Why a photo leads its group whatever was measured.
const PICKED_BEFORE: &str = "picked before";

/// `order` (indexes into a group's photos, the one to keep first) with the photos picked
/// before (`picked`, by index) ahead of the others, each part in the order it had: seen
/// again, a group suggests what the user chose, not what Enter would send to the trash.
fn picks_first(order: Vec<usize>, picked: &[bool]) -> Vec<usize> {
    let (mut first, rest): (Vec<usize>, Vec<usize>) = order.into_iter().partition(|&i| picked[i]);
    first.extend(rest);
    first
}

/// A group to choose the best of, in the list gone through.
#[derive(Serialize)]
pub(crate) struct CullGroupId {
    id: String,
    /// One of its photos is picked already as the one to keep.
    picked: bool,
}

/// The duplicate groups (or series) among the photos `filter` selects, oldest first: the
/// ones gone through to choose the best of each.
#[tauri::command]
pub(crate) async fn cull_groups(app: State<'_, App>, filter: Filter, series: bool) -> Result<Vec<CullGroupId>, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        let ids = s.library.group_ids(&filter, series).map_err(text)?;
        Ok(ids.into_iter().map(|(id, picked)| CullGroupId { id, picked }).collect())
    })
    .await
    .map_err(text)?
}

/// Of a group's `rows` (in the order taken), the `most` that come first in `order` (the
/// one to keep first), in the order taken: what a model is shown of a group bigger than a
/// question holds.
fn best_frames(rows: &[u64], order: &[u64], most: usize) -> Vec<u64> {
    let best: HashSet<u64> = order.iter().copied().take(most).collect();
    rows.iter().copied().filter(|row| best.contains(row)).collect()
}

/// The frames of the group `id` a model is shown, of its `rows` (in the order taken): all
/// of them, or of a group bigger than a question holds the best by what is known so far.
/// Only such a group is put in order for it.
fn frames_to_ask(library: &Library, id: &str, rows: &[u64]) -> Result<Vec<u64>, String> {
    if rows.len() <= pick::MAX_FRAMES {
        return Ok(rows.to_vec());
    }
    let view = view_of(library, id, rows.to_vec())?;
    let shown: Vec<u64> = view.photos.iter().map(|p| p.row).collect();
    Ok(best_frames(&shown, &view.order, pick::MAX_FRAMES))
}

/// Counts a call about the group `id` with the rest of what the models were asked. One
/// that cannot be counted is logged: the answer is not lost for it.
fn count_call(s: &Session, id: &str, record: &RunRecord) {
    let id = &label(id);
    if let Err(e) = s.library.record_ask(id, record) {
        s.log.event(&format!("Best of: the call about {id} could not be counted: {e:#}"), &[]);
    }
}

/// A group as the log names it: photos picked by hand by the first of them and how many,
/// not every row.
fn label(id: &str) -> String {
    match selection(id) {
        Some(rows) => format!("{SELECTION}{}+{}", rows.first().copied().unwrap_or(0), rows.len()),
        None => id.to_owned(),
    }
}

/// Why a group got no answer.
#[derive(Debug)]
enum NoAnswer {
    /// The group's own trouble (a frame with no image to send): no call was made, and the
    /// next group may well fare better.
    Group(String),
    /// The model's: the call failed, or what it said could not be used.
    Model(AiError),
    /// The answer came and could not be kept: nor would the next one be.
    Store(String),
}

impl std::fmt::Display for NoAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoAnswer::Group(why) | NoAnswer::Store(why) => f.write_str(why),
            NoAnswer::Model(e) => e.fmt(f),
        }
    }
}

/// Asks `asker` which frame of the group `id` to keep, of its `rows` (the photos the
/// filter selects, in the order taken), and keeps its answer. Claude gets each frame as an
/// image of its own; Gemini, which takes one file, gets them on one sheet, each loaded no
/// larger than its cell.
fn ask_about(s: &Session, asker: &Asker, id: &str, rows: &[u64]) -> Result<(), NoAnswer> {
    let sent = frames_to_ask(&s.library, id, rows).map_err(NoAnswer::Group)?;
    if sent.len() < 2 {
        return Err(NoAnswer::Group("there is one photo here: nothing to compare".into()));
    }
    let edge = asker.edge(sent.len());
    let mut pictures = Vec::with_capacity(sent.len());
    for &row in &sent {
        let picture = s
            .library
            .photo_id(row)
            .ok_or_else(|| "a photo of the group is gone".to_owned())
            .and_then(|id| pixels(s, &id, AiImage::Preview, edge).map_err(|e| format!("{e:#}")))
            .and_then(|found| found.ok_or_else(|| "a photo of the group has no image to send".to_owned()));
        pictures.push(picture.map_err(NoAnswer::Group)?);
    }
    let kind = if selection(id).is_some() {
        Frames::Chosen
    } else if id.starts_with("series:") {
        Frames::Burst
    } else {
        Frames::Twice
    };
    let ask = PickAsk { frames: sent.len(), kind, sheet: asker.sheet() };
    // Counted with the rest of what the models were asked, an answer that could not be
    // used too: it was paid for all the same.
    let (answer, record) = match asker.ask(&pick::message(&ask), &pictures) {
        Ok(answered) => answered,
        Err(failed) => {
            if let Some(record) = &failed.record {
                count_call(s, id, record);
            }
            return Err(NoAnswer::Model(failed.error));
        }
    };
    count_call(s, id, &record);
    let said = pick::read(&answer, sent.len()).map_err(NoAnswer::Model)?;
    let verdict = c1index::cull::NewVerdict {
        at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64),
        provider: record.provider.clone(),
        model: record.model.clone(),
        order: said.order.iter().map(|&i| i as u32).collect(),
        reasons: said.reasons,
        note: said.note,
    };
    s.library
        .put_verdict(&sent, verdict)
        .map_err(|e| NoAnswer::Store(format!("the answer about {id} could not be kept: {e:#}")))
}

/// Asks a model (Claude or Gemini, as the AI settings say) which frame of one group to
/// keep, and gives the group in the order it said, with its reasons. Not while every group
/// is being asked about: the run may be on this one, and both would be paid for.
#[tauri::command]
pub(crate) async fn ask_group(app: State<'_, App>, filter: Filter, id: String) -> Result<CullGroupView, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        let run = lock(&JOB.state).read(&s);
        if run.running && run.cloud {
            return Err("AI is being asked about every group: stop that first, or wait for it to get here".into());
        }
        // Before a picture is loaded: with no key or no sign-in there is nothing to send.
        let asker = asker(&load_settings(&s), Arc::default(), &pick::system(), pick::schema())?;
        let rows = group_rows(&s.library, &filter, &id)?;
        prepare(&s, &rows);
        ask_about(&s, &asker, &id, &rows).map_err(text)?;
        group_view(&s.library, &filter, &id)
    })
    .await
    .map_err(text)?
}

/// What asking a model about every group would come to.
#[derive(Serialize)]
pub(crate) struct AskEstimate {
    /// Groups not asked about yet (as they are now), and the frames that would be sent.
    groups: usize,
    photos: usize,
    /// Groups left out: asked about already.
    skipped: usize,
    /// US dollars at effort `low`, with an API key; nothing on a plan or with Gemini.
    cost: Option<f64>,
    plan: bool,
    /// `Claude`, `Gemini`; `Local` cannot be asked.
    via: &'static str,
    model: String,
    /// How much the model may think (`low`, `medium`, `high`): above `low`, thinking adds
    /// to what `cost` counts.
    effort: String,
    /// Calls made at once.
    workers: usize,
    /// Why no model can be asked as the settings are (no key, not signed in, the local model).
    unable: Option<String>,
}

/// A group by id, with its rows under the filter, in the order taken.
type Unasked = (String, Vec<u64>);

/// The groups among the photos `filter` selects that a model's answer does not stand for,
/// oldest first, and how many it does stand for.
fn unasked(library: &Library, filter: &Filter, series: bool) -> Result<(Vec<Unasked>, usize), String> {
    let ids = library.group_ids(filter, series).map_err(text)?;
    // Every group's rows from one read of the filter's photos, oldest first, as
    // [`group_rows`] gives them one group at a time: a read per group takes a minute over
    // a few thousand groups.
    let all = Filter {
        duplicates: filter.duplicates || !series,
        series: filter.series || series,
        stacked: false,
        ..filter.clone()
    };
    let rows = library.query(&all, &QueryOptions { sort: Sort::Date, ..QueryOptions::default() }).map_err(text)?.rows;
    let mut members: HashMap<String, Vec<u64>> = HashMap::new();
    for row in rows {
        let Some(look) = library.look(row) else { continue };
        if let Some(id) = if series { look.series } else { look.duplicates } {
            members.entry(id).or_default().push(row);
        }
    }
    let (ids, rows): (Vec<String>, Vec<Vec<u64>>) = ids
        .into_iter()
        .filter_map(|(id, _)| members.remove(&id).filter(|rows| rows.len() > 1).map(|rows| (id, rows)))
        .unzip();
    let asked = library.asked(&rows).map_err(text)?;
    let skipped = asked.iter().filter(|&&asked| asked).count();
    let todo = ids.into_iter().zip(rows).zip(asked).filter(|(_, asked)| !asked).map(|(group, _)| group).collect();
    Ok((todo, skipped))
}

/// What asking about every series (or duplicate group) under `filter` would take, and
/// whether a model can be asked at all as the settings are.
#[tauri::command]
pub(crate) async fn ask_estimate(app: State<'_, App>, filter: Filter, series: bool) -> Result<AskEstimate, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        let settings = load_settings(&s);
        // The key, the sign-in: said before the question is put, not by its first group.
        let unable = asker(&settings, Arc::default(), &pick::system(), pick::schema()).err();
        let (todo, skipped) = unasked(&s.library, &filter, series)?;
        let photos: usize = todo.iter().map(|(_, rows)| rows.len().min(pick::MAX_FRAMES)).sum();
        let model = claude::model(&settings.model);
        let claude = || model.map_or(settings.model.clone(), |m| m.name.to_string());
        let (via, name, cost) = match settings.access {
            Access::Local => ("Local", SIGLIP2.name.to_string(), None),
            Access::Gemini => ("Gemini", settings.gemini_model.clone(), None),
            Access::Plan => ("Claude", claude(), None),
            Access::Api => ("Claude", claude(), model.map(|m| claude::estimate_frames(&m, todo.len(), photos))),
        };
        Ok(AskEstimate {
            groups: todo.len(),
            photos,
            skipped,
            cost,
            plan: settings.access != Access::Api,
            via,
            model: name,
            effort: settings.effort.clone(),
            workers: settings.workers(),
            unable,
        })
    })
    .await
    .map_err(text)?
}

/// The local model as ranking uses it: the image tower and the sentences photos are held
/// against.
struct Ranker {
    /// One picture at a time goes through the image tower.
    embedder: Mutex<Embedder>,
    prompts: Prompts,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// How far looking at every group, or asking about it, has got.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct RankState {
    running: bool,
    /// A model is asked to compare each group's frames; else the local model looks at each photo.
    cloud: bool,
    /// Groups looked at (or asked about), of how many.
    done: usize,
    total: usize,
    /// Photos that could not be looked at, or groups that got no answer; `events.log`
    /// says why of each.
    failed: usize,
    /// The group looked at (or asked about) last: a screen showing that one reads it again.
    group: Option<String>,
    /// Why the last run ended early.
    error: Option<String>,
}

/// A run's state with the session it is of: asked about from another session (another
/// catalog opened since), there is no run to tell of.
struct Held<S> {
    of: Weak<S>,
    state: RankState,
}

impl<S> Held<S> {
    const fn new() -> Self {
        let state = RankState { running: false, cloud: false, done: 0, total: 0, failed: 0, group: None, error: None };
        Held { of: Weak::new(), state }
    }

    fn is_of(&self, s: &Arc<S>) -> bool {
        Weak::ptr_eq(&self.of, &Arc::downgrade(s))
    }

    fn read(&self, s: &Arc<S>) -> RankState {
        if self.is_of(s) { self.state.clone() } else { RankState::default() }
    }

    /// Changes the state of `s`'s run, which starts from nothing when the state held was
    /// another session's.
    fn change(&mut self, s: &Arc<S>, change: impl FnOnce(&mut RankState)) -> RankState {
        if !self.is_of(s) {
            *self = Held { of: Arc::downgrade(s), state: RankState::default() };
        }
        change(&mut self.state);
        self.state.clone()
    }
}

/// Looking at photos with the local model: the one run over every group, and the model
/// itself. One for the process, whatever the catalog open: the model is one.
///
/// - `running` is raised by [`start_ranking`] before the run's thread starts and lowered
///   by [`Ends`] when it ends, however it ends. `stop` is cleared only by the start that
///   raised `running`, so a stop asked for never reaches a later run.
/// - `state` is of one session; any other is told of no run.
/// - `ranker` is the model, kept while it is `wanted` (the choosing screen is open) or a
///   run goes on. Its lock is held only to read or change the slot, never across a load:
///   `loading` is what a second caller waits on while the first loads.
struct RankJob {
    running: AtomicBool,
    /// Shared, for a model's client to end a call under way by.
    stop: LazyLock<Arc<AtomicBool>>,
    state: Mutex<Held<Session>>,
    wanted: AtomicBool,
    ranker: Mutex<Option<Arc<Ranker>>>,
    loading: Mutex<()>,
}

static JOB: RankJob = RankJob {
    running: AtomicBool::new(false),
    stop: LazyLock::new(Arc::default),
    state: Mutex::new(Held::new()),
    wanted: AtomicBool::new(false),
    ranker: Mutex::new(None),
    loading: Mutex::new(()),
};

impl RankJob {
    /// The model, loaded when it is not. One load at a time: whoever asks while it loads
    /// waits for that load and is given its model. `stop` ends the embedding of the
    /// sentences (the first time ever only: they are kept beside the model).
    fn ranker(&self, stop: &AtomicBool) -> Result<Arc<Ranker>, String> {
        let held = || lock(&self.ranker).clone();
        if let Some(ranker) = held() {
            return Ok(ranker);
        }
        let _loading = lock(&self.loading);
        if let Some(ranker) = held() {
            return Ok(ranker);
        }
        let mut embedder = Embedder::open(&models_root()).map_err(text)?;
        let prompts = Prompts::load(&mut embedder, &|| stop.load(Ordering::Relaxed)).map_err(text)?;
        let ranker = Arc::new(Ranker { embedder: Mutex::new(embedder), prompts });
        // Kept only where it will be asked for again: the screen closed while it loaded
        // has let it go already.
        let mut slot = lock(&self.ranker);
        if self.needed() {
            *slot = Some(ranker.clone());
        }
        Ok(ranker)
    }

    fn needed(&self) -> bool {
        self.wanted.load(Ordering::SeqCst) || self.running.load(Ordering::SeqCst)
    }

    /// Lets the model go, unless the screen is open or a run goes on: when the screen
    /// closes, and when a run ends.
    fn release(&self) {
        let gone = {
            let mut slot = lock(&self.ranker);
            if self.needed() { None } else { slot.take() }
        };
        drop(gone);
    }

    /// Changes the state of `s`'s run, and tells the window when `s` is the session open.
    fn report(&self, app: &AppHandle, s: &Arc<Session>, change: impl FnOnce(&mut RankState)) {
        let state = lock(&self.state).change(s, change);
        if is_current(app, s) {
            let _ = app.emit("ranking", state);
        }
    }

    /// The run of `s` is over, with `error` if it ended early. Called with `running`
    /// lowered, so a run of another catalog may have begun: the state is its own then,
    /// and is left alone.
    fn ended(&self, app: &AppHandle, s: &Arc<Session>, error: Option<String>) {
        let state = {
            let mut held = lock(&self.state);
            if !held.is_of(s) {
                return;
            }
            held.change(s, |state| {
                state.running = false;
                state.error = error;
            })
        };
        if is_current(app, s) {
            let _ = app.emit("ranking", state);
            let _ = app.emit("ranking-done", ());
        }
    }
}

/// Ends a run: lowers `running` and lets the model go if no one needs it, a panic included.
struct Ends;

impl Drop for Ends {
    fn drop(&mut self) {
        JOB.running.store(false, Ordering::SeqCst);
        JOB.release();
    }
}

/// What looks at photos: the local model, or a test's stand-in.
trait Eye {
    /// Gets ready to look, when the first photo needs it. `false`: stopped meanwhile. An
    /// error ends the look.
    fn open(&mut self) -> Result<bool, String>;
    /// The embedding of the photo at `row`, once [`Eye::open`]. An error is that photo's
    /// alone: the look goes on.
    fn embed(&mut self, row: u64) -> Result<Vec<f32>, String>;
    /// What an embedding comes to, once [`Eye::open`].
    fn score(&self, embedding: &[f32]) -> Aesthetic;
}

/// The local model looking at the photos of `s`.
struct ModelEye<'a> {
    s: &'a Session,
    stop: &'a AtomicBool,
    /// Make a preview or thumbnail for a photo that has none. Not in a run over every
    /// group: that would render the catalog as a side effect, with no way to stop it.
    make: bool,
    ranker: Option<Arc<Ranker>>,
}

impl Eye for ModelEye<'_> {
    fn open(&mut self) -> Result<bool, String> {
        if self.ranker.is_none() {
            match JOB.ranker(self.stop) {
                Ok(ranker) => self.ranker = Some(ranker),
                // Stopped while the sentences were embedded: no failure.
                Err(_) if self.stop.load(Ordering::Relaxed) => return Ok(false),
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    fn embed(&mut self, row: u64) -> Result<Vec<f32>, String> {
        let ranker = self.ranker.as_ref().ok_or("the local model is not loaded")?;
        let id = self.s.library.photo_id(row).ok_or("the photo is gone from the library")?;
        let picture = if self.make {
            pixels(self.s, &id, AiImage::Thumb, LOOK_EDGE)
        } else {
            pixels_at_hand(self.s, &id, LOOK_EDGE)
        };
        let seen = picture
            .map_err(|e| format!("{e:#}"))
            .and_then(|found| found.ok_or_else(|| "it has no thumbnail or preview yet".to_owned()))
            .and_then(|picture| {
                ranker.embedder.lock().unwrap_or_else(|e| e.into_inner()).image(&picture).map_err(text)
            });
        if let Err(why) = &seen {
            self.s.log.event(&format!("Best of: the local model could not look at {}: {why}", id.ext), &[]);
        }
        seen
    }

    fn score(&self, embedding: &[f32]) -> Aesthetic {
        self.ranker.as_ref().map_or_else(Aesthetic::default, |ranker| aesthetic::score(embedding, &ranker.prompts))
    }
}

/// What a look at some photos came to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Looked {
    /// Photos that could not be looked at.
    failed: usize,
    /// Stopped before every photo was looked at.
    stopped: bool,
}

/// Has `eye` look at the photos at `rows` it has not seen yet (or saw rendered otherwise),
/// and keeps what it saw. A view scored another way is scored again from its embedding,
/// without looking again; where every photo has its view, `eye` is not even opened. A
/// photo not measured is not looked at: nothing would say which render the view is of, so
/// it could not be kept. `stop` ends it between photos, with what was seen so far kept.
fn look_at(library: &Library, rows: &[u64], stop: &AtomicBool, eye: &mut dyn Eye) -> Result<Looked, String> {
    let have = library.views(rows).map_err(text)?;
    let mut new = Vec::new();
    let mut looked = Looked::default();
    for &row in rows {
        if stop.load(Ordering::Relaxed) {
            looked.stopped = true;
            break;
        }
        let stored = match have.get(&row) {
            Some(view) if view.version == aesthetic::VERSION => continue,
            Some(view) => Some(view.embedding.clone()),
            None if library.look(row).is_none() => {
                looked.failed += 1;
                continue;
            }
            None => None,
        };
        if !eye.open()? {
            looked.stopped = true;
            break;
        }
        let embedding = match stored.map_or_else(|| eye.embed(row), Ok) {
            Ok(embedding) => embedding,
            Err(_) => {
                looked.failed += 1;
                continue;
            }
        };
        let scores = eye.score(&embedding).as_array();
        new.push(NewView { row, version: aesthetic::VERSION, embedding, scores });
    }
    let made = new.len();
    // One measured again meanwhile is not kept either.
    looked.failed += made - library.put_views(new).map_err(text)?;
    Ok(looked)
}

/// Has the local model look at one group's photos, and gives the group in its new order.
#[tauri::command]
pub(crate) async fn rank_group(app: State<'_, App>, filter: Filter, id: String) -> Result<CullGroupView, String> {
    let s = app.session()?;
    JOB.wanted.store(true, Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        // One group, asked for by hand: a run stopped before does not stop it.
        let go = AtomicBool::new(false);
        let rows = group_rows(&s.library, &filter, &id)?;
        prepare(&s, &rows);
        let mut eye = ModelEye { s: &s, stop: &go, make: true, ranker: None };
        look_at(&s.library, &rows, &go, &mut eye)?;
        group_view(&s.library, &filter, &id)
    })
    .await
    .map_err(text)?
}

/// Asks about each of `todo` with `ask`, `workers` of them at a time, in order, and tells
/// `told` of each that was answered (`None`) or not (why). A group with no answer is passed
/// over. The run ends early, with why, on a failure no other group would fare better with
/// (no credit, a limit reached, the answers not kept), and when [`ASK_FAILURES`] calls in
/// a row gave no answer; a group's own trouble, with no call made, is neither. `stopped`
/// ends it quietly, as does a call stopped under way.
fn ask_each<T: Sync>(
    todo: &[T],
    workers: usize,
    stopped: &(dyn Fn() -> bool + Sync),
    ask: &(dyn Fn(&T) -> Result<(), NoAnswer> + Sync),
    told: &(dyn Fn(&T, Option<&str>) + Sync),
) -> Result<(), String> {
    let next = AtomicUsize::new(0);
    let in_a_row = AtomicUsize::new(0);
    let ended: Mutex<Option<String>> = Mutex::new(None);
    let over = || stopped() || lock(&ended).is_some();
    let end = |why: String| {
        lock(&ended).get_or_insert(why);
    };
    std::thread::scope(|scope| {
        for _ in 0..workers.clamp(1, todo.len().max(1)) {
            scope.spawn(|| {
                while !over() {
                    let Some(group) = todo.get(next.fetch_add(1, Ordering::SeqCst)) else { break };
                    match ask(group) {
                        Ok(()) => {
                            in_a_row.store(0, Ordering::SeqCst);
                            told(group, None);
                        }
                        Err(NoAnswer::Model(AiError::Stopped)) => break,
                        Err(NoAnswer::Model(e)) if e.stops_run() => end(e.to_string()),
                        Err(NoAnswer::Store(why)) => end(why),
                        Err(NoAnswer::Group(why)) => told(group, Some(&why)),
                        Err(NoAnswer::Model(e)) => {
                            let why = e.to_string();
                            told(group, Some(&why));
                            if in_a_row.fetch_add(1, Ordering::SeqCst) + 1 >= ASK_FAILURES {
                                end(format!("{ASK_FAILURES} groups in a row got no answer; the last: {why}"));
                            }
                        }
                    }
                }
            });
        }
    });
    lock(&ended).take().map_or(Ok(()), Err)
}

/// Asks a model about every group no answer stands for, oldest first, as many at a time as
/// the AI settings say photos are described ([`ask_each`] has the rules). The model is
/// made once, before anything is read: with no key or no sign-in there is no run.
fn run_asking(app: &AppHandle, s: &Arc<Session>, filter: &Filter, series: bool) -> Result<(), String> {
    let settings = load_settings(s);
    let asker = asker(&settings, Arc::clone(&JOB.stop), &pick::system(), pick::schema())?;
    let (todo, _) = unasked(&s.library, filter, series)?;
    JOB.report(app, s, |state| state.total = todo.len());
    ask_each(
        &todo,
        settings.workers(),
        &|| JOB.stop.load(Ordering::Relaxed) || !is_current(app, s),
        &|(id, rows)| ask_about(s, &asker, id, rows),
        &|(id, _), failed| {
            if let Some(why) = failed {
                s.log.event(&format!("Best of: asking which frame of {id} to keep failed: {why}"), &[]);
            }
            JOB.report(app, s, |state| {
                state.done += 1;
                state.failed += usize::from(failed.is_some());
                state.group = Some(id.clone());
            });
        },
    )
}

fn run_ranking(app: &AppHandle, s: &Arc<Session>, filter: &Filter, series: bool) -> Result<(), String> {
    // The groups the screen goes through, in its order.
    let groups = s.library.group_ids(filter, series).map_err(text)?;
    JOB.report(app, s, |state| state.total = groups.len());
    let mut eye = ModelEye { s, stop: &JOB.stop, make: false, ranker: None };
    for (id, _) in groups {
        if JOB.stop.load(Ordering::Relaxed) || !is_current(app, s) {
            break;
        }
        let rows = group_rows(&s.library, filter, &id)?;
        prepare(s, &rows);
        let looked = look_at(&s.library, &rows, &JOB.stop, &mut eye)?;
        // Stopped in the middle of a group: what was seen of it is kept, but it is not done.
        if looked.stopped {
            break;
        }
        JOB.report(app, s, |state| {
            state.done += 1;
            state.failed += looked.failed;
            state.group = Some(id);
        });
    }
    Ok(())
}

/// Has the local model look at every series (or duplicate group) among the photos `filter`
/// selects, in the background, unless a run is under way. Each group's views are kept as
/// they come, so a run stopped keeps what it saw. Only pictures there already are looked
/// at: a photo with no thumbnail or preview yet is counted as failed, not rendered. With
/// `cloud`, the model the AI settings choose is asked about each group instead, and its
/// answers are kept as they come.
#[tauri::command]
pub(crate) async fn start_ranking(app: AppHandle, filter: Filter, series: bool, cloud: bool) -> Result<(), String> {
    let s = app.state::<App>().session()?;
    JOB.wanted.store(true, Ordering::SeqCst);
    if JOB.running.swap(true, Ordering::SeqCst) {
        // This catalog's run of the kind asked for: its progress is what the screen shows.
        // One of a catalog closed since ends after the photo (or the call) it is on.
        let held = lock(&JOB.state);
        return if !held.is_of(&s) {
            Err("A run over another catalog's groups is still ending: try again in a moment".into())
        } else if held.state.cloud == cloud {
            Ok(())
        } else if cloud {
            Err("The local model is looking at every group: stop that first (Shift+R)".into())
        } else {
            Err("AI is being asked about every group: stop that first (Shift+A)".into())
        };
    }
    JOB.stop.store(false, Ordering::SeqCst);
    JOB.report(&app, &s, |state| *state = RankState { running: true, cloud, ..RankState::default() });
    std::thread::spawn(move || {
        let ends = Ends;
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if cloud { run_asking(&app, &s, &filter, series) } else { run_ranking(&app, &s, &filter, series) }
        }));
        let error = match run {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e),
            Err(_) if cloud => Some("asking about the groups crashed".to_owned()),
            Err(_) => Some("looking at the photos crashed".to_owned()),
        };
        if let Some(e) = &error {
            s.log.event(&format!("Ranking alike photos failed: {e}"), &[]);
        }
        // Before the window hears of the end: a run started on hearing it finds none going.
        drop(ends);
        JOB.ended(&app, &s, error);
    });
    Ok(())
}

#[tauri::command]
pub(crate) fn stop_ranking() {
    JOB.stop.store(true, Ordering::SeqCst);
}

/// How far the run over this catalog's groups has got: asked by the choosing screen when
/// it opens, which is also what says the model is wanted again.
#[tauri::command]
pub(crate) fn ranking_state(app: State<'_, App>) -> RankState {
    JOB.wanted.store(true, Ordering::SeqCst);
    app.session().map(|s| lock(&JOB.state).read(&s)).unwrap_or_default()
}

/// The choosing is over: the local model is let go, unless a run still needs it (it is
/// let go when that ends). Off the window's thread: the model is hundreds of megabytes
/// to free.
#[tauri::command]
pub(crate) async fn cull_done() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        JOB.wanted.store(false, Ordering::SeqCst);
        JOB.release();
    })
    .await
    .map_err(text)
}

/// One group (`dup:<row>`, `series:<row>`) among the photos `filter` selects: its photos in
/// the order taken, and in order of which to keep.
#[tauri::command]
pub(crate) async fn cull_group(app: State<'_, App>, filter: Filter, id: String) -> Result<CullGroupView, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        prepare(&s, &group_rows(&s.library, &filter, &id)?);
        group_view(&s.library, &filter, &id)
    })
    .await
    .map_err(text)?
}

/// What becomes of the photos of a group that were not kept.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub(crate) enum Rest {
    /// To the app's trash.
    Trash,
    Hide,
    Rating(u8),
    Color(ColorLabel),
    Nothing,
}

impl Rest {
    /// The mark the rest gets, where that is what becomes of it.
    fn mark(self) -> Mark {
        match self {
            Rest::Rating(v) => Mark::Rating(v),
            Rest::Color(c) => Mark::Color(c),
            Rest::Trash | Rest::Hide | Rest::Nothing => Mark::Nothing,
        }
    }

    /// What is done where the source removes photos as `removes` says: one that removes
    /// nothing has no trash, and the rest is hidden.
    fn at(self, removes: Removal) -> Rest {
        if self == Rest::Trash && removes == Removal::None { Rest::Hide } else { self }
    }
}

/// What the photos kept are marked with.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub(crate) enum Mark {
    Nothing,
    Rating(u8),
    Color(ColorLabel),
}

impl Mark {
    /// The edit that gives a photo rated `rating` and tagged `color` this mark, if it does
    /// not have it already. A rating of 0 is no rating.
    fn edit(self, rating: Option<u8>, color: Option<ColorLabel>) -> Option<Op> {
        match self {
            Mark::Nothing => None,
            Mark::Rating(v) => {
                let value = Some(v.min(5)).filter(|&v| v > 0);
                (rating != value).then_some(Op::SetRating { base: None, value })
            }
            Mark::Color(c) => (color != Some(c)).then_some(Op::SetColor { base: None, value: Some(c) }),
        }
    }
}

/// A pick as made (`on`), or as taken back: what [`cull_revert`] needs to turn it round.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct CullStep {
    on: bool,
    /// The edits that turn the ratings and colours round.
    ops: Vec<(u64, Op)>,
    /// Rows that went to the trash, were hidden, were picked (or the reverse, when not `on`).
    binned: Vec<u64>,
    hidden: Vec<u64>,
    kept: Vec<u64>,
    /// Rows picked before, in the rest this time, whose pick the step took back (or gave
    /// back, when not `on`): a group has the picks of its last pick only.
    #[serde(default)]
    unkept: Vec<u64>,
    /// Rows left alone: variants of an image that is kept, where trashing takes the whole image.
    skipped: Vec<u64>,
}

/// `rest` without the rows whose image (`asset_of`) one of `keep` is a variant of, and
/// those rows.
fn spared(keep: &[u64], rest: &[u64], asset_of: impl Fn(u64) -> Option<String>) -> (Vec<u64>, Vec<u64>) {
    let held: HashSet<String> = keep.iter().filter_map(|&row| asset_of(row)).collect();
    rest.iter().partition(|&&row| asset_of(row).is_none_or(|asset| !held.contains(&asset)))
}

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// Keeps `keep` and deals with `rest` as `action` says, where the source removes photos as
/// `removes` says. All of it or nothing: when a part of it fails, the parts done are taken
/// back, so no group is left half picked with nothing to undo.
fn apply(
    library: &Library,
    removes: Removal,
    keep: &[u64],
    rest: &[u64],
    action: Rest,
    mark: Mark,
    remember: bool,
) -> Result<CullStep, String> {
    if keep.is_empty() {
        return Err("No photo to keep was named: a pick keeps one at least".into());
    }
    let rest: Vec<u64> = rest.iter().copied().filter(|row| !keep.contains(row)).collect();
    let mut step = CullStep { on: true, ..CullStep::default() };
    match carry_out(library, removes, keep, &rest, action.at(removes), mark, remember, &mut step) {
        Ok(()) => Ok(step),
        Err(e) => match revert(library, step) {
            Ok(_) => Err(e),
            Err(back) => Err(format!("{e}. What was done of it could not be taken back: {back}")),
        },
    }
}

/// [`apply`], with what it did so far written to `step` as it goes.
#[allow(clippy::too_many_arguments)]
fn carry_out(
    library: &Library,
    removes: Removal,
    keep: &[u64],
    rest: &[u64],
    action: Rest,
    mark: Mark,
    remember: bool,
    step: &mut CullStep,
) -> Result<(), String> {
    let mut all: Vec<u64> = keep.iter().chain(rest).copied().collect();
    let mark_of = |row: u64| if keep.contains(&row) { mark } else { action.mark() };
    if (mark, action.mark()) != (Mark::Nothing, Mark::Nothing) {
        step.ops = library
            .edit_photos(&all, |row, photo: &Photo| {
                Ok(mark_of(row).edit(photo.rating, photo.color).into_iter().collect())
            })
            .map_err(err)?;
    }
    match action {
        Rest::Trash => {
            let whole = removes == Removal::MarksTrashed;
            let (going, skipped) = if whole {
                spared(keep, rest, |row| library.photo(row).ok().flatten().map(|p| p.asset.0))
            } else {
                (rest.to_vec(), Vec::new())
            };
            step.skipped = skipped;
            step.binned = library.set_binned(&going, true, whole).map_err(err)?;
        }
        Rest::Hide => step.hidden = library.set_hidden(rest, true).map_err(err)?,
        Rest::Rating(_) | Rest::Color(_) | Rest::Nothing => {}
    }
    // The picks, and the groups stacked again, once: also those of the variants that went
    // to the trash with their image.
    all.extend(&step.binned);
    if remember {
        (step.kept, step.unkept) = library.repick(keep, rest, &all).map_err(err)?;
    } else {
        // Photos picked by hand: the picks of the groups they are in stay as they were.
        library.restack(&all).map_err(err)?;
    }
    Ok(())
}

fn revert(library: &Library, step: CullStep) -> Result<CullStep, String> {
    let ops = if step.ops.is_empty() { Vec::new() } else { library.edit(&step.ops).map_err(err)? };
    // The rows are the ones that changed: their other variants are among them already.
    let binned = library.set_binned(&step.binned, !step.on, false).map_err(err)?;
    let hidden = library.set_hidden(&step.hidden, !step.on).map_err(err)?;
    let around: Vec<u64> = binned.iter().chain(&hidden).copied().collect();
    let (kept, unkept) = if step.on {
        let (back, dropped) = library.repick(&step.unkept, &step.kept, &around).map_err(err)?;
        (dropped, back)
    } else {
        library.repick(&step.kept, &step.unkept, &around).map_err(err)?
    };
    Ok(CullStep { on: !step.on, ops, binned, hidden, kept, unkept, skipped: step.skipped })
}

/// Keeps `keep` of a group and deals with `rest` as `action` says, as one step: the photos
/// kept are marked (`mark`) and, unless `remember` is false (photos picked by hand), are
/// remembered as the ones to keep and stand for their group when stacked. Returns what
/// [`cull_revert`] takes back.
#[tauri::command]
pub(crate) async fn cull_apply(
    app: State<'_, App>,
    keep: Vec<u64>,
    rest: Vec<u64>,
    action: Rest,
    mark: Mark,
    remember: Option<bool>,
) -> Result<CullStep, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        s.touched();
        let remember = remember.unwrap_or(true);
        apply(&s.library, s.source.capabilities().removes, &keep, &rest, action, mark, remember)
    })
    .await
    .map_err(text)?
}

/// Turns a pick round: takes back one made, or makes again one taken back. Returns the
/// step that turns it round once more.
#[tauri::command]
pub(crate) async fn cull_revert(app: State<'_, App>, step: CullStep) -> Result<CullStep, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        s.touched();
        revert(&s.library, step)
    })
    .await
    .map_err(text)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_best_picture_is_named_for_what_sets_it_apart() {
        let flat = Aesthetic::from([0.5, 0.5, 0.5, 0.5]);
        let moody = Aesthetic::from([0.5, 0.55, 0.8, 0.5]);
        let sharp = Aesthetic::from([0.5, 0.5, 0.5, 0.7]);
        assert_eq!(leading(&moody, &[flat, moody, sharp]), Some("mood"));
        assert_eq!(leading(&sharp, &[flat, moody, sharp]), Some("technically"));
        assert_eq!(leading(&flat, &[flat, flat]), None, "nothing sets it apart");
        // A few hundredths on one axis is noise, not a reason: 0.03 above the mean of the two.
        let a_hair = Aesthetic::from([0.56, 0.5, 0.5, 0.5]);
        assert_eq!(leading(&a_hair, &[flat, a_hair]), None);
        let clearly = Aesthetic::from([0.62, 0.5, 0.5, 0.5]);
        assert_eq!(leading(&clearly, &[flat, clearly]), Some("composition"));
    }

    #[test]
    fn a_group_too_big_for_a_question_sends_its_best_in_the_order_taken() {
        let rows = [10, 11, 12, 13, 14];
        // The order of which to keep: 13 first, 10 last.
        let order = [13, 11, 14, 12, 10];
        assert_eq!(best_frames(&rows, &order, 3), [11, 13, 14], "the three best, as they were taken");
        assert_eq!(best_frames(&rows, &order, 9), rows);
        assert_eq!(best_frames(&rows, &order, 0), [0u64; 0]);
    }

    /// Asks about `groups` (each the outcome asking about it has) and gives how the run
    /// ended, the groups answered and the groups told of as failed.
    fn asking(groups: &[&str], workers: usize) -> (Result<(), String>, Vec<usize>, Vec<usize>) {
        let todo: Vec<(usize, &str)> = groups.iter().copied().enumerate().collect();
        let (answered, failed) = (Mutex::new(Vec::new()), Mutex::new(Vec::new()));
        let run = ask_each(
            &todo,
            workers,
            &|| false,
            &|&(_, outcome)| match outcome {
                "ok" => Ok(()),
                "no image" => Err(NoAnswer::Group("a photo of the group has no image to send".into())),
                "cut" => Err(NoAnswer::Model(AiError::Truncated)),
                "refused" => Err(NoAnswer::Model(AiError::Refused(None))),
                "no key" => Err(NoAnswer::Model(AiError::NoKey)),
                "limit" => Err(NoAnswer::Model(AiError::Limit("try again at 5 pm".into()))),
                "stopped" => Err(NoAnswer::Model(AiError::Stopped)),
                "disk" => Err(NoAnswer::Store("the answer could not be kept: the disk is full".into())),
                other => panic!("{other}"),
            },
            &|&(i, _), why| lock(if why.is_some() { &failed } else { &answered }).push(i),
        );
        let sorted = |list: Mutex<Vec<usize>>| {
            let mut list = list.into_inner().unwrap();
            list.sort_unstable();
            list
        };
        (run, sorted(answered), sorted(failed))
    }

    #[test]
    fn a_group_with_no_answer_is_counted_and_passed_over() {
        // A frame with no image, an answer cut off, one declined: the run goes on.
        let (run, answered, failed) = asking(&["ok", "no image", "cut", "ok", "refused", "no image", "ok"], 1);
        assert_eq!((run, answered, failed), (Ok(()), vec![0, 3, 6], vec![1, 2, 4, 5]));
        // Groups with no image to send, however many in a row, do not end it: no call
        // failed, and asked again they would stop the next run at the same place.
        let (run, answered, failed) = asking(&["no image", "no image", "no image", "no image", "ok"], 1);
        assert_eq!((run, answered, failed), (Ok(()), vec![4], vec![0, 1, 2, 3]));
    }

    #[test]
    fn three_calls_in_a_row_with_no_answer_end_the_run() {
        let (run, answered, failed) = asking(&["ok", "cut", "refused", "ok", "cut", "no image", "cut", "cut", "ok"], 1);
        let why = run.unwrap_err();
        assert!(why.starts_with("3 groups in a row got no answer") && why.contains("cut off"), "{why}");
        // The answer between the first two failures started the count again; the group
        // with no image neither counted nor did.
        assert_eq!((answered, failed), (vec![0, 3], vec![1, 2, 4, 5, 6, 7]));
    }

    #[test]
    fn what_no_other_group_would_fare_better_with_ends_the_run_at_once() {
        for (fatal, said) in [("no key", "no API key"), ("limit", "usage limit"), ("disk", "disk is full")] {
            let (run, answered, failed) = asking(&["ok", fatal, "ok"], 1);
            assert!(run.unwrap_err().contains(said), "{fatal}");
            assert_eq!((answered, failed), (vec![0], vec![]), "{fatal}: neither answered nor passed over");
        }
        // A call stopped under way ends it with no error, and its group is not told of.
        assert_eq!(asking(&["ok", "stopped", "ok"], 1), (Ok(()), vec![0], vec![]));
    }

    #[test]
    fn a_stop_ends_the_run_before_the_next_group_and_several_are_asked_about_at_once() {
        let stop = AtomicBool::new(false);
        let asked = AtomicUsize::new(0);
        let run = ask_each(
            &[1, 2, 3, 4],
            1,
            &|| stop.load(Ordering::SeqCst),
            &|&group| {
                asked.fetch_add(1, Ordering::SeqCst);
                stop.store(group == 2, Ordering::SeqCst);
                Ok(())
            },
            &|_, _| {},
        );
        assert_eq!((run, asked.load(Ordering::SeqCst)), (Ok(()), 2));

        // Four at a time: every group once, whoever takes it.
        let groups = vec!["ok"; 40];
        let (run, answered, failed) = asking(&groups, 4);
        assert_eq!((run, answered, failed), (Ok(()), (0..40).collect(), vec![]));
        // And an end reached by one of them is the run's.
        let mut groups = vec!["ok"; 40];
        groups[7] = "limit";
        let (run, answered, _) = asking(&groups, 4);
        assert!(run.unwrap_err().contains("usage limit"));
        assert!(!answered.contains(&7));
    }

    #[test]
    fn a_run_is_told_of_only_to_its_own_catalog() {
        let (one, other) = (Arc::new(1u8), Arc::new(2u8));
        let mut held: Held<u8> = Held::new();
        assert_eq!(held.read(&one), RankState::default());
        held.change(&one, |state| *state = RankState { running: true, total: 9, done: 4, ..RankState::default() });
        held.change(&one, |state| state.error = Some("the disk is full".into()));
        assert_eq!((held.read(&one).done, held.read(&one).error.is_some()), (4, true));
        // Another catalog opened: nothing of the first one's run, its error least of all.
        assert!(!held.is_of(&other));
        assert_eq!(held.read(&other), RankState::default());
        // A run of the other starts from nothing.
        let state = held.change(&other, |state| state.total = 2);
        assert_eq!(state, RankState { total: 2, ..RankState::default() });
        assert_eq!(held.read(&one), RankState::default());
    }

    /// Looks without a model: a photo's embedding is its row, and those in `blind` cannot
    /// be looked at. `stops` after that many photos, if at all.
    #[derive(Default)]
    struct FakeEye {
        opened: usize,
        embedded: Vec<u64>,
        blind: Vec<u64>,
        stops: Option<(usize, Arc<AtomicBool>)>,
    }

    impl Eye for FakeEye {
        fn open(&mut self) -> Result<bool, String> {
            self.opened += 1;
            Ok(true)
        }

        fn embed(&mut self, row: u64) -> Result<Vec<f32>, String> {
            if self.blind.contains(&row) {
                return Err("no picture".into());
            }
            self.embedded.push(row);
            if let Some((after, stop)) = &self.stops
                && self.embedded.len() >= *after
            {
                stop.store(true, Ordering::SeqCst);
            }
            Ok(vec![row as f32])
        }

        fn score(&self, embedding: &[f32]) -> Aesthetic {
            Aesthetic::from([embedding[0] / 10.0; 4])
        }
    }

    #[test]
    fn a_look_keeps_what_was_seen_and_counts_what_was_not() {
        let (_dir, library, rows) = burst();
        let [one, two, three, _] = rows;
        let go = AtomicBool::new(false);
        // 2 cannot be read; 4 was never measured, so no view of it could be kept: it is not
        // even looked at, and both are counted.
        let mut eye = FakeEye { blind: vec![two], ..FakeEye::default() };
        let looked = look_at(&library, &rows, &go, &mut eye).unwrap();
        assert_eq!(looked, Looked { failed: 2, stopped: false });
        assert_eq!(eye.embedded, [one, three]);
        let views = library.views(&rows).unwrap();
        assert_eq!(views.len(), 2);
        assert_eq!(views[&three].scores, [three as f32 / 10.0; 4]);

        // Looked at again: only what is missing is tried, and the group says one is unseen.
        let mut eye = FakeEye::default();
        assert_eq!(look_at(&library, &[one, two, three], &go, &mut eye).unwrap(), Looked::default());
        assert_eq!(eye.embedded, [two]);
        // Everything seen: the model is not asked for at all.
        let mut eye = FakeEye::default();
        assert_eq!(look_at(&library, &[one, two, three], &go, &mut eye).unwrap(), Looked::default());
        assert_eq!((eye.opened, eye.embedded.len()), (0, 0));
    }

    #[test]
    fn a_look_stopped_midway_says_so_and_keeps_its_part() {
        let (_dir, library, rows) = burst();
        let [one, two, three, _] = rows;
        let stop = Arc::new(AtomicBool::new(false));
        let mut eye = FakeEye { stops: Some((1, stop.clone())), ..FakeEye::default() };
        let looked = look_at(&library, &[one, two, three], &stop, &mut eye).unwrap();
        assert_eq!(looked, Looked { failed: 0, stopped: true }, "not a group done");
        assert_eq!(library.views(&rows).unwrap().keys().copied().collect::<Vec<_>>(), [one]);
        // Stopped before it began: nothing is opened.
        let mut eye = FakeEye::default();
        assert!(look_at(&library, &[two, three], &stop, &mut eye).unwrap().stopped);
        assert_eq!(eye.opened, 0);
    }

    #[test]
    fn a_group_says_how_many_of_its_photos_the_model_has_not_seen() {
        let (_dir, library, rows) = burst();
        let [one, two, three, _] = rows;
        let filter = Filter::default();
        let id = library.look(one).unwrap().series.unwrap();
        assert_eq!(group_view(&library, &filter, &id).unwrap().unseen, 3);
        let go = AtomicBool::new(false);
        let mut eye = FakeEye { blind: vec![two], ..FakeEye::default() };
        look_at(&library, &[one, two, three], &go, &mut eye).unwrap();
        let view = group_view(&library, &filter, &id).unwrap();
        assert_eq!((view.unseen, view.judge), (1, Judge::Sharpness), "one unseen: the views do not count yet");
        look_at(&library, &[one, two, three], &go, &mut FakeEye::default()).unwrap();
        let view = group_view(&library, &filter, &id).unwrap();
        assert_eq!((view.unseen, view.judge), (0, Judge::Local));
    }

    #[test]
    fn a_variant_of_a_kept_image_is_spared_the_trash() {
        // Rows 1 and 3 are variants of image "a"; 2 is its duplicate "b"; 9 is not known.
        let asset_of = |row: u64| match row {
            1 | 3 => Some("a".to_owned()),
            2 => Some("b".to_owned()),
            _ => None,
        };
        assert_eq!(spared(&[1], &[2, 3, 9], asset_of), (vec![2, 9], vec![3]));
        assert_eq!(spared(&[2], &[1, 3], asset_of), (vec![1, 3], vec![]));
    }

    #[test]
    fn a_mark_edits_only_a_photo_that_lacks_it() {
        let rate = |value| Some(Op::SetRating { base: None, value });
        assert_eq!(Mark::Nothing.edit(Some(3), None), None);
        assert_eq!(Mark::Rating(5).edit(Some(3), None), rate(Some(5)));
        assert_eq!(Mark::Rating(5).edit(Some(5), None), None);
        assert_eq!(Mark::Rating(9).edit(None, None), rate(Some(5)));
        assert_eq!(Mark::Rating(0).edit(Some(2), None), rate(None), "no stars is no rating");
        assert_eq!(Mark::Rating(0).edit(None, None), None);
        let red = Some(Op::SetColor { base: None, value: Some(ColorLabel::Red) });
        assert_eq!(Mark::Color(ColorLabel::Red).edit(Some(3), Some(ColorLabel::Green)), red);
        assert_eq!(Mark::Color(ColorLabel::Red).edit(None, Some(ColorLabel::Red)), None);
        // The rest is marked only where that is what becomes of it.
        assert_eq!(Rest::Rating(1).mark(), Mark::Rating(1));
        assert_eq!(Rest::Color(ColorLabel::Red).mark(), Mark::Color(ColorLabel::Red));
        assert_eq!([Rest::Trash.mark(), Rest::Hide.mark(), Rest::Nothing.mark()], [Mark::Nothing; 3]);
        // No trash where the source removes nothing.
        assert_eq!(Rest::Trash.at(Removal::None), Rest::Hide);
        assert_eq!(Rest::Trash.at(Removal::Files), Rest::Trash);
        assert_eq!(Rest::Rating(1).at(Removal::None), Rest::Rating(1));
    }

    #[test]
    fn photos_picked_before_lead_the_order() {
        assert_eq!(picks_first(vec![2, 0, 1], &[false, true, false]), [1, 2, 0]);
        assert_eq!(picks_first(vec![2, 0, 1], &[true, false, true]), [2, 0, 1]);
        assert_eq!(picks_first(vec![1, 0], &[false, false]), [1, 0]);
    }

    use c1index::Bin;
    use c1model::memory::MemorySource;
    use c1model::{AssetId, EditInfo, Exif, FileInfo, Iptc, PhotoId, Place, SourceId};

    /// Photo `n`, taken `n` seconds in; `asset`: the image it is a variant of.
    fn photo(n: u32, asset: u32) -> Photo {
        Photo {
            id: PhotoId::new(&SourceId("mem".into()), n.to_string()),
            asset: AssetId(asset.to_string()),
            variant_index: u32::from(n != asset),
            file: FileInfo { name: format!("P{n}.RAF"), ext: "raf".into(), ..FileInfo::default() },
            captured_at: Some(1_700_000_000_000 + i64::from(n) * 1000),
            imported_at: None,
            exif: Exif { camera_model: Some("X100VI".into()), ..Exif::default() },
            rating: None,
            color: None,
            keywords: Vec::new(),
            iptc: Iptc::default(),
            place: Place::default(),
            geo: None,
            collections: Vec::new(),
            edit: EditInfo::default(),
            trashed: false,
        }
    }

    fn look(sharpness: f32, hash: u64) -> c1analysis::Analysis {
        let palette = vec![c1analysis::Swatch { rgb: [90, 90, 90], share: 1.0 }];
        c1analysis::Analysis {
            width: 600,
            height: 400,
            hue_stats: c1analysis::stats_from_palette(&palette, 40.0),
            palette,
            chroma: 40.0,
            tint_spread: 30.0,
            brightness: 0.5,
            contrast: 0.2,
            highlights: 0.0,
            shadows: 0.0,
            sharpness,
            hash,
        }
    }

    /// A burst of three (photos 1, 2, 3; 3 is the sharpest) and photo 4, a variant of 1's
    /// image that is no part of the burst. Returns the rows of 1 to 4.
    fn burst() -> (tempfile::TempDir, Library, [u64; 4]) {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path()).unwrap();
        let mut photos: Vec<Photo> = (1..=3).map(|n| photo(n, n)).collect();
        photos.push(Photo { captured_at: Some(1_600_000_000_000), ..photo(4, 1) });
        library.sync(&MemorySource::new("mem", photos, Vec::new())).unwrap();
        let everything = Filter { bin: Bin::Include, ..Filter::default() };
        let all = library.query(&everything, &QueryOptions::default()).unwrap().rows;
        let row = |n: u32| *all.iter().find(|r| library.photo_id(**r).unwrap().ext == n.to_string()).unwrap();
        let rows = [row(1), row(2), row(3), row(4)];
        let h = 0x0f0f_3c3c_5a5a_a5a5u64;
        let key = |n: usize| library.photo_id(rows[n]).unwrap().key();
        library
            .put_analyses(vec![
                (key(0), (1, 1), look(100.0, h)),
                (key(1), (1, 1), look(300.0, h ^ 0b1)),
                (key(2), (1, 1), look(900.0, h ^ 0b11)),
            ])
            .unwrap();
        library.regroup().unwrap();
        assert!(library.look(rows[0]).unwrap().series.is_some(), "1, 2 and 3 are a series");
        (dir, library, rows)
    }

    /// What a pick leaves of the burst: the rows in the trash, hidden, picked, on show when
    /// stacked, and the ratings of 1 to 3.
    type Left = (Vec<u64>, Vec<u64>, Vec<u64>, Vec<u64>, Vec<Option<u8>>);

    fn left(library: &Library, rows: &[u64; 4]) -> Left {
        let of = |is: &dyn Fn(u64) -> bool| rows.iter().copied().filter(|&r| is(r)).collect::<Vec<u64>>();
        (
            of(&|r| library.is_binned(r)),
            of(&|r| library.is_hidden(r)),
            library.kept_of(rows),
            of(&|r| !library.is_binned(r) && library.look(r).is_some_and(|l| !l.stacked)),
            rows[..3].iter().map(|&r| library.photo(r).unwrap().unwrap().rating).collect(),
        )
    }

    #[test]
    fn a_pick_is_taken_back_whole_and_made_again() {
        let (_dir, library, rows) = burst();
        let [one, two, three, _] = rows;
        let before = left(&library, &rows);
        assert_eq!(before, (vec![], vec![], vec![], vec![three], vec![None; 3]), "the sharpest is on show");

        let step =
            apply(&library, Removal::Files, &[two], &[one, two, three], Rest::Trash, Mark::Rating(5), true).unwrap();
        assert_eq!((&step.binned, &step.kept, &step.unkept), (&vec![one, three], &vec![two], &vec![]));
        let picked = (vec![one, three], vec![], vec![two], vec![two], vec![None, Some(5), None]);
        assert_eq!(left(&library, &rows), picked, "2 is kept, rated and on show; 2 named in the rest stays");

        let undone = revert(&library, step.clone()).unwrap();
        assert!(!undone.on);
        assert_eq!(left(&library, &rows), before, "taken back: the trash, the pick, the rating, the stack");
        let redone = revert(&library, undone).unwrap();
        assert_eq!(redone, step);
        assert_eq!(left(&library, &rows), picked, "made again");
    }

    #[test]
    fn a_group_picked_again_keeps_only_the_new_pick() {
        let (_dir, library, rows) = burst();
        let [one, two, three, _] = rows;
        // Picked once with the rest left as it is: all three are still there.
        apply(&library, Removal::Files, &[one], &[two, three], Rest::Nothing, Mark::Nothing, true).unwrap();
        let first = left(&library, &rows);
        assert_eq!(first, (vec![], vec![], vec![one], vec![one], vec![None; 3]));

        // Picked again, another one: the earlier pick is the rest now, and no pick any more.
        let step =
            apply(&library, Removal::Files, &[two], &[one, three], Rest::Rating(1), Mark::Nothing, true).unwrap();
        assert_eq!((&step.kept, &step.unkept), (&vec![two], &vec![one]));
        assert_eq!(left(&library, &rows), (vec![], vec![], vec![two], vec![two], vec![Some(1), None, Some(1)]));

        // To the trash this time, the earlier pick with the rest.
        let again = apply(&library, Removal::Files, &[three], &[one, two], Rest::Trash, Mark::Nothing, true).unwrap();
        assert_eq!(left(&library, &rows).2, [three], "a photo in the trash is no pick");
        revert(&library, again).unwrap();
        // Taken back, the earlier picks are picks again.
        let undone = revert(&library, step).unwrap();
        assert_eq!(left(&library, &rows), first);
        revert(&library, undone).unwrap();
        assert_eq!(left(&library, &rows).2, [two]);
    }

    #[test]
    fn a_pick_keeps_a_photo_and_trashes_only_where_there_is_a_trash() {
        let (_dir, library, rows) = burst();
        let [one, two, three, four] = rows;
        let before = left(&library, &rows);
        // Nothing kept: nothing is done, whatever the rest.
        assert!(apply(&library, Removal::Files, &[], &[one, two, three], Rest::Trash, Mark::Nothing, true).is_err());
        assert_eq!(left(&library, &rows), before);

        // A source that removes nothing: the rest is hidden, not put in a trash it has not got.
        let step = apply(&library, Removal::None, &[three], &[one, two], Rest::Trash, Mark::Nothing, true).unwrap();
        assert_eq!((&step.binned, &step.hidden), (&vec![], &vec![one, two]));
        revert(&library, step).unwrap();
        assert_eq!(left(&library, &rows), before);

        // A source that trashes whole images: 4 is a variant of the image kept, and is left;
        // kept the other way round, it goes with its image, shown in the group or not.
        let step =
            apply(&library, Removal::MarksTrashed, &[one], &[two, four], Rest::Trash, Mark::Nothing, true).unwrap();
        assert_eq!((&step.binned, &step.skipped), (&vec![two], &vec![four]));
        revert(&library, step).unwrap();
        let step =
            apply(&library, Removal::MarksTrashed, &[two], &[one, three], Rest::Trash, Mark::Nothing, true).unwrap();
        assert_eq!(step.binned, [one, three, four]);
        revert(&library, step).unwrap();
        assert_eq!(left(&library, &rows), before);
    }

    /// A grey picture with a red square at (`x`, `y`) (shares of the frame): somewhere for
    /// a subject to sit.
    fn scene(x: f32, y: f32) -> c1image::RgbImage {
        let (w, h) = (300u32, 200u32);
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for j in 0..h {
            for i in 0..w {
                let inside = (i as f32 / w as f32 - x).abs() < 0.05 && (j as f32 / h as f32 - y).abs() < 0.075;
                px.extend_from_slice(if inside { &[210, 30, 30] } else { &[128, 128, 128] });
            }
        }
        c1image::RgbImage::from_raw(w, h, px).unwrap()
    }

    fn picked(rows: &[u64]) -> String {
        format!("{SELECTION}{}", rows.iter().map(u64::to_string).collect::<Vec<_>>().join(","))
    }

    #[test]
    fn a_selection_id_names_its_rows_and_resolves_them_under_the_filter() {
        assert_eq!(selection("pick:3,1,1, 2"), Some(vec![1, 2, 3]));
        assert_eq!(selection("series:4"), None);
        assert_eq!(label("pick:9,4,7"), "pick:4+3");
        assert_eq!(label("dup:4"), "dup:4");
        let (_dir, library, rows) = burst();
        let [one, _, three, four] = rows;
        let id = picked(&[three, four, one]);
        // Oldest first: 4 was taken before the burst.
        assert_eq!(group_rows(&library, &Filter::default(), &id).unwrap(), [four, one, three]);
        let narrower = Filter { rows: Some(vec![one, three]), ..Filter::default() };
        assert_eq!(group_rows(&library, &narrower, &id).unwrap(), [one, three]);
        let many: Vec<u64> = (0..=SELECTION_MAX as u64).collect();
        assert!(group_rows(&library, &Filter::default(), &picked(&many)).is_err());
    }

    #[test]
    fn photos_picked_by_hand_are_chosen_among_afresh_and_leave_the_groups_picks_alone() {
        let (_dir, library, rows) = burst();
        let [one, two, three, _] = rows;
        apply(&library, Removal::Files, &[two], &[one, three], Rest::Nothing, Mark::Nothing, true).unwrap();
        let id = picked(&[one, two, three]);
        let view = group_view(&library, &Filter::default(), &id).unwrap();
        assert!(view.selection);
        assert!(view.photos.iter().all(|p| !p.kept && !p.reasons.contains(&PICKED_BEFORE.to_owned())));
        assert_eq!(view.order[0], three, "the sharpest, not the series' pick");
        assert_eq!(view.suggested[0], three);

        // Kept from the selection: the series keeps its own pick, and it is undone whole.
        let step =
            apply(&library, Removal::Files, &[one], &[two, three], Rest::Rating(1), Mark::Nothing, false).unwrap();
        assert!(step.kept.is_empty() && step.unkept.is_empty());
        assert_eq!(library.kept_of(&rows), [two]);
        revert(&library, step).unwrap();
        assert_eq!(library.kept_of(&rows), [two]);
        assert_eq!(left(&library, &rows).4, [None; 3]);
    }

    #[test]
    fn framing_is_measured_once_per_render_and_a_photo_not_measured_is_measured_too() {
        let (_dir, library, rows) = burst();
        let [one, two, three, four] = rows;
        let calls = AtomicUsize::new(0);
        let at = [(one, 0.14), (two, 1.0 / 3.0), (three, 0.5), (four, 2.0 / 3.0)];
        let stamp = AtomicUsize::new(1);
        let load = |row: u64| {
            calls.fetch_add(1, Ordering::SeqCst);
            let x = at.iter().find(|(r, _)| *r == row)?.1;
            let s = stamp.load(Ordering::SeqCst) as u64;
            Some(((s, s), scene(x, 0.3)))
        };
        assert!(library.look(four).is_none(), "4 is no part of the burst and was never measured");
        assert_eq!(c1index::analyse::frame_with(&library, &rows, 2, &load).unwrap(), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert!(library.look(four).is_some(), "measured on the way");
        assert_eq!(library.framings(&rows).unwrap().len(), 4);
        // Kept: nothing is read again.
        c1index::analyse::frame_with(&library, &rows, 2, &load).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        // Rendered again: measured again.
        library.put_analyses(vec![(library.photo_id(one).unwrap().key(), (7, 7), look(100.0, 0))]).unwrap();
        stamp.store(7, Ordering::SeqCst);
        c1index::analyse::frame_with(&library, &rows, 2, &load).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 5);
        // One that cannot be read is counted.
        library.put_analyses(vec![(library.photo_id(two).unwrap().key(), (8, 8), look(300.0, 0))]).unwrap();
        assert_eq!(c1index::analyse::frame_with(&library, &rows, 1, &|_| None).unwrap(), 1);

        // Every photo framed: the order goes by composition, and each has its score.
        let view = group_view(&library, &Filter::default(), &picked(&[one, three, four])).unwrap();
        assert!(view.photos.iter().all(|p| p.framing.is_some() && p.composition.is_some()), "{:?}", view.judge);
        assert_eq!(view.left_out, 0);
        assert_eq!(view.suggested[0], view.order[0]);
    }
}
