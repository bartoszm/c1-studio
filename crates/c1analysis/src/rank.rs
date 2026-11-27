//! Which photo of a group of alike photos to keep: the members put in order, best first,
//! with a few words on why, and the few worth keeping besides the best.
//!
//! - A model that looked at the whole group ([`Candidate::verdict`]) has the last word.
//! - Else the photographer's own rating: a higher one wins outright.
//! - Else a blurry frame loses to a sharp one.
//! - Else how the frame is composed ([`Candidate::framing`], with the local model's view of
//!   its composition once it has looked), where every frame was measured and they differ.
//! - Else (and between frames composed alike) a score of what was measured: how sharp a
//!   frame is next to the sharpest, less for clipped tones, with a model's view of the
//!   picture ([`Candidate::aesthetic`]) weighed in when it tells the frames apart, and a
//!   little more for a frame already edited.

use serde::{Deserialize, Serialize};

use crate::{ClassSettings, Framing, distance};

/// A frame this many times less detailed than the sharpest scores nothing for sharpness.
const SHARPNESS_SPAN: f32 = 8.0;
/// Taken off a blurry frame when the group has one that is not.
const BLURRY_COST: f32 = 0.5;
/// The most that clipped highlights and shadows take off.
const CLIP_COST: f32 = 0.15;
/// Added for a frame the photographer has edited.
const EDITED_BONUS: f32 = 0.1;
/// Aesthetic scores closer together than this say nothing: the frames are one picture.
const AESTHETIC_SPREAD: f32 = 0.04;
/// How much of the score the aesthetic makes up when it counts.
const AESTHETIC_WEIGHT: f32 = 0.5;
/// The two best within this of each other are too close to call.
const CLOSE: f32 = 0.03;
/// Compositions closer together than this are composed alike: the score orders them. Under
/// this across the whole group, composition says nothing.
const COMPOSITION_MARGIN: f32 = 0.04;
/// The local model's composition, as a difference from the group's mean, is spread this
/// much before it is weighed with what was measured: its views of one scene lie close
/// together.
const MODEL_GAIN: f32 = 2.0;
/// Its share of a frame's composition, once it has looked at every frame.
const MODEL_SHARE: f32 = 0.5;
/// The most frames suggested to keep, and how close to the best one has to be (in
/// composition where that decides, else in score).
const SUGGEST_MAX: usize = 3;
const SUGGEST_SPAN: f32 = 0.08;
/// Hash bits apart, or how far the subject moved (share of the frame), for another frame to
/// be a different picture and not a copy of one suggested already.
const DISTINCT_BITS: u32 = 12;
const DISTINCT_MOVE: f32 = 0.15;

/// Why the best is the best, by the order.
pub const BEST_COMPOSITION: &str = "best composition";
/// Why another frame is suggested besides the best.
pub const FRAMED_OTHERWISE: &str = "worth keeping too: framed otherwise";
pub const ANOTHER_MOMENT: &str = "worth keeping too: another moment";

/// What ranking needs to know of one photo of a group.
#[derive(Clone, Debug, Default)]
pub struct Candidate {
    pub sharpness: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub rating: Option<u8>,
    pub edited: bool,
    /// How good a picture a model finds it, 0 to 1.
    pub aesthetic: Option<f32>,
    /// Its place (0 is best) in the order a model gave the whole group, and why.
    pub verdict: Option<(usize, String)>,
    /// Its perceptual hash ([`crate::Analysis::hash`]): close for frames that look alike.
    pub hash: u64,
    /// How it is framed, measured from its pixels.
    pub framing: Option<Framing>,
    /// The local model's view of its composition, 0 to 1.
    pub seen_composition: Option<f32>,
}

