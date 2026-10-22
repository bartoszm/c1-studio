//! Putting photos on the map: positions from a place's name (offline from the gazetteer,
//! else as an online lookup found them), places from a position, positions from the
//! photos taken just before and after, and the map itself.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use c1geo::{Approx, Level, Resolved, gazetteer};
use c1model::{GeoOrigin, GeoPoint, Op, Place};
use serde::{Deserialize, Serialize};
use tantivy::collector::{Count, DocSetCollector};
use tantivy::columnar::StrColumn;
use tantivy::{DocAddress, Searcher};

use super::{Library, QueryOptions};
use crate::edits::now_ms;
use crate::filter::Filter;
use crate::geo::{self, Located, MapResult, MapView, PlacePoint};
use crate::gpx::Fix;
use crate::places::{Coarse, Guessed, Looked, same_spot};
use crate::schema::{self, Dim, NO_PLACE};

/// What places' names were found as, remembered: the gazetteer's answers do not change,
/// and a name it lacks is looked for with typos, which takes a while.
#[derive(Default)]
pub(super) struct PlaceCache {
    /// Where the place at a path is, the gazetteer's or an online lookup's answer.
    by_path: HashMap<String, Option<Approx>>,
    /// The gazetteer's answer alone, by path.
    offline: HashMap<String, Option<Approx>>,
    /// Online lookups, loaded once.
    online: Option<HashMap<String, Looked>>,
    /// What place paths' names were found as in the gazetteer (`path#code`).
    resolved: HashMap<String, Resolved>,
    /// Bumped when photos' places may have changed (a place edited, a sync).
    places: u64,
    /// The catalog's spellings, as of a count of `places`.
    spellings: Option<(u64, Arc<Spellings>)>,
}

/// For each country, state and city of the gazetteer the catalog names, the name it uses
/// most: a place found from a position is written the way the catalog already writes it.
#[derive(Default)]
struct Spellings {
    countries: HashMap<u32, String>,
    states: HashMap<u32, String>,
    cities: HashMap<u32, String>,
}

/// A place no position is known for at the level it names: to look up online.
#[derive(Clone, Debug, Serialize)]
pub struct Unresolved {
    /// As [`schema::place_tree`] writes it.
    pub path: String,
    pub place: Place,
    /// Photos the filter selects at it.
    pub photos: usize,
    /// What the gazetteer found, if anything.
    pub have: Option<Level>,
    pub want: Level,
}

/// What an online lookup found for a place.
#[derive(Clone, Debug, Deserialize)]
pub struct PlaceLookup {
    pub path: String,
    pub found: Option<Approx>,
}

/// Lookups kept at once, at most: the app sends a handful.
const LOOKUPS_AT_ONCE: usize = 200;

/// A photo as the placing passes read it from the index.
struct Seen {
    row: u64,
    at: Option<i64>,
    geo: Option<(f64, f64)>,
    rating: u64,
    sharpness: f64,
    /// Its deepest place path, `Poland|Malopolska|Kraków` (shared by the photos at it).
    path: Option<Arc<str>>,
}

