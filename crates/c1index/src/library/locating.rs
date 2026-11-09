//! Locating photos with a vision model: what the catalog knows of where a photo was
//! taken (the photos with a position of their own taken around it, the places of those
//! without, the trip it belongs to, home), sent with the photo as clues; and a guess
//! checked against it.

use std::collections::HashMap;
use std::sync::Arc;

use c1ai::PlaceGuess;
use c1geo::gazetteer;
use c1model::Place;

use super::Library;
use super::describing::DOUBTED;
use super::placing::{MAX_KMH, between, km};
use crate::gpx::Fix;
use crate::schema::NO_PLACE;
use crate::trips::Home;

/// How far around a photo, in time, the photos told of as clues are looked for, unless
/// a run says otherwise.
pub const LOCATE_WINDOW_MINUTES: i64 = 180;

/// A guessed position this far from the photos taken around it, beyond what the time
/// between them allows, is doubted.
const REACH_SLACK_KM: f64 = 5.0;

/// How far around a photo, in time, the reach of a guessed position is checked: past a
/// few hours, at [`MAX_KMH`], nearly anywhere within a country is in reach. Fixed, not the
/// run's clue window: a check worth making whatever was sent.
const REACH_WINDOW_MINUTES: i64 = 180;

/// A span of time as said in a clue: "40 s", "12 min", "3 h 5 min", "2 days".
fn span(ms: i64) -> String {
    let s = ms.abs() / 1000;
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        3600..172_800 => match (s / 3600, s % 3600 / 60) {
            (h, 0) => format!("{h} h"),
            (h, m) => format!("{h} h {m} min"),
        },
        _ => format!("{} days", s / 86_400),
    }
}

/// A place as said in a clue: its levels, the finest first.
fn said(place: &Place) -> Option<String> {
    let levels: Vec<&str> = [&place.location, &place.city, &place.state, &place.country]
        .into_iter()
        .flatten()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    (!levels.is_empty()).then(|| levels.join(", "))
}

/// A place path as said in a clue: `Poland|—|Kraków` → "Kraków, Poland".
fn said_path(path: &str) -> String {
    let mut levels: Vec<&str> = path.split('|').map(str::trim).filter(|l| !l.is_empty() && *l != NO_PLACE).collect();
    levels.reverse();
    levels.join(", ")
}

/// What lies at a position, as a clue: "50.0614, 19.9366 (Kraków, Poland)".
fn at_point(lat: f64, lon: f64) -> String {
    let named = gazetteer().reverse(lat, lon).map(|f| match f.city {
        Some(city) => format!("{city}, {}", f.country),
        None => f.country,
    });
    match named {
        Some(named) => format!("{lat:.4}, {lon:.4} ({named})"),
        None => format!("{lat:.4}, {lon:.4}"),
    }
}

/// What a photo is put into a scene by: when it was taken, the place written on it (its
/// path), the trip it is on.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SceneShot {
    pub row: u64,
    pub at: Option<i64>,
    pub place: Option<String>,
    pub trip: Option<usize>,
}

/// Photos taken one after another at one place, for a model to locate together: no two
/// more than `gap` ms apart, the whole at most `span` ms, on one trip, with no two places
/// written that differ (a photo with none goes with any). A photo with no capture time is
/// a scene of its own. Scenes come in the order taken; rows within them too.
pub(super) fn scenes(mut shots: Vec<SceneShot>, gap: i64, span: i64) -> Vec<Vec<u64>> {
    shots.sort_by_key(|s| (s.at.is_none(), s.at, s.row));
    let mut out: Vec<Vec<u64>> = Vec::new();
    // The scene being made: its first and last time, its place, its trip.
    let mut open: Option<(i64, i64, Option<String>, Option<usize>)> = None;
    for s in shots {
        let joins = match (&open, s.at) {
            (Some((first, last, place, trip)), Some(at)) => {
                at - last <= gap
                    && at - first <= span
                    && *trip == s.trip
                    && !matches!((place, &s.place), (Some(a), Some(b)) if a != b)
            }
            _ => false,
        };
        if joins {
            let scene = open.as_mut().expect("joins an open scene");
            scene.1 = s.at.expect("joins with a time");
            if scene.2.is_none() {
                scene.2 = s.place.clone();
            }
            out.last_mut().expect("an open scene is listed").push(s.row);
        } else {
            open = s.at.map(|at| (at, at, s.place.clone(), s.trip));
            out.push(vec![s.row]);
        }
    }
    out
}