/// What the order rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Judge {
    /// A model that compared the frames.
    Cloud,
    /// How the frames are composed, then what was measured.
    Composition,
    /// The local model's view of each frame, with what was measured.
    Local,
    /// What was measured alone.
    Sharpness,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Entry {
    /// 0 to 1, for showing the frames next to each other; the order is in [`Ranked::order`].
    pub score: f32,
    /// How well composed, 0 to 1, where every frame of the group was measured.
    pub composition: Option<f32>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Ranked {
    /// Indexes into the candidates, best first.
    pub order: Vec<usize>,
    pub judge: Judge,
    /// The two best are too close to call.
    pub close: bool,
    /// Per candidate, in the order given.
    pub entries: Vec<Entry>,
    /// The frames worth keeping, best first: the best, and the few as good as it that are
    /// different pictures.
    pub suggest: Vec<usize>,
}

/// Each frame's composition, where every one was measured: its framing, weighed with the
/// local model's view (spread about the group's mean) once it has looked at all of them.
fn compositions(candidates: &[Candidate]) -> Vec<Option<f32>> {
    let n = candidates.len();
    let Some(framed) = candidates.iter().map(|c| c.framing.map(|f| f.score)).collect::<Option<Vec<f32>>>() else {
        return vec![None; n];
    };
    let Some(seen) = candidates.iter().map(|c| c.seen_composition).collect::<Option<Vec<f32>>>() else {
        return framed.into_iter().map(Some).collect();
    };
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let (f_mean, m_mean) = (mean(&framed), mean(&seen));
    framed
        .iter()
        .zip(&seen)
        .map(|(f, m)| {
            let model = (f_mean + MODEL_GAIN * (m - m_mean)).clamp(0.0, 1.0);
            Some(((1.0 - MODEL_SHARE) * f + MODEL_SHARE * model).clamp(0.0, 1.0))
        })
        .collect()
}

/// Per frame, its tier by `values` (0 is best): a tier starts at its best value and holds
/// the frames within [`COMPOSITION_MARGIN`] of it. Anchored so, frames compare the same
/// way whichever is looked at first.
fn tiers(values: &[f32]) -> Vec<usize> {
    let mut by: Vec<usize> = (0..values.len()).collect();
    by.sort_by(|&a, &b| values[b].total_cmp(&values[a]).then(a.cmp(&b)));
    let mut tier = vec![0; values.len()];
    let (mut at, mut anchor) = (0, by.first().map_or(0.0, |&i| values[i]));
    for i in by {
        if anchor - values[i] > COMPOSITION_MARGIN {
            at += 1;
            anchor = values[i];
        }
        tier[i] = at;
    }
    tier
}

pub fn rank(candidates: &[Candidate], s: &ClassSettings) -> Ranked {
    let n = candidates.len();
    let best_sharp = candidates.iter().map(|c| c.sharpness).fold(0.0f32, f32::max);
    let any_sharp = candidates.iter().any(|c| c.sharpness >= s.blurry_below);
    let blurry = |i: usize| any_sharp && candidates[i].sharpness < s.blurry_below;
    let clip = |c: &Candidate| c.highlights.max(0.0) + c.shadows.max(0.0);
    let least_clip = candidates.iter().map(clip).fold(f32::INFINITY, f32::min);
    let looks: Vec<f32> = candidates.iter().filter_map(|c| c.aesthetic).collect();
    let (low, high) = looks.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    // Every frame needs a view for it to count, and the views have to differ.
    let by_looks = looks.len() == n && n > 1 && high - low >= AESTHETIC_SPREAD;
    let judged = candidates.iter().any(|c| c.verdict.is_some());
    // Edited tells frames apart only where some are not (Capture One counts most as edited).
    let some_unedited = candidates.iter().any(|c| !c.edited);
    let composed = compositions(candidates);
    let framed = n > 1 && composed.iter().all(Option::is_some);
    let comp: Vec<f32> = composed.iter().map(|c| c.unwrap_or(0.0)).collect();
    let (c_low, c_high) = comp.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let by_composition = framed && c_high - c_low >= COMPOSITION_MARGIN;
    let tier = if by_composition { tiers(&comp) } else { vec![0; n] };
    let lone_best = by_composition && tier.iter().filter(|&&t| t == 0).count() == 1;
    // A note on the framing tells frames apart only where not every frame has it.
    let noted: Vec<Vec<&'static str>> =
        candidates.iter().map(|c| c.framing.map(|f| f.notes()).unwrap_or_default()).collect();
    let telling = |note: &str| framed && !noted.iter().all(|notes| notes.contains(&note));

    let mut entries: Vec<Entry> = Vec::with_capacity(n);
    // As worked out, before it is brought into 0 to 1 for showing: what the order goes by.
    let mut scores: Vec<f32> = Vec::with_capacity(n);
    for (i, c) in candidates.iter().enumerate() {
        let mut reasons = Vec::new();
        let ratio = ((c.sharpness.max(0.0) + 1.0) / (best_sharp + 1.0)).ln() / SHARPNESS_SPAN.ln();
        let mut score = (1.0 + ratio).clamp(0.0, 1.0);
        if lone_best && tier[i] == 0 && !blurry(i) {
            reasons.push(BEST_COMPOSITION.to_owned());
        }
        reasons.extend(noted[i].iter().filter(|note| telling(note)).map(|note| (*note).to_owned()));
        if by_looks {
            let look = (c.aesthetic.unwrap_or(low) - low) / (high - low);
            score = (1.0 - AESTHETIC_WEIGHT) * score + AESTHETIC_WEIGHT * look;
            if c.aesthetic == Some(high) {
                reasons.push("best picture".to_owned());
            }
        }
        if n > 1 && c.sharpness == best_sharp {
            reasons.push("sharpest".to_owned());
        }
        if blurry(i) {
            score -= BLURRY_COST;
            reasons.push("blurry".to_owned());
        }
        let clipped = (clip(c) - least_clip).min(CLIP_COST);
        if clipped >= CLIP_COST / 3.0 {
            reasons.push("clipped".to_owned());
        }
        score -= clipped;
        if c.edited && some_unedited {
            score += EDITED_BONUS;
            reasons.push("edited".to_owned());
        }
        if let Some(stars) = c.rating.filter(|&r| r > 0) {
            reasons.insert(0, format!("rated {}", "★".repeat(stars as usize)));
        }
        if let Some((_, why)) = &c.verdict
            && !why.trim().is_empty()
        {
            reasons.insert(0, why.trim().to_owned());
        }
        scores.push(score);
        entries.push(Entry { score: score.clamp(0.0, 1.0), composition: composed[i], reasons });
    }

    let stars = |i: usize| candidates[i].rating.unwrap_or(0);
    let place = |i: usize| candidates[i].verdict.as_ref().map_or(usize::MAX, |v| v.0);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        place(a)
            .cmp(&place(b))
            // Frames the model was not shown follow by what is known of them.
            .then_with(|| stars(b).cmp(&stars(a)))
            .then_with(|| blurry(a).cmp(&blurry(b)))
            .then_with(|| tier[a].cmp(&tier[b]))
            .then_with(|| scores[b].total_cmp(&scores[a]))
            .then_with(|| a.cmp(&b))
    });
    let close = !judged
        && order.len() > 1
        && stars(order[0]) == stars(order[1])
        && blurry(order[0]) == blurry(order[1])
        && tier[order[0]] == tier[order[1]]
        && scores[order[0]] - scores[order[1]] < CLOSE;
    let judge = if judged {
        Judge::Cloud
    } else if by_composition {
        Judge::Composition
    } else if by_looks {
        Judge::Local
    } else {
        Judge::Sharpness
    };
    // Held against the best by composition where that decides, else by the score.
    let value = if by_composition { &comp } else { &scores };
    let suggest = suggest(candidates, &order, value, &blurry);
    if let Some((&first, more)) = suggest.split_first() {
        for &i in more {
            let why = if subject_moved(&candidates[first], &candidates[i]) { FRAMED_OTHERWISE } else { ANOTHER_MOMENT };
            entries[i].reasons.push(why.to_owned());
        }
    }
    Ranked { order, judge, close, entries, suggest }
}