/// Places found for photos with a position, each with the photos it is for.
#[derive(Clone, Debug, Serialize)]
pub struct FillGroup {
    pub place: Place,
    pub rows: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NeighbourSettings {
    /// Give photos with no position, or only their place's (≈), one from the photos with
    /// GPS taken around them.
    pub positions: bool,
    /// Those taken at most this far apart (1 to 2880: two days); a single one, within half.
    pub position_gap_minutes: i64,
    /// Give photos with no place, or a less specific one, a neighbour's that says the same
    /// and more.
    pub places: bool,
    /// A neighbour taken at most this far away (1 to 2880).
    pub place_gap_minutes: i64,
}

impl Default for NeighbourSettings {
    fn default() -> Self {
        Self { positions: true, position_gap_minutes: 30, places: true, place_gap_minutes: 15 }
    }
}

/// What placing photos from their neighbours would do.
#[derive(Clone, Debug, Default, Serialize)]
pub struct NeighbourPlan {
    /// Photos the filter selects with no position, or only their place's.
    pub unpositioned: usize,
    /// Of them, those given a position.
    pub positions: usize,
    /// Photos the filter selects with no location (the place's finest level).
    pub unlocated: usize,
    /// Of them, those given a more specific place.
    pub places: usize,
    /// Some of the positions, to show: row, lat, lon.
    pub sample: Vec<(u64, f64, f64)>,
    /// The places given: from (empty for none), to, how many photos; most photos first.
    pub place_counts: Vec<(String, String, usize)>,
}

/// One photo's placing from its neighbours.
enum Change {
    Position(f64, f64),
    /// The photo's place path `from` (none: no place) made `to`, from the neighbour's at
    /// `path` (its `row`, to read the whole place, code and all).
    Place {
        from: Option<Arc<str>>,
        to: String,
        path: Arc<str>,
        row: u64,
    },
}

/// What a neighbour's place says of a photo's.
#[derive(Clone)]
enum Says {
    /// The same and more: the photo's place made this path.
    More(String),
    /// Nothing the photo's place does not: the same, or less.
    Same,
    /// Somewhere else.
    Else,
}

/// Faster than this between two photos, they were taken from a plane: no line between them.
const MAX_KMH: f64 = 150.0;

/// The levels of a place path: `Poland|—|Kraków` → country, no state, city.
pub fn place_of_path(path: &str) -> Place {
    let mut levels = path.split('|').map(|l| Some(l.trim().to_owned()).filter(|l| !l.is_empty() && l != NO_PLACE));
    Place {
        country: levels.next().flatten(),
        state: levels.next().flatten(),
        city: levels.next().flatten(),
        location: levels.next().flatten(),
        country_code: None,
    }
}

/// The place at path `own` made more specific by the one at `other`, as a path: when they
/// differ at no level both name and `other` goes deeper. `Poland|—|Kraków` refined by
/// `Poland|Malopolska|Kraków|Wawel` is the latter; not by itself or `Poland|—|Wieliczka`.
fn refined(own: &str, other: &str) -> Option<String> {
    let (own, other) = (levels(own), levels(other));
    let deepest = |levels: &[Option<String>]| levels.iter().rposition(Option::is_some);
    if deepest(&other) <= deepest(&own) {
        return None;
    }
    let mut merged = Vec::new();
    for i in 0..own.len().max(other.len()) {
        let (mine, theirs) = (own.get(i).cloned().flatten(), other.get(i).cloned().flatten());
        match (mine, theirs) {
            (Some(m), Some(t)) if m != t => return None,
            (Some(l), _) | (None, Some(l)) => merged.push(l),
            (None, None) => merged.push(NO_PLACE.to_owned()),
        }
    }
    Some(merged.join("|"))
}

/// A place path's levels, those it leaves empty `None`.
fn levels(path: &str) -> Vec<Option<String>> {
    path.split('|').map(|l| Some(l.trim().to_owned()).filter(|l| !l.is_empty() && l != NO_PLACE)).collect()
}

/// What the place at path `other` says of a photo's at `own` (none: it has no place).
fn says(own: Option<&str>, other: &str) -> Says {
    let Some(own) = own else { return Says::More(other.to_owned()) };
    if own == other {
        return Says::Same;
    }
    if let Some(to) = refined(own, other) {
        return Says::More(to);
    }
    let (own, other) = (levels(own), levels(other));
    let differ = own.iter().zip(&other).any(|(a, b)| matches!((a, b), (Some(a), Some(b)) if a != b));
    if differ { Says::Else } else { Says::Same }
}

/// The place a photo at `own` (none: no place), taken at `at`, gets from the photos with a
/// place around it (`placed`, by time): on each side, the nearest within `gap` that says
/// more, past those saying the same; one saying somewhere else ends that side. Both sides
/// saying more, they must agree. The new path, with the neighbour's path and row.
fn place_from(
    placed: &[(i64, Arc<str>, u64)],
    (row, at): (u64, i64),
    own: Option<&Arc<str>>,
    gap: i64,
    memo: &mut HashMap<(Arc<str>, Arc<str>), Says>,
) -> Option<(String, Arc<str>, u64)> {
    let i = placed.partition_point(|p| p.0 < at);
    let mut side = |nearest_first: &mut dyn Iterator<Item = &(i64, Arc<str>, u64)>| {
        for p in nearest_first {
            if (p.0 - at).abs() > gap {
                break;
            }
            if p.2 == row {
                continue;
            }
            let said = match own {
                None => Says::More(p.1.to_string()),
                Some(own) if *own == p.1 => Says::Same,
                Some(own) => memo.entry((own.clone(), p.1.clone())).or_insert_with(|| says(Some(own), &p.1)).clone(),
            };
            match said {
                Says::More(to) => return Some((to, p.1.clone(), p.2)),
                Says::Same => continue,
                Says::Else => return None,
            }
        }
        None
    };
    let before = side(&mut placed[..i].iter().rev());
    let after = side(&mut placed[i..].iter());
    match (before, after) {
        (Some(b), Some(a)) if a.0 != b.0 => None,
        (Some(p), _) | (None, Some(p)) => Some(p),
        (None, None) => None,
    }
}

fn named(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| !s.trim().is_empty())
}

/// The deepest of a document's place tree values: its whole place.
fn deepest(column: &StrColumn, doc: u32, cache: &mut HashMap<u64, (usize, Arc<str>)>) -> Option<Arc<str>> {
    let mut best: Option<&(usize, Arc<str>)> = None;
    let ords: Vec<u64> = column.term_ords(doc).collect();
    for ord in &ords {
        if !cache.contains_key(ord) {
            let mut text = String::new();
            column.ord_to_str(*ord, &mut text).ok()?;
            cache.insert(*ord, (text.matches('|').count(), text.into()));
        }
    }
    for ord in &ords {
        let entry = &cache[ord];
        if best.is_none_or(|b| entry.0 > b.0) {
            best = Some(entry);
        }
    }
    best.map(|b| b.1.clone())
}

fn better(a: &Located, b: &Located) -> bool {
    (a.rating, a.sharpness, std::cmp::Reverse(a.row)) > (b.rating, b.sharpness, std::cmp::Reverse(b.row))
}