/// Which of `n` photos a scene sends, at most `max`: spread evenly, first and last in.
pub fn sample(n: usize, max: usize) -> Vec<usize> {
    if n <= max || max < 2 {
        return (0..n.min(max.max(1))).collect();
    }
    (0..max).map(|i| i * (n - 1) / (max - 1)).collect()
}

/// The photos with a position of their own nearest before (or at) and after `at`, each
/// within `window` ms.
fn around(fixes: &[Fix], at: i64, window: i64) -> (Option<&Fix>, Option<&Fix>) {
    let i = fixes.partition_point(|f| f.at <= at);
    let before = i.checked_sub(1).and_then(|i| fixes.get(i)).filter(|f| at - f.at <= window);
    let after = fixes.get(i).filter(|f| f.at - at <= window);
    (before, after)
}

/// What the photos with a position taken around `at` say, as clue lines.
fn fix_clues(fixes: &[Fix], at: i64, window: i64) -> Vec<String> {
    let mut lines = Vec::new();
    let (before, after) = around(fixes, at, window);
    if let Some(f) = before {
        lines.push(match at - f.at {
            0 => format!("A photo taken at the same moment has the position {}", at_point(f.lat, f.lon)),
            d => format!("{} before it, the photographer was at {}", span(d), at_point(f.lat, f.lon)),
        });
    }
    if let Some(f) = after {
        lines.push(format!("{} after it, at {}", span(f.at - at), at_point(f.lat, f.lon)));
    }
    if let (Some(b), Some(a)) = (before, after)
        && b.at < at
        && a.at > at
    {
        match between(&[*b, *a], at, window * 2) {
            Some((lat, lon)) if km((b.lat, b.lon), (a.lat, a.lon)) > 0.2 => {
                lines.push(format!("On a straight line between the two, by time: about {lat:.4}, {lon:.4}"))
            }
            Some(_) => {}
            None => lines.push(format!(
                "The two are {:.0} km apart: the photographer travelled fast (a flight, a train) in between",
                km((b.lat, b.lon), (a.lat, a.lon))
            )),
        }
    }
    lines
}

/// What the places written on photos taken around `at` say, as clue lines: the nearest
/// before and after within `window`, where they differ from the photo's own.
fn place_clues(placed: &[(i64, Arc<str>, u64)], (row, at): (u64, i64), own: Option<&str>, window: i64) -> Vec<String> {
    let i = placed.partition_point(|p| p.0 < at);
    let before = placed[..i].iter().rev().find(|p| p.2 != row);
    let after = placed[i..].iter().find(|p| p.2 != row);
    let mut lines = Vec::new();
    for (p, when) in [(before, "before"), (after, "after")] {
        let Some((t, path, _)) = p.filter(|p| (p.0 - at).abs() <= window) else { continue };
        if own == Some(&**path) || lines.iter().any(|l: &String| l.ends_with(&said_path(path))) {
            continue;
        }
        lines.push(format!("{} {when} it, a photo was placed in {}", span(t - at), said_path(path)));
    }
    lines
}

/// The point of a guess is doubted when the photos with a position taken around it
/// (within `window`) were too far away to have got there in time.
fn unreachable(fixes: &[Fix], at: i64, window: i64, lat: f64, lon: f64, radius_km: f64) -> Option<String> {
    let (before, after) = around(fixes, at, window);
    [before, after].into_iter().flatten().find_map(|f| {
        let hours = (at - f.at).abs() as f64 / 3_600_000.0;
        let d = km((f.lat, f.lon), (lat, lon));
        (d > radius_km + REACH_SLACK_KM + MAX_KMH * hours).then(|| {
            let when = if f.at <= at { "before" } else { "after" };
            format!("{d:.0} km from the photo taken {} {when}", span(at - f.at))
        })
    })
}

