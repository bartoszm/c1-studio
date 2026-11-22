//! Which photo of a group of alike photos to keep: the members put in order, best first,
//! with a few words on why.
//!
//! - A model that looked at the whole group ([`Candidate::verdict`]) has the last word.
//! - Else the photographer's own rating: a higher one wins outright.
//! - Else a score of what was measured: how sharp a frame is next to the sharpest, less for
//!   clipped tones, with a model's view of the picture ([`Candidate::aesthetic`]) weighed
//!   in when it tells the frames apart, and a little more for a frame already edited.

use serde::{Deserialize, Serialize};

use crate::ClassSettings;

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
}

/// What the order rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Judge {
    /// A model that compared the frames.
    Cloud,
    /// The local model's view of each frame, with what was measured.
    Local,
    /// What was measured alone.
    Sharpness,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Entry {
    /// 0 to 1, for showing the frames next to each other; the order is in [`Ranked::order`].
    pub score: f32,
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
}

pub fn rank(candidates: &[Candidate], s: &ClassSettings) -> Ranked {
    let n = candidates.len();
    let best_sharp = candidates.iter().map(|c| c.sharpness).fold(0.0f32, f32::max);
    let any_sharp = candidates.iter().any(|c| c.sharpness >= s.blurry_below);
    let clip = |c: &Candidate| c.highlights.max(0.0) + c.shadows.max(0.0);
    let least_clip = candidates.iter().map(clip).fold(f32::INFINITY, f32::min);
    let looks: Vec<f32> = candidates.iter().filter_map(|c| c.aesthetic).collect();
    let (low, high) = looks.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    // Every frame needs a view for it to count, and the views have to differ.
    let by_looks = looks.len() == n && n > 1 && high - low >= AESTHETIC_SPREAD;
    let judged = candidates.iter().any(|c| c.verdict.is_some());

    let mut entries: Vec<Entry> = Vec::with_capacity(n);
    // As worked out, before it is brought into 0 to 1 for showing: what the order goes by.
    let mut scores: Vec<f32> = Vec::with_capacity(n);
    for c in candidates {
        let mut reasons = Vec::new();
        let ratio = ((c.sharpness.max(0.0) + 1.0) / (best_sharp + 1.0)).ln() / SHARPNESS_SPAN.ln();
        let mut score = (1.0 + ratio).clamp(0.0, 1.0);
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
        if any_sharp && c.sharpness < s.blurry_below {
            score -= BLURRY_COST;
            reasons.push("blurry".to_owned());
        }
        let clipped = (clip(c) - least_clip).min(CLIP_COST);
        if clipped >= CLIP_COST / 3.0 {
            reasons.push("clipped".to_owned());
        }
        score -= clipped;
        if c.edited {
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
        entries.push(Entry { score: score.clamp(0.0, 1.0), reasons });
    }

    let stars = |i: usize| candidates[i].rating.unwrap_or(0);
    let place = |i: usize| candidates[i].verdict.as_ref().map_or(usize::MAX, |v| v.0);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        place(a)
            .cmp(&place(b))
            // Frames the model was not shown follow by what is known of them.
            .then_with(|| stars(b).cmp(&stars(a)))
            .then_with(|| scores[b].total_cmp(&scores[a]))
            .then_with(|| a.cmp(&b))
    });
    let close =
        !judged && order.len() > 1 && stars(order[0]) == stars(order[1]) && scores[order[0]] - scores[order[1]] < CLOSE;
    let judge = if judged {
        Judge::Cloud
    } else if by_looks {
        Judge::Local
    } else {
        Judge::Sharpness
    };
    Ranked { order, judge, close, entries }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(sharpness: f32) -> Candidate {
        Candidate { sharpness, ..Candidate::default() }
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
    }

    #[test]
    fn one_frame_or_none_ranks_without_reasons() {
        assert!(rank(&[], &d()).order.is_empty());
        let r = rank(&[frame(0.0)], &d());
        assert_eq!((r.order.as_slice(), r.close), ([0].as_slice(), false));
        assert!(r.entries[0].reasons.is_empty());
    }
}