fn valid(a: &Approx) -> bool {
    a.lat.is_finite() && a.lon.is_finite() && a.lat.abs() <= 90.0 && a.lon.abs() <= 180.0
}

fn km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dlat = (b.0 - a.0).to_radians();
    let dlon = wrapped(b.1 - a.1).to_radians() * ((a.0 + b.0) / 2.0).to_radians().cos();
    6371.0 * dlat.hypot(dlon)
}

/// A longitude difference the short way round, -180 to 180.
fn wrapped(dlon: f64) -> f64 {
    (dlon + 540.0).rem_euclid(360.0) - 180.0
}

/// Where a photo taken at `at` was, between the fixes either side of it if they are at
/// most `gap` ms apart and were not taken from a plane; the short way round the world.
fn between(fixes: &[Fix], at: i64, gap: i64) -> Option<(f64, f64)> {
    let after = fixes.partition_point(|f| f.at < at);
    let next = fixes.get(after)?;
    if next.at == at {
        return Some((next.lat, next.lon));
    }
    let prev = fixes.get(after.checked_sub(1)?)?;
    let span = next.at - prev.at;
    let hours = span as f64 / 3_600_000.0;
    if span > gap || km((prev.lat, prev.lon), (next.lat, next.lon)) > 1.0 + MAX_KMH * hours {
        return None;
    }
    let t = (at - prev.at) as f64 / span as f64;
    let lon = prev.lon + wrapped(next.lon - prev.lon) * t;
    Some((prev.lat + (next.lat - prev.lat) * t, wrapped(lon)))
}

/// Where a photo taken at `at` was: [`between`] the fixes either side (at most `gap`
/// apart), or, failing that, at the nearest fix if within `near`.
fn position_at(fixes: &[Fix], at: i64, gap: i64, near: i64) -> Option<(f64, f64)> {
    between(fixes, at, gap).or_else(|| {
        let i = fixes.partition_point(|f| f.at < at);
        [i.checked_sub(1).and_then(|i| fixes.get(i)), fixes.get(i)]
            .into_iter()
            .flatten()
            .filter(|f| (f.at - at).abs() <= near)
            .min_by_key(|f| (f.at - at).abs())
            .map(|f| (f.lat, f.lon))
    })
}

impl Library {
    /// The photos `filter` selects, read from the index's columns; `paths` reads their
    /// places too.
    fn seen(&self, searcher: &Searcher, filter: &Filter, paths: bool) -> anyhow::Result<Vec<Seen>> {
        let query = self.compile(filter)?;
        let docs = searcher.search(query.as_ref(), &DocSetCollector)?;
        let mut by_segment: Vec<Vec<u32>> = vec![Vec::new(); searcher.segment_readers().len()];
        for DocAddress { segment_ord, doc_id } in docs {
            by_segment[segment_ord as usize].push(doc_id);
        }
        let mut out = Vec::new();
        for (segment, docs) in searcher.segment_readers().iter().zip(by_segment) {
            let fast = segment.fast_fields();
            let (row, captured, lat, lon) =
                (fast.u64("row")?, fast.i64("captured_sort")?, fast.f64("lat")?, fast.f64("lon")?);
            let (rating, sharpness) = (fast.u64("rating_sort")?, fast.f64("sharpness")?);
            let place = if paths { fast.str(Dim::PlaceTree.field_name())? } else { None };
            let mut names = HashMap::new();
            for doc in docs {
                let Some(r) = row.first(doc) else { continue };
                out.push(Seen {
                    row: r,
                    at: captured.first(doc).and_then(schema::captured_of_sort_key),
                    geo: lat.first(doc).zip(lon.first(doc)),
                    rating: rating.first(doc).unwrap_or(0),
                    sharpness: sharpness.first(doc).unwrap_or(0.0),
                    path: place.as_ref().and_then(|c| deepest(c, doc, &mut names)),
                });
            }
        }
        Ok(out)
    }

    /// The photo key of a row.
    pub(super) fn key_of(&self, row: u64) -> Option<String> {
        self.keys.read().unwrap().get(row as usize)?.clone()
    }

    /// Photos' places may have changed: the catalog's spellings are worked out again.
    pub(super) fn places_changed(&self) {
        self.place_cache.lock().unwrap().places += 1;
    }

    /// The whole place of a photo at `path` (its country code too), or the path's levels.
    fn place_at(&self, path: &str, row: u64) -> Place {
        match self.store.photo(row) {
            Ok(Some(p)) if schema::place_tree(&p.place).last().map(String::as_str) == Some(path) => p.place,
            _ => place_of_path(path),
        }
    }

    fn online(cache: &mut PlaceCache, book: &crate::places::PlaceBook) -> anyhow::Result<()> {
        if cache.online.is_none() {
            cache.online = Some(book.online()?);
        }
        Ok(())
    }

    /// The gazetteer's answer for the place at `path`, worked out once. The cache is not
    /// held meanwhile: a name it lacks takes a while.
    fn offline_at(&self, path: &str, row: u64) -> Option<Approx> {
        if let Some(found) = self.place_cache.lock().unwrap().offline.get(path) {
            return *found;
        }
        let found = gazetteer().locate(&self.place_at(path, row));
        self.place_cache.lock().unwrap().offline.insert(path.to_owned(), found);
        found
    }

