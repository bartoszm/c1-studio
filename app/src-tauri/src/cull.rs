//! Choosing the best of alike photos: one duplicate group or series at a time, its photos
//! in order of which to keep, and the pick itself — the photos kept, and what becomes of
//! the rest — as one step that can be taken back.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use c1ai::aesthetic::{self, Aesthetic, Prompts};
use c1ai::models::SIGLIP2;
use c1ai::siglip::Embedder;
use c1analysis::{Candidate, Judge};
use c1index::{Filter, GroupSort, Library, QueryOptions, Sort};
use c1model::{ColorLabel, Op, Removal};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::ai::{AiImage, models_root, pixels};
use crate::{App, Lowers, Session, is_current, text};

/// The long edge of the picture the local model is given: it squashes it to 224 itself.
const LOOK_EDGE: u32 = 448;

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
    /// 0 to 1, next to the others of its group.
    score: f32,
    reasons: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct CullGroupView {
    id: String,
    /// Its photos the filter selects, oldest first.
    photos: Vec<CullPhoto>,
    /// Their rows, the one to keep first.
    order: Vec<u64>,
    judge: Judge,
    /// The two best are too close to call.
    close: bool,
    /// The local model is downloaded: it can be asked to look.
    model: bool,
}

/// The rows of a group among the photos `filter` selects, oldest first: every frame that
/// matches, stacked or not.
fn group_rows(library: &Library, filter: &Filter, id: &str) -> Result<Vec<u64>, String> {
    let narrowed = Filter { group: Some(id.to_owned()), stacked: false, ..filter.clone() };
    Ok(library.query(&narrowed, &QueryOptions { sort: Sort::Date, ..QueryOptions::default() }).map_err(text)?.rows)
}

fn aesthetic_of(scores: [f32; 4]) -> Aesthetic {
    let [composition, moment, mood, technical] = scores;
    Aesthetic { composition, moment, mood, technical }
}

/// What ranking calls the frame the model likes best.
const BEST_PICTURE: &str = "best picture";

/// The way `seen` stands out most from the group's `views`: the axis on which it is
/// furthest above their mean.
fn leading(seen: &Aesthetic, views: &[Aesthetic]) -> &'static str {
    let axes = |a: &Aesthetic| [a.composition, a.moment, a.mood, a.technical];
    let n = views.len().max(1) as f32;
    let mut best = ("picture", 0.0);
    for (i, axis) in aesthetic::AXES.iter().enumerate() {
        let above = axes(seen)[i] - views.iter().map(|v| axes(v)[i]).sum::<f32>() / n;
        if above > best.1 {
            best = (if axis.name == "technical" { "technically" } else { axis.name }, above);
        }
    }
    best.0
}

fn group_view(library: &Library, filter: &Filter, id: &str) -> Result<CullGroupView, String> {
    let rows = group_rows(library, filter, id)?;
    let kept: HashSet<u64> = library.kept_of(&rows).into_iter().collect();
    // Only views scored the way photos are scored now: older ones wait to be scored again.
    let views = library.views(&rows).map_err(text)?;
    let seen = |row: u64| views.get(&row).filter(|v| v.version == aesthetic::VERSION).map(|v| aesthetic_of(v.scores));
    let classes = library.class_settings();
    let mut photos = Vec::with_capacity(rows.len());
    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(photo) = library.photo(row).map_err(text)? else { continue };
        let Some(look) = library.look(row) else { continue };
        let a = &look.analysis;
        candidates.push(Candidate {
            sharpness: a.sharpness,
            highlights: a.highlights,
            shadows: a.shadows,
            rating: photo.rating,
            edited: photo.edit.edited,
            aesthetic: seen(row).map(|s| s.overall()),
            verdict: None,
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
            score: 0.0,
            reasons: Vec::new(),
        });
    }
    let ranked = c1analysis::rank(&candidates, &classes);
    for (photo, entry) in photos.iter_mut().zip(ranked.entries) {
        photo.score = entry.score;
        photo.reasons = entry.reasons;
    }
    // The best picture by the model's view is best in some way: said by name.
    let views: Vec<Aesthetic> = photos.iter().filter_map(|p| p.aesthetic).collect();
    for photo in &mut photos {
        if let (Some(seen), Some(reason)) = (photo.aesthetic, photo.reasons.iter_mut().find(|r| *r == BEST_PICTURE)) {
            *reason = format!("best {}", leading(&seen, &views));
        }
    }
    Ok(CullGroupView {
        id: id.to_owned(),
        order: ranked.order.iter().map(|&i| photos[i].row).collect(),
        photos,
        judge: ranked.judge,
        close: ranked.close,
        model: SIGLIP2.installed(&models_root()),
    })
}