impl Library {
    /// For each of `rows`, what the catalog knows of where it was taken, a line each:
    /// its place as written, the photos with a position of their own taken within
    /// `window_minutes` before and after it (with the point between them), the places of
    /// photos taken around it, its trip and home. Photos with no capture time get their
    /// place alone.
    pub fn locate_context(&self, rows: &[u64], window_minutes: i64) -> anyhow::Result<HashMap<u64, Vec<String>>> {
        let window = window_minutes.clamp(1, 2880) * 60_000;
        let searcher = self.reader.searcher();
        let fixes = self.fixes_cached()?;
        let placed = self.placed(&searcher)?;
        let (trips, home) = {
            let cache = self.trip_cache()?;
            let cache = cache.as_ref().expect("trip_cache fills it");
            let home = cache.settings.home.as_ref().and_then(Home::normalised).or_else(|| cache.guess.clone());
            let trips: Vec<(i64, i64, String)> = cache
                .trips
                .iter()
                .filter(|(t, _, _)| t.kind != crate::trips::Kind::Dismissed)
                .map(|(t, label, _)| (t.start, t.end, t.name.clone().unwrap_or_else(|| label.clone())))
                .collect();
            (trips, home)
        };
        let mut out = HashMap::new();
        for &row in rows {
            let Some(photo) = self.photo(row)? else { continue };
            let mut lines = Vec::new();
            if let Some(place) = said(&photo.place) {
                let code = photo.place.country_code.as_deref().map(|c| format!(" ({c})")).unwrap_or_default();
                lines.push(format!("The photographer placed it in {place}{code}"));
            }
            if let Some(at) = photo.captured_at {
                lines.extend(fix_clues(&fixes, at, window));
                let own = crate::schema::place_tree(&photo.place).pop();
                lines.extend(place_clues(&placed, (row, at), own.as_deref(), window));
                if let Some((start, end, name)) = trips.iter().find(|(s, e, _)| (*s..=*e).contains(&at)) {
                    let days = (end - start) / 86_400_000 + 1;
                    lines.push(format!("Taken on a trip: {name} ({days} days)"));
                }
            }
            if let Some(home) = &home {
                let place: Vec<&str> = [&home.city, &home.country].into_iter().flatten().map(String::as_str).collect();
                if !place.is_empty() {
                    lines.push(format!("The photographer lives in {}", place.join(", ")));
                }
            }
            out.insert(row, lines);
        }
        Ok(out)
    }

    /// Doubts a guessed point the photographer could not have reached in time from the
    /// photos with a position taken around it: its p is left at most [`DOUBTED`].
    pub(super) fn check_reach(&self, row: u64, guess: &mut PlaceGuess) -> anyhow::Result<Vec<String>> {
        let Some(point) = guess.point.as_mut() else { return Ok(Vec::new()) };
        let Some(at) = self.photo(row)?.and_then(|p| p.captured_at) else { return Ok(Vec::new()) };
        let fixes = self.fixes_cached()?;
        let window = REACH_WINDOW_MINUTES * 60_000;
        let Some(why) = unreachable(&fixes, at, window, point.lat, point.lon, point.radius_km as f64) else {
            return Ok(Vec::new());
        };
        if point.p > DOUBTED {
            let line = format!("the position is {why}: {:.0}% lowered to {:.0}%", point.p * 100.0, DOUBTED * 100.0);
            point.p = DOUBTED;
            Ok(vec![line])
        } else {
            Ok(vec![format!("the position is {why}")])
        }
    }