    /// Where the place at `path` is: the gazetteer's answer, or an online lookup's when
    /// that is finer.
    fn approx_at(&self, path: &str, row: u64) -> anyhow::Result<Option<Approx>> {
        if let Some(found) = self.place_cache.lock().unwrap().by_path.get(path) {
            return Ok(*found);
        }
        let offline = self.offline_at(path, row);
        let mut cache = self.place_cache.lock().unwrap();
        Self::online(&mut cache, &self.place_book)?;
        let online = cache.online.as_ref().and_then(|o| o.get(path)).and_then(|l| l.found);
        let found = match (offline, online) {
            (Some(a), Some(b)) if b.level > a.level => Some(b),
            (None, b) => b,
            (a, _) => a,
        };
        cache.by_path.insert(path.to_owned(), found);
        Ok(found)
    }

    /// The photos `filter` selects that have a position, clustered for `view`, and those
    /// placed only by name, where their place is. A filter's own `within` is set aside, so
    /// the map keeps showing the places around the area it picked.
    pub fn map(&self, filter: &Filter, view: &MapView) -> anyhow::Result<MapResult> {
        let base = Filter { within: None, ..filter.clone() };
        let searcher = self.reader.searcher();
        let mut points = Vec::new();
        // Photos at a place's position, by place path: one point per position.
        let mut by_place: HashMap<Arc<str>, Vec<PlacePoint>> = HashMap::new();
        let mut add = |path: &Arc<str>, a: Approx, s: &Seen| {
            let best = Located { row: s.row, lat: a.lat, lon: a.lon, rating: s.rating, sharpness: s.sharpness };
            let at = by_place.entry(path.clone()).or_default();
            match at.iter_mut().find(|p| p.lat == a.lat && p.lon == a.lon) {
                Some(p) => {
                    p.count += 1;
                    if better(&best, &p.best) {
                        p.best = best;
                    }
                }
                None => at.push(PlacePoint {
                    path: path.to_string(),
                    lat: a.lat,
                    lon: a.lon,
                    level: a.level,
                    count: 1,
                    best,
                }),
            }
        };
        let mut result = MapResult::default();
        // The filter may want only positions of their own, or only those that are not.
        let wants = |approx: bool| base.approx.is_none_or(|a| a == approx);
        if wants(false) {
            for s in self.seen(&searcher, &Filter { gps: Some(true), approx: Some(false), ..base.clone() }, false)? {
                let Some((lat, lon)) = s.geo else { continue };
                points.push(Located { row: s.row, lat, lon, rating: s.rating, sharpness: s.sharpness });
            }
        }
        if wants(true) {
            // Given their place's position: shown under the place they have now (it may
            // have been edited since); their mark says only how precise the position is.
            let promoted =
                self.seen(&searcher, &Filter { gps: Some(true), approx: Some(true), ..base.clone() }, true)?;
            let (coarse, keys) = (self.coarse.read().unwrap(), self.keys.read().unwrap());
            for s in promoted {
                let Some((lat, lon)) = s.geo else { continue };
                let mark = keys.get(s.row as usize).and_then(Option::as_deref).and_then(|k| coarse.get(k));
                match (&s.path, mark.filter(|c| c.holds(lat, lon))) {
                    (Some(path), Some(c)) => {
                        result.place_only += 1;
                        add(path, Approx { lat, lon, level: c.level }, &s);
                    }
                    // Its place cleared since (or its mark gone): a position like any other.
                    _ => points.push(Located { row: s.row, lat, lon, rating: s.rating, sharpness: s.sharpness }),
                }
            }
        }
        let exact = geo::cluster(&points, view);
        (result.located, result.in_view, result.clusters) = (exact.located, exact.in_view, exact.clusters);
        for s in self.seen(&searcher, &Filter { gps: Some(false), placed: Some(true), ..base.clone() }, true)? {
            let Some(path) = &s.path else { continue };
            result.place_only += 1;
            match self.approx_at(path, s.row)? {
                Some(a) => add(path, a, &s),
                None => result.unresolved += 1,
            }
        }
        let places: Vec<PlacePoint> = by_place.into_values().flatten().collect();
        result.approx_located = places.iter().map(|p| p.count).sum();
        (result.approx, result.approx_in_view, result.approx_hidden) = geo::cluster_places(&places, view);
        let unplaced = self.compile(&Filter { gps: Some(false), placed: Some(false), ..base })?;
        result.unplaced = searcher.search(unplaced.as_ref(), &Count)?;
        Ok(result)
    }