/// The local model as ranking uses it: the image tower and the sentences photos are held
/// against. Loaded when first asked for, and let go when the choosing ends.
struct Ranker {
    embedder: Embedder,
    prompts: Prompts,
}

static RANKER: Mutex<Option<Arc<Ranker>>> = Mutex::new(None);

fn ranker(stop: &AtomicBool) -> Result<Arc<Ranker>, String> {
    let mut held = RANKER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ranker) = &*held {
        return Ok(ranker.clone());
    }
    let embedder = Embedder::open(&models_root()).map_err(text)?;
    let prompts = Prompts::load(&embedder, &|| stop.load(Ordering::Relaxed)).map_err(text)?;
    let ranker = Arc::new(Ranker { embedder, prompts });
    *held = Some(ranker.clone());
    Ok(ranker)
}

/// Has the local model look at the photos at `rows` it has not seen yet (or saw rendered
/// otherwise), and keeps what it saw. A view scored another way is scored again from its
/// embedding, without the model. `stop` ends it between photos. Returns how many photos
/// could not be looked at.
fn look_at(s: &Session, ranker: &Ranker, rows: &[u64], stop: &AtomicBool) -> Result<usize, String> {
    let have = s.library.views(rows).map_err(text)?;
    let mut new = Vec::new();
    let mut failed = 0;
    for &row in rows {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let embedding = match have.get(&row) {
            Some(view) if view.version == aesthetic::VERSION => continue,
            Some(view) => view.embedding.clone(),
            None => {
                let picture = s.library.photo_id(row).and_then(|id| pixels(s, &id, AiImage::Thumb, LOOK_EDGE).ok()?);
                match picture.map(|p| ranker.embedder.image(&p)) {
                    Some(Ok(embedding)) => embedding,
                    _ => {
                        failed += 1;
                        continue;
                    }
                }
            }
        };
        let seen = aesthetic::score(&embedding, &ranker.prompts);
        new.push((row, aesthetic::VERSION, embedding, [seen.composition, seen.moment, seen.mood, seen.technical]));
    }
    s.library.put_views(new).map_err(text)?;
    Ok(failed)
}

/// Has the local model look at one group's photos, and gives the group in its new order.
#[tauri::command]
pub(crate) async fn rank_group(app: State<'_, App>, filter: Filter, id: String) -> Result<CullGroupView, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        // One group, asked for by hand: a run stopped before does not stop it.
        let go = AtomicBool::new(false);
        let rows = group_rows(&s.library, &filter, &id)?;
        look_at(&s, &*ranker(&go)?, &rows, &go)?;
        group_view(&s.library, &filter, &id)
    })
    .await
    .map_err(text)?
}

/// How far looking at every group has got.
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct RankState {
    running: bool,
    /// Groups looked at, of how many.
    done: usize,
    total: usize,
    /// Photos that could not be looked at.
    failed: usize,
    /// Why the last run ended early.
    error: Option<String>,
}

// One run at a time, whatever the catalog open: the model is one.
static RANKING: AtomicBool = AtomicBool::new(false);
static RANK_STOP: AtomicBool = AtomicBool::new(false);
static RANK_STATE: Mutex<RankState> =
    Mutex::new(RankState { running: false, done: 0, total: 0, failed: 0, error: None });

fn report_rank(app: &AppHandle, change: impl FnOnce(&mut RankState)) {
    let state = {
        let mut state = RANK_STATE.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut state);
        state.clone()
    };
    let _ = app.emit("ranking", state);
}