    /// `rows` as scenes to locate together ([`scenes`]): no two more than `gap_minutes`
    /// apart, each spanning at most `span_minutes`.
    pub fn locate_scenes(&self, rows: &[u64], gap_minutes: i64, span_minutes: i64) -> anyhow::Result<Vec<Vec<u64>>> {
        let trips: Vec<(i64, i64)> = {
            let cache = self.trip_cache()?;
            let cache = cache.as_ref().expect("trip_cache fills it");
            cache
                .trips
                .iter()
                .filter(|(t, _, _)| t.kind != crate::trips::Kind::Dismissed)
                .map(|(t, _, _)| (t.start, t.end))
                .collect()
        };
        let mut shots = Vec::with_capacity(rows.len());
        for &row in rows {
            let Some(photo) = self.photo(row)? else { continue };
            let at = photo.captured_at;
            shots.push(SceneShot {
                row,
                at,
                place: crate::schema::place_tree(&photo.place).pop(),
                trip: at.and_then(|at| trips.iter().position(|(s, e)| (*s..=*e).contains(&at))),
            });
        }
        let minutes = |m: i64| m.clamp(1, 2880) * 60_000;
        Ok(scenes(shots, minutes(gap_minutes), minutes(span_minutes)))
    }

    /// [`Library::fixes`], read once per index generation: a run checks every answer
    /// against them.
    fn fixes_cached(&self) -> anyhow::Result<Arc<Vec<Fix>>> {
        let searcher = self.reader.searcher();
        let generation = searcher.generation().generation_id();
        if let Some((g, fixes)) = self.fix_cache.lock().unwrap().as_ref()
            && *g == generation
        {
            return Ok(Arc::clone(fixes));
        }
        let fixes = Arc::new(self.fixes(&searcher)?);
        *self.fix_cache.lock().unwrap() = Some((generation, Arc::clone(&fixes)));
        Ok(fixes)
    }

    /// Positions were marked as guesses or freed of the mark: the fixes are read again.
    pub(super) fn fixes_changed(&self) {
        *self.fix_cache.lock().unwrap() = None;
    }