/// Whether the subject of `b` sits elsewhere in the frame than that of `a`.
fn subject_moved(a: &Candidate, b: &Candidate) -> bool {
    match (a.framing.and_then(|f| f.subject), b.framing.and_then(|f| f.subject)) {
        (Some([ax, ay]), Some([bx, by])) => ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt() > DISTINCT_MOVE,
        _ => false,
    }
}

/// The frames worth keeping, best first: the best, then, in `order`, frames as good (`value`
/// within [`SUGGEST_SPAN`] of the best's, the same rating, not blurry where the best is not)
/// that are a different picture from each one taken already. At most [`SUGGEST_MAX`], and
/// never more than half the group.
fn suggest(candidates: &[Candidate], order: &[usize], value: &[f32], blurry: &dyn Fn(usize) -> bool) -> Vec<usize> {
    let Some((&first, rest)) = order.split_first() else { return Vec::new() };
    let most = SUGGEST_MAX.min(candidates.len().div_ceil(2)).max(1);
    let mut out = vec![first];
    for &i in rest {
        if out.len() >= most {
            break;
        }
        let c = &candidates[i];
        let as_good = value[first] - value[i] <= SUGGEST_SPAN
            && c.rating.unwrap_or(0) == candidates[first].rating.unwrap_or(0)
            && (blurry(first) || !blurry(i));
        let different = out
            .iter()
            .all(|&j| distance(candidates[j].hash, c.hash) >= DISTINCT_BITS || subject_moved(&candidates[j], c));
        if as_good && different {
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(sharpness: f32) -> Candidate {
        Candidate { sharpness, ..Candidate::default() }
    }

    /// A frame measured as composed `score` well.
    fn framed(sharpness: f32, score: f32) -> Candidate {
        Candidate { framing: Some(Framing { score, ..Framing::default() }), ..frame(sharpness) }
    }

    fn d() -> ClassSettings {
        ClassSettings::default()
    }

    #[test]
    fn the_sharpest_frame_comes_first_when_nothing_else_is_known() {
        let r = rank(&[frame(900.0), frame(4000.0), frame(2000.0)], &d());
        assert_eq!(r.order, [1, 2, 0]);
        assert_eq!(r.judge, Judge::Sharpness);
        assert!(!r.close);
        assert_eq!(r.entries[1].reasons, ["sharpest"]);
        assert_eq!(r.entries[1].score, 1.0);
        assert!(r.entries[0].score < r.entries[2].score);
    }

    #[test]
    fn a_rating_beats_any_score() {
        let rated = Candidate { rating: Some(4), ..frame(400.0) };
        let r = rank(&[frame(9000.0), rated, Candidate { rating: Some(2), ..frame(9000.0) }], &d());
        assert_eq!(r.order, [1, 2, 0]);
        assert_eq!(r.entries[1].reasons[0], "rated ★★★★");
    }

    #[test]
    fn a_blurry_frame_never_wins_on_looks_while_a_sharp_one_is_there() {
        let s = d();
        let blurry = Candidate { aesthetic: Some(0.9), ..frame(s.blurry_below * 0.9) };
        let sharp = Candidate { aesthetic: Some(0.2), ..frame(s.blurry_below * 1.2) };
        let r = rank(&[blurry, sharp], &s);
        assert_eq!(r.order, [1, 0]);
        assert!(r.entries[0].reasons.contains(&"blurry".to_owned()));
        // A group of only blurry frames is not marked down: the least blurry is the best there is.
        let r = rank(&[frame(10.0), frame(20.0)], &s);
        assert_eq!(r.order, [1, 0]);
        assert!(r.entries.iter().all(|e| !e.reasons.contains(&"blurry".to_owned())));
    }

    #[test]
    fn looks_count_only_when_every_frame_has_them_and_they_differ() {
        let with = |sharpness, look| Candidate { aesthetic: Some(look), ..frame(sharpness) };
        // The softer frame is the better picture by far.
        let r = rank(&[with(4000.0, 0.30), with(3000.0, 0.70)], &d());
        assert_eq!((r.order.as_slice(), r.judge), ([1, 0].as_slice(), Judge::Local));
        assert_eq!(r.entries[1].reasons, ["best picture"]);
        // Views a hair apart say nothing: sharpness decides.
        let r = rank(&[with(4000.0, 0.50), with(3000.0, 0.51)], &d());
        assert_eq!((r.order.as_slice(), r.judge), ([0, 1].as_slice(), Judge::Sharpness));
        // One frame not looked at yet: the same.
        let r = rank(&[with(4000.0, 0.30), frame(3000.0), with(100.0, 0.9)], &d());
        assert_eq!((r.order[0], r.judge), (0, Judge::Sharpness));
    }

    #[test]
    fn a_verdict_sets_the_order_and_frames_it_left_out_follow() {
        let told = |sharpness, place: usize, why: &str| Candidate {
            verdict: Some((place, why.to_owned())),
            ..frame(sharpness)
        };
        let r = rank(
            &[told(9000.0, 1, "eyes half closed"), frame(8000.0), told(500.0, 0, " the gesture "), frame(8500.0)],
            &d(),
        );
        assert_eq!(r.order, [2, 0, 3, 1]);
        assert_eq!(r.judge, Judge::Cloud);
        assert!(!r.close);
        assert_eq!(r.entries[2].reasons[0], "the gesture");
    }

    #[test]
    fn near_equal_frames_are_too_close_to_call_and_clipping_and_edits_tell() {
        assert!(rank(&[frame(4000.0), frame(3990.0)], &d()).close);
        assert!(!rank(&[frame(4000.0), frame(1000.0)], &d()).close);
        // A rating apart is never close.
        assert!(!rank(&[Candidate { rating: Some(1), ..frame(4000.0) }, frame(4000.0)], &d()).close);
        let clipped = Candidate { highlights: 0.2, ..frame(4000.0) };
        let r = rank(&[clipped, frame(3900.0)], &d());
        assert_eq!(r.order, [1, 0]);
        assert!(r.entries[0].reasons.contains(&"clipped".to_owned()));
        let edited = Candidate { edited: true, ..frame(3600.0) };
        let r = rank(&[frame(4000.0), edited], &d());
        assert_eq!(r.order, [1, 0]);
        assert_eq!(r.entries[1].reasons, ["edited"]);
        // Every frame edited: it sets none apart.
        let both = |sharpness| Candidate { edited: true, ..frame(sharpness) };
        let r = rank(&[both(4000.0), both(3600.0)], &d());
        assert_eq!(r.order, [0, 1]);
        assert_eq!(
            (r.entries[0].reasons.as_slice(), r.entries[1].reasons.len()),
            (["sharpest".to_owned()].as_slice(), 0)
        );
    }

    #[test]
    fn one_frame_or_none_ranks_without_reasons() {
        assert!(rank(&[], &d()).order.is_empty());
        let r = rank(&[frame(0.0)], &d());
        assert_eq!((r.order.as_slice(), r.close), ([0].as_slice(), false));
        assert!(r.entries[0].reasons.is_empty());
        assert_eq!(r.suggest, [0]);
    }

    #[test]
    fn composition_leads_when_every_frame_has_one() {
        let r = rank(&[framed(4000.0, 0.5), framed(3000.0, 0.8)], &d());
        assert_eq!((r.order.as_slice(), r.judge), ([1, 0].as_slice(), Judge::Composition));
        assert_eq!(r.entries[1].reasons[0], BEST_COMPOSITION);
        assert_eq!(r.entries[1].composition, Some(0.8));
        assert!(!r.close);
    }

    #[test]
    fn compositions_within_the_margin_fall_back_to_the_score() {
        let r = rank(&[framed(3000.0, 0.80), framed(4000.0, 0.78), framed(5000.0, 0.40)], &d());
        assert_eq!((r.order.as_slice(), r.judge), ([1, 0, 2].as_slice(), Judge::Composition));
        assert!(r.entries.iter().all(|e| !e.reasons.contains(&BEST_COMPOSITION.to_owned())));
    }

    #[test]
    fn compositions_all_alike_rank_as_before() {
        let r = rank(&[framed(3000.0, 0.50), framed(4000.0, 0.51)], &d());
        assert_eq!((r.order.as_slice(), r.judge), ([1, 0].as_slice(), Judge::Sharpness));
    }

    #[test]
    fn a_blurry_frame_loses_even_with_the_best_composition() {
        let s = d();
        let r = rank(&[framed(s.blurry_below * 0.9, 0.9), framed(s.blurry_below * 1.2, 0.3)], &s);
        assert_eq!(r.order, [1, 0]);
        assert!(!r.entries[0].reasons.contains(&BEST_COMPOSITION.to_owned()));
    }

    #[test]
    fn a_rating_and_a_verdict_still_come_first() {
        let rated = Candidate { rating: Some(3), ..framed(4000.0, 0.3) };
        let r = rank(&[framed(4000.0, 0.9), rated], &d());
        assert_eq!(r.order, [1, 0]);
        let told = Candidate { verdict: Some((0, "the look".into())), ..framed(4000.0, 0.3) };
        let r = rank(&[framed(4000.0, 0.9), told], &d());
        assert_eq!((r.order[0], r.judge), (1, Judge::Cloud));
    }

    #[test]
    fn one_frame_without_framing_turns_composition_off() {
        let r = rank(&[framed(3000.0, 0.9), frame(4000.0), framed(2000.0, 0.1)], &d());
        assert_eq!((r.order[0], r.judge), (1, Judge::Sharpness));
        assert!(r.entries.iter().all(|e| e.composition.is_none()));
    }

    #[test]
    fn the_models_composition_is_blended() {
        let seen = |sharpness, model| Candidate { seen_composition: Some(model), ..framed(sharpness, 0.6) };
        let r = rank(&[seen(4000.0, 0.50), seen(3000.0, 0.70)], &d());
        assert_eq!((r.order.as_slice(), r.judge), ([1, 0].as_slice(), Judge::Composition));
        let [a, b] = [r.entries[0].composition.unwrap(), r.entries[1].composition.unwrap()];
        assert!((a - 0.5).abs() < 1e-4 && (b - 0.7).abs() < 1e-4, "{a} {b}");
    }

    #[test]
    fn faults_are_said_only_where_they_tell_frames_apart() {
        let with = |score, tilt: Option<f32>, clutter| Candidate {
            framing: Some(Framing { score, tilt, clutter, ..Framing::default() }),
            ..frame(4000.0)
        };
        let r = rank(&[with(0.8, None, 0.9), with(0.4, Some(4.0), 0.9)], &d());
        assert!(r.entries[1].reasons.contains(&crate::TILTED.to_owned()));
        // Every frame has busy edges: it sets neither apart.
        assert!(r.entries.iter().all(|e| !e.reasons.contains(&crate::BUSY_EDGES.to_owned())));
    }

    #[test]
    fn tiers_are_anchored_and_the_order_is_steady() {
        let r = rank(&[framed(1000.0, 0.50), framed(2000.0, 0.47), framed(3000.0, 0.44), framed(4000.0, 0.41)], &d());
        assert_eq!(r.order, [1, 0, 3, 2]);
        assert_eq!(tiers(&[0.50, 0.47, 0.44, 0.41]), [0, 0, 1, 1]);
    }

    #[test]
    fn close_needs_the_same_tier() {
        assert!(!rank(&[framed(4000.0, 0.6), framed(4000.0, 0.8)], &d()).close);
        assert!(rank(&[framed(4000.0, 0.8), framed(3990.0, 0.79)], &d()).close);
    }

    #[test]
    fn copies_give_one_suggestion_and_different_pictures_more() {
        let hashed = |sharpness, hash| Candidate { hash, ..frame(sharpness) };
        // Near-identical frames: keep the best.
        let r = rank(&[hashed(4000.0, 0), hashed(3900.0, 1), hashed(3800.0, 3)], &d());
        assert_eq!(r.suggest, [0]);
        // A different picture as good as the best: both.
        let r = rank(&[hashed(4000.0, 0), hashed(3900.0, u64::MAX), hashed(3800.0, 1)], &d());
        assert_eq!(r.suggest, [0, 1]);
        assert_eq!(r.entries[1].reasons.last().map(String::as_str), Some(ANOTHER_MOMENT));
        // Not one much worse.
        let r = rank(&[hashed(4000.0, 0), hashed(500.0, u64::MAX)], &d());
        assert_eq!(r.suggest, [0]);
    }

    #[test]
    fn a_blurry_or_lower_rated_frame_is_never_suggested_and_few_are() {
        let s = d();
        let hashed = |sharpness, hash: u64| Candidate { hash, ..frame(sharpness) };
        let r = rank(&[hashed(4000.0, 0), hashed(s.blurry_below * 0.9, u64::MAX), hashed(3900.0, 0xFFFF_0000)], &s);
        assert_eq!(r.suggest, [0, 2]);
        let rated = Candidate { rating: Some(2), ..hashed(4000.0, 0) };
        let r = rank(&[rated, hashed(4000.0, u64::MAX), hashed(4000.0, 0xFFFF_0000)], &s);
        assert_eq!(r.suggest, [0]);
        // Six different pictures, all as good: three at most.
        let six: Vec<Candidate> = (0..6).map(|i| hashed(4000.0, 0xFFFF_u64 << (i * 10))).collect();
        assert_eq!(rank(&six, &s).suggest.len(), 3);
        // Of three, two at most: half the group, rounded up.
        assert_eq!(rank(&six[..3], &s).suggest.len(), 2);
    }

    #[test]
    fn a_subject_moved_makes_another_picture() {
        let at = |score, x: f32| Candidate {
            framing: Some(Framing { score, subject: Some([x, 0.5]), ..Framing::default() }),
            ..frame(4000.0)
        };
        let r = rank(&[at(0.80, 0.33), at(0.76, 0.66), at(0.79, 0.35)], &d());
        assert_eq!(r.suggest, [0, 1]);
        assert_eq!(r.entries[1].reasons.last().map(String::as_str), Some(FRAMED_OTHERWISE));
    }
}