fn run_ranking(app: &AppHandle, s: &Arc<Session>, filter: &Filter, series: bool) -> Result<(), String> {
    let groups = s.library.groups_sorted(filter, series, GroupSort::Oldest, false, 0, usize::MAX).map_err(text)?.groups;
    report_rank(app, |state| *state = RankState { running: true, total: groups.len(), ..RankState::default() });
    let ranker = ranker(&RANK_STOP)?;
    for group in groups {
        if RANK_STOP.load(Ordering::Relaxed) || !is_current(app, s) {
            break;
        }
        let rows = group_rows(&s.library, filter, &group.id)?;
        let failed = look_at(s, &ranker, &rows, &RANK_STOP)?;
        report_rank(app, |state| {
            state.done += 1;
            state.failed += failed;
        });
    }
    Ok(())
}

/// Has the local model look at every series (or duplicate group) among the photos `filter`
/// selects, in the background, unless a run is under way. Each group's views are kept as
/// they come, so a run stopped keeps what it saw.
#[tauri::command]
pub(crate) async fn start_ranking(app: AppHandle, filter: Filter, series: bool) -> Result<(), String> {
    let s = app.state::<App>().session()?;
    if RANKING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    RANK_STOP.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let _done = Lowers(&RANKING);
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_ranking(&app, &s, &filter, series)));
        let error = match run {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e),
            Err(_) => Some("looking at the photos crashed".to_owned()),
        };
        if let Some(e) = &error {
            s.log.event(&format!("Ranking alike photos failed: {e}"), &[]);
        }
        report_rank(&app, |state| {
            state.running = false;
            state.error = error;
        });
        let _ = app.emit("ranking-done", ());
    });
    Ok(())
}