    /// The places the photos `filter` selects name more precisely than the gazetteer
    /// knows them, and that were not looked up online yet; the most photographed first.
    pub fn unresolved_places(&self, filter: &Filter) -> anyhow::Result<Vec<Unresolved>> {
        let searcher = self.reader.searcher();
        let base = Filter { within: None, gps: Some(false), placed: Some(true), ..filter.clone() };
        let mut counts: HashMap<Arc<str>, (usize, u64)> = HashMap::new();
        for s in self.seen(&searcher, &base, true)? {
            if let Some(path) = s.path {
                counts.entry(path).or_insert((0, s.row)).0 += 1;
            }
        }
        let asked: HashSet<String> = {
            let mut cache = self.place_cache.lock().unwrap();
            Self::online(&mut cache, &self.place_book)?;
            let online = cache.online.as_ref().expect("loaded");
            counts.keys().filter(|p| online.contains_key(&***p)).map(|p| p.to_string()).collect()
        };
        let mut out = Vec::new();
        for (path, (photos, row)) in counts {
            if asked.contains(&*path) {
                continue;
            }
            let place = self.place_at(&path, row);
            let Some(mut want) = Level::named(&place) else { continue };
            let have = self.offline_at(&path, row).map(|a| a.level);
            // A location that only names the city again (`Enna|Enna`), or a city typed as
            // the location (`Germany|—|—|Berlin`), is no finer than the city.
            let named = |v: &Option<String>| v.as_deref().map(c1geo::fold).filter(|s| !s.is_empty());
            if want == Level::Location
                && have == Some(Level::City)
                && (named(&place.city).is_none() || named(&place.city) == named(&place.location))
            {
                want = Level::City;
            }
            if have.is_none_or(|h| h < want) {
                out.push(Unresolved { path: path.to_string(), place, photos, have, want });
            }
        }
        out.sort_by(|a, b| b.photos.cmp(&a.photos).then_with(|| a.path.cmp(&b.path)));
        Ok(out)
    }

    /// Keeps what online lookups found (or that they found nothing: not asked again). A
    /// position out of range is not kept, so the place is asked again another time.
    pub fn set_place_points(&self, items: &[PlaceLookup]) -> anyhow::Result<()> {
        let now = now_ms();
        let looked: Vec<(String, Looked)> = items
            .iter()
            .take(LOOKUPS_AT_ONCE)
            .filter(|i| !i.path.trim().is_empty() && i.path.len() <= 1024 && i.found.as_ref().is_none_or(valid))
            .map(|i| (i.path.clone(), Looked { found: i.found, at: now }))
            .collect();
        self.place_book.put_online(&looked)?;
        let mut cache = self.place_cache.lock().unwrap();
        Self::online(&mut cache, &self.place_book)?;
        for (path, l) in looked {
            cache.by_path.remove(&path);
            cache.online.as_mut().expect("loaded").insert(path, l);
        }
        Ok(())
    }

    /// The photos `filter` selects whose whole place is `path` and that have no position,
    /// not even their place's.
    pub fn place_rows(&self, filter: &Filter, path: &str) -> anyhow::Result<Vec<u64>> {
        let searcher = self.reader.searcher();
        let base = Filter { within: None, gps: Some(false), place_under: Some(path.to_owned()), ..filter.clone() };
        Ok(self
            .seen(&searcher, &base, true)?
            .into_iter()
            .filter(|s| s.path.as_deref() == Some(path))
            .map(|s| s.row)
            .collect())
    }

    /// Gives the photos `filter` selects at each of `paths` (and with no position) their
    /// place's position, marked as only that: they keep showing as approximate. Returns
    /// the ops that undo it.
    pub fn promote(&self, filter: &Filter, paths: &[String]) -> anyhow::Result<Vec<(u64, Op)>> {
        let mut rows = Vec::new();
        for path in paths {
            rows.extend(self.place_rows(filter, path)?);
        }
        self.promote_rows(&rows)
    }

    /// Gives each photo among `rows` that has no position its place's, marked as only that.
    /// Returns the ops that undo it.
    pub fn promote_rows(&self, rows: &[u64]) -> anyhow::Result<Vec<(u64, Op)>> {
        let mut changes = Vec::new();
        let mut marks = Vec::new();
        for &row in rows {
            let Some(photo) = self.photo(row)? else { continue };
            let Some(path) = schema::place_tree(&photo.place).pop() else { continue };
            if photo.geo.is_some() {
                continue;
            }
            let Some(a) = self.approx_at(&path, row)? else { continue };
            let value = GeoPoint { lat: a.lat, lon: a.lon, alt: None, origin: GeoOrigin::Geocoded };
            changes.push((row, Op::SetGeo { base: None, value: Some(value) }));
            marks.push((photo.id.key(), Some(Coarse { lat: a.lat, lon: a.lon, level: a.level, path })));
        }
        // Marks first: a mark on a photo not (yet) at its point is ignored, a position
        // without its mark would pass for one taken there. The index reads them too.
        self.place_book.set_coarse(&marks)?;
        self.coarse.write().unwrap().extend(marks.into_iter().filter_map(|(k, c)| Some((k, c?))));
        self.edit(&changes)
    }

    /// What a place's names are in the gazetteer, remembered by path and code.
    fn resolve_place(&self, place: &Place) -> Resolved {
        let key = format!(
            "{}#{}",
            schema::place_tree(place).pop().unwrap_or_default(),
            place.country_code.as_deref().unwrap_or("")
        );
        if let Some(r) = self.place_cache.lock().unwrap().resolved.get(&key) {
            return *r;
        }
        let r = gazetteer().resolve(place);
        self.place_cache.lock().unwrap().resolved.insert(key, r);
        r
    }

