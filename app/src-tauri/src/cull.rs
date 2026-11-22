//! Choosing the best of alike photos: one duplicate group or series at a time, its photos
//! in order of which to keep, and the pick itself — the photos kept, and what becomes of
//! the rest — as one step that can be taken back.

use std::collections::HashSet;

use c1analysis::{Candidate, Judge};
use c1index::{Filter, Library, QueryOptions, Sort};
use c1model::{ColorLabel, Op, Removal};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{App, Session, text};

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
}

fn group_view(library: &Library, filter: &Filter, id: &str) -> Result<CullGroupView, String> {
    // Every frame of the group that matches, stacked or not.
    let narrowed = Filter { group: Some(id.to_owned()), stacked: false, ..filter.clone() };
    let rows =
        library.query(&narrowed, &QueryOptions { sort: Sort::Date, ..QueryOptions::default() }).map_err(text)?.rows;
    let kept: HashSet<u64> = library.kept_of(&rows).into_iter().collect();
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
            aesthetic: None,
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
            score: 0.0,
            reasons: Vec::new(),
        });
    }
    let ranked = c1analysis::rank(&candidates, &classes);
    for (photo, entry) in photos.iter_mut().zip(ranked.entries) {
        photo.score = entry.score;
        photo.reasons = entry.reasons;
    }
    Ok(CullGroupView {
        id: id.to_owned(),
        order: ranked.order.iter().map(|&i| photos[i].row).collect(),
        photos,
        judge: ranked.judge,
        close: ranked.close,
    })
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