    /// Of `rows`, those with no position of their own: none, or only their place's (≈).
    /// A locating run takes these.
    pub fn unpositioned(&self, rows: &[u64]) -> anyhow::Result<Vec<u64>> {
        let coarse = self.coarse.read().unwrap();
        let mut out = Vec::new();
        for &row in rows {
            let Some(photo) = self.photo(row)? else { continue };
            let approx = |g: &c1model::GeoPoint| coarse.get(&photo.id.key()).is_some_and(|c| c.holds(g.lat, g.lon));
            if photo.geo.as_ref().is_none_or(approx) {
                out.push(row);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(min: i64, lat: f64, lon: f64) -> Fix {
        Fix { at: min * 60_000, lat, lon }
    }

    #[test]
    fn spans_read_as_a_person_says_them() {
        assert_eq!(span(40_000), "40 s");
        assert_eq!(span(-12 * 60_000), "12 min");
        assert_eq!(span(3 * 3_600_000 + 5 * 60_000), "3 h 5 min");
        assert_eq!(span(2 * 3_600_000), "2 h");
        assert_eq!(span(3 * 86_400_000), "3 days");
    }

    #[test]
    fn the_photos_either_side_give_a_point_between() {
        let fixes = [fix(0, 50.0614, 19.9366), fix(20, 50.0500, 19.9500)];
        let lines = fix_clues(&fixes, 10 * 60_000, 180 * 60_000);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].starts_with("10 min before it") && lines[0].contains("50.0614, 19.9366"), "{lines:?}");
        assert!(lines[1].starts_with("10 min after it"), "{lines:?}");
        assert!(lines[2].contains("about 50.0557, 19.9433"), "{lines:?}");
        // Out of the window: nothing.
        assert!(fix_clues(&fixes, 10 * 60_000, 5 * 60_000).is_empty());
    }

    #[test]
    fn a_photo_taken_at_the_same_moment_is_told() {
        let fixes = [fix(0, 50.0, 19.0), fix(10, 50.06, 19.94), fix(20, 50.07, 19.95)];
        let lines = fix_clues(&fixes, 10 * 60_000, 180 * 60_000);
        assert!(lines[0].starts_with("A photo taken at the same moment") && lines[0].contains("50.0600"), "{lines:?}");
        assert!(lines[1].starts_with("10 min after it"), "{lines:?}");
        assert_eq!(lines.len(), 2, "no point between: the moment is known, {lines:?}");
    }

    fn shot(row: u64, min: i64, place: Option<&str>, trip: Option<usize>) -> SceneShot {
        SceneShot { row, at: Some(min * 60_000), place: place.map(str::to_owned), trip }
    }

    #[test]
    fn photos_one_after_another_at_one_place_make_a_scene() {
        let (gap, span) = (10 * 60_000, 60 * 60_000);
        let shots = vec![
            shot(3, 4, None, None),
            shot(1, 0, Some("Poland|—|Laski"), None),
            shot(2, 2, None, None),
            // 20 minutes later: another scene.
            shot(4, 24, None, None),
            // Another place written.
            shot(5, 26, Some("Poland|—|Bolesław"), None),
            shot(6, 27, Some("Poland|—|Laski"), None),
            // No time: alone.
            SceneShot { row: 7, at: None, place: None, trip: None },
            // Another trip.
            shot(8, 28, Some("Poland|—|Laski"), Some(1)),
        ];
        assert_eq!(scenes(shots, gap, span), vec![vec![1, 2, 3], vec![4, 5], vec![6], vec![8], vec![7]]);
        // A long walk, a shot every 5 minutes: cut every hour.
        let walk: Vec<SceneShot> = (0..30).map(|i| shot(i, i as i64 * 5, None, None)).collect();
        let cut = scenes(walk, gap, span);
        assert_eq!(cut.iter().map(Vec::len).collect::<Vec<_>>(), [13, 13, 4]);
    }

    #[test]
    fn a_big_scene_sends_photos_spread_evenly() {
        assert_eq!(sample(4, 9), [0, 1, 2, 3]);
        assert_eq!(sample(20, 5), [0, 4, 9, 14, 19]);
        assert_eq!(sample(10, 9).len(), 9);
        assert_eq!(*sample(10, 9).last().unwrap(), 9);
    }

    #[test]
    fn a_flight_between_is_told_not_drawn() {
        let fixes = [fix(0, 50.06, 19.94), fix(120, 38.72, -9.13)];
        let lines = fix_clues(&fixes, 60 * 60_000, 180 * 60_000);
        assert!(lines.last().unwrap().contains("travelled fast"), "{lines:?}");
    }

    #[test]
    fn places_around_are_told_once_and_not_the_photo_s_own() {
        let placed: Vec<(i64, Arc<str>, u64)> = vec![
            (0, Arc::from("Poland|—|Kraków|Kazimierz"), 1),
            (10 * 60_000, Arc::from("Poland|—|Kraków"), 2),
            (20 * 60_000, Arc::from("Poland|—|Kraków|Kazimierz"), 3),
        ];
        let lines = place_clues(&placed, (2, 10 * 60_000), Some("Poland|—|Kraków"), 60 * 60_000);
        assert_eq!(lines, ["10 min before it, a photo was placed in Kazimierz, Kraków, Poland"]);
    }

    #[test]
    fn a_point_out_of_reach_is_doubted() {
        let fixes = [fix(0, 50.06, 19.94)];
        // Warsaw, 250 km away, 10 minutes later.
        assert!(
            unreachable(&fixes, 10 * 60_000, 180 * 60_000, 52.23, 21.01, 1.0).unwrap().contains("km from the photo")
        );
        // Two hours later it could be.
        assert_eq!(unreachable(&fixes, 120 * 60_000, 180 * 60_000, 52.23, 21.01, 1.0), None);
        // Wieliczka, 13 km: near enough.
        assert_eq!(unreachable(&fixes, 10 * 60_000, 180 * 60_000, 49.98, 20.06, 1.0), None);
    }
}