#[tauri::command]
pub(crate) fn stop_ranking() {
    RANK_STOP.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub(crate) fn ranking_state() -> RankState {
    RANK_STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The choosing is over: the local model is let go (unless a run still needs it).
#[tauri::command]
pub(crate) fn cull_done() {
    if !RANKING.load(Ordering::SeqCst) {
        *RANKER.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// One group (`dup:<row>`, `series:<row>`) among the photos `filter` selects: its photos in
/// the order taken, and in order of which to keep.
#[tauri::command]
pub(crate) async fn cull_group(app: State<'_, App>, filter: Filter, id: String) -> Result<CullGroupView, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || group_view(&s.library, &filter, &id)).await.map_err(text)?
}

/// What becomes of the photos of a group that were not kept.
#[derive(Clone, Copy, Deserialize)]
pub(crate) enum Rest {
    /// To the app's trash.
    Trash,
    Hide,
    Rating(u8),
    Color(ColorLabel),
    Nothing,
}

/// What the photos kept are marked with.
#[derive(Clone, Copy, Deserialize)]
pub(crate) enum Mark {
    Nothing,
    Rating(u8),
    Color(ColorLabel),
}

/// A pick as made (`on`), or as taken back: what [`cull_revert`] needs to turn it round.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct CullStep {
    on: bool,
    /// The edits that turn the ratings and colours round.
    ops: Vec<(u64, Op)>,
    /// Rows that went to the trash, were hidden, were picked (or the reverse, when not `on`).
    binned: Vec<u64>,
    hidden: Vec<u64>,
    kept: Vec<u64>,
    /// Rows left alone: variants of an image that is kept, where trashing takes the whole image.
    skipped: Vec<u64>,
}

/// `rest` without the rows whose image (`asset_of`) one of `keep` is a variant of, and
/// those rows.
fn spared(keep: &[u64], rest: &[u64], asset_of: impl Fn(u64) -> Option<String>) -> (Vec<u64>, Vec<u64>) {
    let held: HashSet<String> = keep.iter().filter_map(|&row| asset_of(row)).collect();
    rest.iter().partition(|&&row| asset_of(row).is_none_or(|asset| !held.contains(&asset)))
}

fn apply(s: &Session, keep: Vec<u64>, rest: Vec<u64>, action: Rest, mark: Mark) -> Result<CullStep, String> {
    let err = |e: anyhow::Error| format!("{e:#}");
    let library = &s.library;
    let rest: Vec<u64> = rest.into_iter().filter(|row| !keep.contains(row)).collect();
    let mut step = CullStep { on: true, ..CullStep::default() };

    let rating = |value: u8| Some(value.min(5)).filter(|&v| v > 0);
    let all: Vec<u64> = keep.iter().chain(&rest).copied().collect();
    let edits = !matches!((mark, action), (Mark::Nothing, Rest::Trash | Rest::Hide | Rest::Nothing));
    if edits {
        s.touched();
        step.ops = library
            .edit_photos(&all, |row, photo| {
                let kept = keep.contains(&row);
                let op = match (kept, mark, action) {
                    (true, Mark::Rating(v), _) | (false, _, Rest::Rating(v)) => {
                        (photo.rating != rating(v)).then_some(Op::SetRating { base: None, value: rating(v) })
                    }
                    (true, Mark::Color(c), _) | (false, _, Rest::Color(c)) => {
                        (photo.color != Some(c)).then_some(Op::SetColor { base: None, value: Some(c) })
                    }
                    _ => None,
                };
                Ok(op.into_iter().collect())
            })
            .map_err(err)?;
    }
    match action {
        Rest::Trash => {
            let whole = s.source.capabilities().removes == Removal::MarksTrashed;
            let (going, skipped) = if whole {
                spared(&keep, &rest, |row| library.photo(row).ok().flatten().map(|p| p.asset.0))
            } else {
                (rest.clone(), Vec::new())
            };
            step.skipped = skipped;
            step.binned = library.set_binned(&going, true, whole).map_err(err)?;
        }
        Rest::Hide => step.hidden = library.set_hidden(&rest, true).map_err(err)?,
        Rest::Rating(_) | Rest::Color(_) | Rest::Nothing => {}
    }
    step.kept = library.set_kept(&keep, true).map_err(err)?;
    library.restack(&all).map_err(err)?;
    Ok(step)
}

fn revert(s: &Session, step: CullStep) -> Result<CullStep, String> {
    let err = |e: anyhow::Error| format!("{e:#}");
    let library = &s.library;
    let ops = if step.ops.is_empty() {
        Vec::new()
    } else {
        s.touched();
        library.edit(&step.ops).map_err(err)?
    };
    // The rows are the ones that changed: their other variants are among them already.
    let binned = library.set_binned(&step.binned, !step.on, false).map_err(err)?;
    let hidden = library.set_hidden(&step.hidden, !step.on).map_err(err)?;
    let kept = library.set_kept(&step.kept, !step.on).map_err(err)?;
    let all: Vec<u64> = binned.iter().chain(&hidden).chain(&step.kept).copied().collect();
    library.restack(&all).map_err(err)?;
    Ok(CullStep { on: !step.on, ops, binned, hidden, kept, skipped: step.skipped })
}

/// Keeps `keep` of a group and deals with `rest` as `action` says, as one step: the photos
/// kept are marked (`mark`) and stand for their group when stacked. Returns what
/// [`cull_revert`] takes back.
#[tauri::command]
pub(crate) async fn cull_apply(
    app: State<'_, App>,
    keep: Vec<u64>,
    rest: Vec<u64>,
    action: Rest,
    mark: Mark,
) -> Result<CullStep, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || apply(&s, keep, rest, action, mark)).await.map_err(text)?
}

/// Turns a pick round: takes back one made, or makes again one taken back. Returns the
/// step that turns it round once more.
#[tauri::command]
pub(crate) async fn cull_revert(app: State<'_, App>, step: CullStep) -> Result<CullStep, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || revert(&s, step)).await.map_err(text)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_best_picture_is_named_for_what_sets_it_apart() {
        let flat = aesthetic_of([0.5, 0.5, 0.5, 0.5]);
        let moody = aesthetic_of([0.5, 0.55, 0.8, 0.5]);
        let sharp = aesthetic_of([0.5, 0.5, 0.5, 0.7]);
        assert_eq!(leading(&moody, &[flat, moody, sharp]), "mood");
        assert_eq!(leading(&sharp, &[flat, moody, sharp]), "technically");
        assert_eq!(leading(&flat, &[flat, flat]), "picture", "nothing sets it apart");
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
}
