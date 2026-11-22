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
use c1model::{ColorLabel, Op, Photo, Removal};
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
    /// Their rows, the one to keep first: a photo picked before comes before the others.
    order: Vec<u64>,
    /// What the order rests on: what was measured, and the local model's view of the
    /// photos once it has looked at them all. Nothing gives a candidate a `verdict` yet.
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
    // The user's own choice, made before, outranks any measure.
    let picked: Vec<bool> = photos.iter().map(|p| p.kept).collect();
    let order = picks_first(ranked.order, &picked);
    for photo in photos.iter_mut().filter(|p| p.kept) {
        photo.reasons.insert(0, PICKED_BEFORE.to_owned());
    }
    Ok(CullGroupView {
        id: id.to_owned(),
        close: ranked.close && !picked.contains(&true),
        order: order.iter().map(|&i| photos[i].row).collect(),
        photos,
        judge: ranked.judge,
        model: SIGLIP2.installed(&models_root()),
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
) -> Result<CullStep, String> {
    if keep.is_empty() {
        return Err("No photo to keep was named: a pick keeps one at least".into());
    }
    let rest: Vec<u64> = rest.iter().copied().filter(|row| !keep.contains(row)).collect();
    let mut step = CullStep { on: true, ..CullStep::default() };
    match carry_out(library, removes, keep, &rest, action.at(removes), mark, &mut step) {
        Ok(()) => Ok(step),
        Err(e) => match revert(library, step) {
            Ok(_) => Err(e),
            Err(back) => Err(format!("{e}. What was done of it could not be taken back: {back}")),
        },
    }
}

/// [`apply`], with what it did so far written to `step` as it goes.
fn carry_out(
    library: &Library,
    removes: Removal,
    keep: &[u64],
    rest: &[u64],
    action: Rest,
    mark: Mark,
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
    (step.kept, step.unkept) = library.repick(keep, rest, &all).map_err(err)?;
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
    tauri::async_runtime::spawn_blocking(move || {
        s.touched();
        apply(&s.library, s.source.capabilities().removes, &keep, &rest, action, mark)
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

        let step = apply(&library, Removal::Files, &[two], &[one, two, three], Rest::Trash, Mark::Rating(5)).unwrap();
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
        apply(&library, Removal::Files, &[one], &[two, three], Rest::Nothing, Mark::Nothing).unwrap();
        let first = left(&library, &rows);
        assert_eq!(first, (vec![], vec![], vec![one], vec![one], vec![None; 3]));

        // Picked again, another one: the earlier pick is the rest now, and no pick any more.
        let step = apply(&library, Removal::Files, &[two], &[one, three], Rest::Rating(1), Mark::Nothing).unwrap();
        assert_eq!((&step.kept, &step.unkept), (&vec![two], &vec![one]));
        assert_eq!(left(&library, &rows), (vec![], vec![], vec![two], vec![two], vec![Some(1), None, Some(1)]));

        // To the trash this time, the earlier pick with the rest.
        let again = apply(&library, Removal::Files, &[three], &[one, two], Rest::Trash, Mark::Nothing).unwrap();
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
        assert!(apply(&library, Removal::Files, &[], &[one, two, three], Rest::Trash, Mark::Nothing).is_err());
        assert_eq!(left(&library, &rows), before);

        // A source that removes nothing: the rest is hidden, not put in a trash it has not got.
        let step = apply(&library, Removal::None, &[three], &[one, two], Rest::Trash, Mark::Nothing).unwrap();
        assert_eq!((&step.binned, &step.hidden), (&vec![], &vec![one, two]));
        revert(&library, step).unwrap();
        assert_eq!(left(&library, &rows), before);

        // A source that trashes whole images: 4 is a variant of the image kept, and is left;
        // kept the other way round, it goes with its image, shown in the group or not.
        let step = apply(&library, Removal::MarksTrashed, &[one], &[two, four], Rest::Trash, Mark::Nothing).unwrap();
        assert_eq!((&step.binned, &step.skipped), (&vec![two], &vec![four]));
        revert(&library, step).unwrap();
        let step = apply(&library, Removal::MarksTrashed, &[two], &[one, three], Rest::Trash, Mark::Nothing).unwrap();
        assert_eq!(step.binned, [one, three, four]);
        revert(&library, step).unwrap();
        assert_eq!(left(&library, &rows), before);
    }
}