    /// For each gazetteer place the catalog names, the name it uses most; worked out again
    /// when places changed.
    fn spellings(&self) -> anyhow::Result<Arc<Spellings>> {
        let generation = {
            let cache = self.place_cache.lock().unwrap();
            if let Some((g, s)) = &cache.spellings
                && *g == cache.places
            {
                return Ok(s.clone());
            }
            cache.places
        };
        let options = QueryOptions { limit: Some(0), facets: vec![Dim::PlaceTree], ..QueryOptions::default() };
        let counts = self.query(&Filter::default(), &options)?.facets.remove(&Dim::PlaceTree).unwrap_or_default();
        let mut votes: [HashMap<u32, HashMap<String, u64>>; 3] = Default::default();
        for (path, n) in counts {
            let levels: Vec<&str> = path.split('|').collect();
            let Some(name) = levels.last().filter(|l| **l != NO_PLACE) else { continue };
            let place = place_of_path(&path);
            let r = self.resolve_place(&place);
            let id = match levels.len() {
                1 => r.country.map(|id| (0, id)),
                2 => r.state.map(|id| (1, id)),
                3 if place.city.is_some() => r.city.map(|id| (2, id)),
                _ => None,
            };
            if let Some((kind, id)) = id {
                *votes[kind].entry(id).or_default().entry((*name).to_owned()).or_default() += n;
            }
        }
        let pick = |v: HashMap<u32, HashMap<String, u64>>| -> HashMap<u32, String> {
            v.into_iter()
                .filter_map(|(id, names)| {
                    Some((id, names.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))?.0))
                })
                .collect()
        };
        let [countries, states, cities] = votes;
        let s = Arc::new(Spellings { countries: pick(countries), states: pick(states), cities: pick(cities) });
        self.place_cache.lock().unwrap().spellings = Some((generation, s.clone()));
        Ok(s)
    }

    /// The gazetteer's names in the catalog's own spelling.
    fn spell(s: &Spellings, found: c1geo::Found) -> Place {
        let ids = found.ids;
        let name = |map: &HashMap<u32, String>, id: Option<u32>, own: Option<String>| {
            id.and_then(|id| map.get(&id).cloned()).or(own)
        };
        Place {
            country: name(&s.countries, ids.country, Some(found.country)),
            country_code: Some(found.country_code),
            state: name(&s.states, ids.state, found.state),
            city: name(&s.cities, ids.city, found.city),
            location: None,
        }
    }

    /// For the photos among `rows` that have a position, the place it is in, for the
    /// place fields they leave empty. A photo whose place says otherwise (another
    /// country, say) is left alone.
    pub fn fill_plan(&self, rows: &[u64]) -> anyhow::Result<Vec<FillGroup>> {
        let mut groups: Vec<FillGroup> = Vec::new();
        let mut spellings = None;
        for &row in rows {
            let Some(photo) = self.photo(row)? else { continue };
            let Some(filled) = self.filled(&photo, &mut spellings)? else { continue };
            match groups.iter_mut().find(|g| g.place == filled) {
                Some(g) => g.rows.push(row),
                None => groups.push(FillGroup { place: filled, rows: vec![row] }),
            }
        }
        groups.sort_by_key(|g| std::cmp::Reverse(g.rows.len()));
        Ok(groups)
    }

    /// Whether the place a position was found in agrees with a photo's: no level the
    /// photo names is another there. A country or state typed but not found in the
    /// gazetteer is not its to judge: they do not agree.
    fn agrees(&self, place: &Place, found: &c1geo::Found) -> bool {
        let own = self.resolve_place(place);
        let same = |mine: Option<u32>, theirs: Option<u32>| mine.is_none() || mine == theirs;
        same(own.country, found.ids.country)
            && same(own.state, found.ids.state)
            && same(own.city, found.ids.city)
            && !(named(&place.country) && own.country.is_none())
            && !(named(&place.state) && own.state.is_none())
    }

    /// A photo's place with what its position says filled in where it is empty; `None`
    /// when there is nothing to fill or the two disagree. The catalog's spellings are
    /// worked out only once something is to be filled.
    fn filled(&self, photo: &c1model::Photo, spellings: &mut Option<Arc<Spellings>>) -> anyhow::Result<Option<Place>> {
        let p = &photo.place;
        if named(&p.country) && named(&p.country_code) && named(&p.state) && named(&p.city) {
            return Ok(None);
        }
        let Some(geo) = photo.geo else { return Ok(None) };
        let Some(found) = gazetteer().reverse(geo.lat, geo.lon) else { return Ok(None) };
        if !self.agrees(p, &found) {
            return Ok(None);
        }
        let s = match spellings {
            Some(s) => s.clone(),
            None => spellings.insert(self.spellings()?).clone(),
        };
        let spelled = Self::spell(&s, found);
        let mut place = p.clone();
        for (slot, value) in [
            (&mut place.country, spelled.country),
            (&mut place.country_code, spelled.country_code),
            (&mut place.state, spelled.state),
            (&mut place.city, spelled.city),
        ] {
            if !named(slot) {
                *slot = value;
            }
        }
        Ok((place != *p).then_some(place))
    }

    /// Fills the empty place fields of the photos among `rows` from their positions.
    /// Returns the ops that undo it.
    pub fn fill_places(&self, rows: &[u64]) -> anyhow::Result<Vec<(u64, Op)>> {
        let mut changes = Vec::new();
        let mut spellings = None;
        for &row in rows {
            let Some(photo) = self.photo(row)? else { continue };
            if let Some(value) = self.filled(&photo, &mut spellings)? {
                changes.push((row, Op::SetPlace { base: photo.place, value }));
            }
        }
        self.edit(&changes)
    }

    /// The catalog's positions of their own, by time: none given from a place's name or
    /// from neighbours.
    fn fixes(&self, searcher: &Searcher) -> anyhow::Result<Vec<Fix>> {
        let mut fixes = Vec::new();
        let own = self.seen(searcher, &Filter { approx: Some(false), ..Filter::default() }, false)?;
        let (guessed, keys) = (self.guessed.read().unwrap(), self.keys.read().unwrap());
        for s in own {
            let (Some(at), Some((lat, lon))) = (s.at, s.geo) else { continue };
            let guess = || keys.get(s.row as usize).and_then(Option::as_deref).and_then(|k| guessed.get(k));
            if guessed.is_empty() || !guess().is_some_and(|g| g.holds(lat, lon)) {
                fixes.push(Fix { at, lat, lon });
            }
        }
        fixes.sort_by_key(|f| f.at);
        Ok(fixes)
    }

    /// The catalog's photos with a place, by time: when, the place's path, the row.
    fn placed(&self, searcher: &Searcher) -> anyhow::Result<Vec<(i64, Arc<str>, u64)>> {
        let mut placed = Vec::new();
        for s in self.seen(searcher, &Filter { placed: Some(true), ..Filter::default() }, true)? {
            if let (Some(at), Some(path)) = (s.at, s.path) {
                placed.push((at, path, s.row));
            }
        }
        placed.sort();
        Ok(placed)
    }

    /// Whether a position lies in a photo's place: anywhere for a photo with none; nowhere
    /// for one in a city the gazetteer lacks, which could be anywhere.
    fn inside(&self, place: &Place, lat: f64, lon: f64) -> bool {
        if !(named(&place.country) || named(&place.state) || named(&place.city)) {
            return true;
        }
        if named(&place.city) && self.resolve_place(place).city.is_none() {
            return false;
        }
        gazetteer().reverse(lat, lon).is_some_and(|found| self.agrees(place, &found))
    }

    /// What the photos taken just before and after say of each photo `filter` selects
    /// (the map's area aside). One with no position, or only its place's: a position between
    /// theirs, if it lies in its place. One with no location: a neighbour's place that says
    /// the same and more. Positions given from a place or from neighbours are not used.
    fn neighbours(&self, filter: &Filter, settings: &NeighbourSettings) -> anyhow::Result<Vec<(u64, Change)>> {
        let searcher = self.reader.searcher();
        let mut out = Vec::new();
        if settings.positions {
            let gap = settings.position_gap_minutes.clamp(1, 2880) * 60_000;
            let fixes = self.fixes(&searcher)?;
            for wanted in unpositioned(filter) {
                for s in self.seen(&searcher, &wanted, false)? {
                    let Some(at) = s.at else { continue };
                    let Some((lat, lon)) = position_at(&fixes, at, gap, gap / 2) else { continue };
                    if s.geo.is_some_and(|g| same_spot(g, (lat, lon))) {
                        continue;
                    }
                    let Some(photo) = self.photo(s.row)? else { continue };
                    if self.inside(&photo.place, lat, lon) {
                        out.push((s.row, Change::Position(lat, lon)));
                    }
                }
            }
        }
        if settings.places
            && let Some(wanted) = unlocated(filter)
        {
            let gap = settings.place_gap_minutes.clamp(1, 2880) * 60_000;
            let placed = self.placed(&searcher)?;
            let mut memo = HashMap::new();
            for s in self.seen(&searcher, &wanted, true)? {
                let Some(at) = s.at else { continue };
                if let Some((to, path, row)) = place_from(&placed, (s.row, at), s.path.as_ref(), gap, &mut memo) {
                    out.push((s.row, Change::Place { from: s.path, to, path, row }));
                }
            }
        }
        Ok(out)
    }

    pub fn neighbour_plan(&self, filter: &Filter, settings: &NeighbourSettings) -> anyhow::Result<NeighbourPlan> {
        let searcher = self.reader.searcher();
        let count = |f: &Filter| -> anyhow::Result<usize> { Ok(searcher.search(self.compile(f)?.as_ref(), &Count)?) };
        let mut plan = NeighbourPlan::default();
        for f in unpositioned(filter) {
            plan.unpositioned += count(&f)?;
        }
        if let Some(f) = unlocated(filter) {
            plan.unlocated = count(&f)?;
        }
        let mut counts: HashMap<(Option<Arc<str>>, String), usize> = HashMap::new();
        for (row, c) in self.neighbours(filter, settings)? {
            match c {
                Change::Position(lat, lon) => {
                    plan.positions += 1;
                    if plan.sample.len() < 500 {
                        plan.sample.push((row, lat, lon));
                    }
                }
                Change::Place { from, to, .. } => {
                    plan.places += 1;
                    *counts.entry((from, to)).or_default() += 1;
                }
            }
        }
        plan.place_counts =
            counts.into_iter().map(|((from, to), n)| (from.as_deref().unwrap_or_default().to_owned(), to, n)).collect();
        plan.place_counts.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| (&a.0, &a.1).cmp(&(&b.0, &b.1))));
        Ok(plan)
    }

    /// Places the photos as [`Library::neighbour_plan`] showed: a place keeps what it says
    /// and gains the rest; a position is marked as a guess. Returns the ops that undo it.
    pub fn neighbour_apply(&self, filter: &Filter, settings: &NeighbourSettings) -> anyhow::Result<Vec<(u64, Op)>> {
        let mut places: HashMap<Arc<str>, Place> = HashMap::new();
        let mut changes = Vec::new();
        let mut guesses = Vec::new();
        let mut seen = HashSet::new();
        for (row, c) in self.neighbours(filter, settings)? {
            let Some(photo) = self.photo(row)? else { continue };
            match c {
                Change::Position(lat, lon) => {
                    if !seen.insert((row, "geo")) {
                        continue;
                    }
                    guesses.push((photo.id.key(), Some(Guessed { lat, lon })));
                    let value = GeoPoint { lat, lon, alt: None, origin: GeoOrigin::Manual };
                    changes.push((row, Op::SetGeo { base: photo.geo, value: Some(value) }));
                }
                Change::Place { path, row: at, .. } => {
                    if !seen.insert((row, "place")) {
                        continue;
                    }
                    let theirs = places.entry(path.clone()).or_insert_with(|| self.place_at(&path, at));
                    let mut value = photo.place.clone();
                    for (slot, given) in [
                        (&mut value.country, &theirs.country),
                        (&mut value.country_code, &theirs.country_code),
                        (&mut value.state, &theirs.state),
                        (&mut value.city, &theirs.city),
                        (&mut value.location, &theirs.location),
                    ] {
                        if !named(slot) {
                            *slot = given.clone();
                        }
                    }
                    if value != photo.place {
                        changes.push((row, Op::SetPlace { base: photo.place, value }));
                    }
                }
            }
        }
        // Marks first: a mark on a photo not (yet) at its point is ignored, a position
        // without its mark would pass for one taken there.
        self.place_book.set_guessed(&guesses)?;
        self.guessed.write().unwrap().extend(guesses.into_iter().filter_map(|(k, g)| Some((k, g?))));
        self.edit(&changes)
    }
}

/// The photos `filter` selects with no position, and those with only their place's, the
/// map's area aside.
fn unpositioned(filter: &Filter) -> [Filter; 2] {
    let base = Filter { within: None, ..filter.clone() };
    [Filter { gps: Some(false), ..base.clone() }, Filter { gps: Some(true), approx: Some(true), ..base }]
}

/// The photos `filter` selects with no location, the map's area aside; none if it wants
/// only those with one.
fn unlocated(filter: &Filter) -> Option<Filter> {
    (filter.located != Some(true)).then(|| Filter { within: None, located: Some(false), ..filter.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_goes_the_short_way_round_and_not_by_plane() {
        let fix = |min: i64, lat: f64, lon: f64| Fix { at: min * 60_000, lat, lon };
        let (lat, lon) = between(&[fix(0, -17.0, 179.9), fix(10, -17.0, -179.9)], 5 * 60_000, 30 * 60_000).unwrap();
        assert!((lat + 17.0).abs() < 1e-9 && lon.abs() > 179.9, "across the date line, not round the world: {lon}");
        // Kraków to Lisbon in 20 minutes: a flight.
        assert_eq!(between(&[fix(0, 50.06, 19.94), fix(20, 38.72, -9.13)], 10 * 60_000, 30 * 60_000), None);
        assert!(between(&[fix(0, 50.06, 19.94), fix(20, 50.08, 19.95)], 10 * 60_000, 30 * 60_000).is_some());
    }

    #[test]
    fn a_place_is_refined_by_one_saying_the_same_and_more() {
        let r = |own, other| refined(own, other);
        assert_eq!(
            r("Poland|—|Kraków", "Poland|Malopolska|Kraków|Wawel").as_deref(),
            Some("Poland|Malopolska|Kraków|Wawel")
        );
        assert_eq!(r("Poland", "Poland|Malopolska").as_deref(), Some("Poland|Malopolska"));
        // What the photo says is kept where the neighbour says nothing.
        assert_eq!(
            r("Poland|Malopolska|Kraków", "Poland|—|Kraków|Wawel").as_deref(),
            Some("Poland|Malopolska|Kraków|Wawel")
        );
        assert_eq!(r("Poland|Malopolska|Kraków", "Poland|Malopolska|Kraków"), None, "the same place");
        assert_eq!(r("Poland|—|Kraków", "Poland|Malopolska|Wieliczka|Mine"), None, "another city");
        assert_eq!(r("Poland|Malopolska|Kraków", "Poland|Malopolska"), None, "less");
        assert_eq!(r("Poland|—|Kraków", "Poland|Malopolska|Kraków"), None, "only a gap above filled: not deeper");
    }
}
