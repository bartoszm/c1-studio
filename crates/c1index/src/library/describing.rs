//! Suggestions from a vision model (stage 5): stored per photo until the user accepts
//! them into the edit queue, as edits from [`Origin::Ai`], or rejects them.

use std::collections::{HashMap, HashSet};

use c1ai::{PlaceGuess, RunRecord};
use c1geo::{Level, gazetteer};
use c1model::{GeoOrigin, GeoPoint, KeywordPath, Op, Place};
use serde::{Deserialize, Serialize};

use super::Library;
use crate::ai::{Entry, Review, Status, Taken, Task};
use crate::edits::Origin;
use crate::places::{Coarse, Guessed, same_spot};
use crate::schema;

/// The key a refine call is recorded under: it is about no photo.
const REFINE_KEY: &str = "refine";

/// How likely a place level the gazetteer does not know (or knows elsewhere) is left at
/// most: a hint for the review, never a tick by default.
pub(super) const DOUBTED: f32 = 0.5;

/// A guessed point this close to the city named (beyond its radius) is taken to be in it,
/// whatever city the gazetteer finds there.
const NEAR_KM: f64 = 5.0;

/// What the user takes from one photo's suggestion, as edited in the review: any field
/// left out is not touched.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Accept {
    pub row: u64,
    /// Pipe paths; ones the catalog lacks are added to it.
    pub keywords: Vec<String>,
    /// Becomes the caption (IPTC description).
    pub caption: Option<String>,
    pub headline: Option<String>,
    pub rating: Option<u8>,
    /// The place levels taken (country, its code, state, city, location); those left
    /// `None` are not touched.
    pub place: Place,
    /// A position taken, `[lat, lon]`: given only to a photo with none (or, from a
    /// locating run, only an approximate one), and marked as approximate (≈), like a
    /// place's position, unless `precise`.
    pub point: Option<[f64; 2]>,
    /// The position is taken as one of the photo's own, not marked approximate: a
    /// landmark recognised, or the point set by hand.
    pub precise: bool,
    /// What the review showed the photo to have.
    pub seen: Seen,
}

/// A photo's caption, headline and rating as the review showed them. A field taken is
/// set only while the photo still has what was shown: one written since (in Capture One,
/// in the panel) is not replaced unseen. Left out, it reads as "the photo had none".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Seen {
    pub caption: Option<String>,
    pub headline: Option<String>,
    pub rating: Option<u8>,
    pub place: Place,
}

/// A review kept, with its photo's row while the photo is in the library.
#[derive(Clone, Debug, Serialize)]
pub struct ReviewRow {
    pub seq: u64,
    pub row: Option<u64>,
    #[serde(flatten)]
    pub review: Review,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn review_of(task: Task, key: String, entry: Entry, status: Status, taken: Option<Taken>) -> Review {
    Review {
        at: now_ms(),
        key,
        model: entry.model,
        prompt: entry.prompt,
        task,
        status,
        suggestion: entry.suggestion,
        taken,
    }
}

impl From<&Accept> for Taken {
    fn from(pick: &Accept) -> Taken {
        let text = |v: &Option<String>| text(v).map(str::to_string);
        Taken {
            keywords: pick.keywords.clone(),
            caption: text(&pick.caption),
            headline: text(&pick.headline),
            rating: pick.rating,
            place: pick.place.clone(),
            point: pick.point.is_some(),
            precise: pick.point.is_some() && pick.precise,
        }
    }
}

/// What accepting did.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Accepted {
    /// The ops that undo it.
    pub undo: Vec<(u64, Op)>,
    /// Photos with a taken field that changed since it was shown: that field was left as
    /// it is, and the suggestion stays to review against what the photo has now.
    pub changed: Vec<u64>,
}

/// A text field as compared: trimmed, empty as none.
fn text(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// The photo's place with the levels `pick` takes, each only where the photo still has
/// what the review showed; and whether a level was left for that.
fn taken_place(now: &Place, pick: &Accept) -> (Place, bool) {
    let mut place = now.clone();
    let mut stale = false;
    let slots =
        |p: &Place| [p.country.clone(), p.country_code.clone(), p.state.clone(), p.city.clone(), p.location.clone()];
    let (taken, seen, had) = (slots(&pick.place), slots(&pick.seen.place), slots(now));
    let mut out = slots(now);
    for i in 0..5 {
        let Some(value) = text(&taken[i]) else { continue };
        if text(&had[i]) == text(&seen[i]) {
            out[i] = Some(value.to_string());
        } else if text(&had[i]) != Some(value) {
            stale = true;
        }
    }
    // Another country taken without a code: the code the photo had was the old country's.
    if text(&out[0]) != text(&had[0]) && text(&taken[1]).is_none() {
        out[1] = None;
    }
    let [country, country_code, state, city, location] = out;
    place.country = country;
    place.country_code = country_code;
    place.state = state;
    place.city = city;
    place.location = location;
    (place, stale)
}

/// Checks a model's place against the gazetteer and fills the country's code. A country
/// or city it does not know, or knows only elsewhere than the level above, is left at
/// most [`DOUBTED`] likely, as is a point outside the city or country named. A state is
/// only told: regions go by many names. Returns why.
pub fn check_place(guess: &mut PlaceGuess) -> Vec<String> {
    let name = |g: &Option<c1ai::Guess>| g.as_ref().map(|g| g.name.clone());
    let place =
        Place { country: name(&guess.country), state: name(&guess.state), city: name(&guess.city), ..Place::default() };
    let g = gazetteer();
    let found = g.resolve(&place);
    // A city found brings its own country and state, so each level is looked up by its
    // own name too: a country the gazetteer does not know is not taken as known through
    // the city.
    let alone = |p: Place| g.resolve(&p);
    let country_known = guess.country.is_none()
        || alone(Place { country: place.country.clone(), ..Place::default() }).country.is_some();
    let state_known = guess.state.is_none()
        || alone(Place { country: place.country.clone(), state: place.state.clone(), ..Place::default() })
            .state
            .is_some();
    guess.country_code = found
        .country
        .filter(|_| country_known && guess.country.is_some())
        .and_then(|id| g.country_code(id))
        .map(str::to_owned);
    let mut why = Vec::new();
    for (level, known, cap) in [
        (&mut guess.country, country_known, true),
        (&mut guess.state, state_known, false),
        (&mut guess.city, found.city.is_some(), true),
    ] {
        let Some(level) = level.as_mut().filter(|_| !known) else { continue };
        if cap && level.p > DOUBTED {
            why.push(format!(
                "{} is not known there: {:.0}% lowered to {:.0}%",
                level.name,
                level.p * 100.0,
                DOUBTED * 100.0
            ));
            level.p = DOUBTED;
        } else {
            why.push(format!("{} is not known there", level.name));
        }
    }
    guess.coherent();
    // Where the point is: in the named country's terms when one of its cities covers it,
    // as a bigger city across a border may reach it too.
    let at_point = |lat: f64, lon: f64| {
        found.country.filter(|_| country_known).and_then(|k| g.reverse_in(lat, lon, k)).or_else(|| g.reverse(lat, lon))
    };
    if let Some(point) = guess.point.as_mut()
        && let Some(at) = at_point(point.lat, point.lon)
    {
        let differ = |named: Option<u32>, there: Option<u32>| named.is_some() && there.is_some() && named != there;
        // A village is often found as the town whose reach covers it: a point near the city
        // named is not doubted for it. Nor is one in the region named when the city found
        // by that name lies in another region: a namesake (the gazetteer lacks the village
        // named, Laski in Małopolska, and found Laski in Łódzkie).
        let near_named = || {
            g.locate(&place).is_some_and(|c| {
                c.level == Level::City
                    && super::placing::km((c.lat, c.lon), (point.lat, point.lon)) <= point.radius_km as f64 + NEAR_KM
            })
        };
        let state_named =
            alone(Place { country: place.country.clone(), state: place.state.clone(), ..Place::default() }).state;
        let namesake = state_named.is_some()
            && state_named == at.ids.state
            && found.city.is_some_and(|c| g.city_state(c) != state_named);
        let city_differs = differ(found.city, at.ids.city) && !namesake && !near_named();
        if (differ(found.country, at.ids.country) || city_differs) && point.p > DOUBTED {
            why.push(format!(
                "the position is in {}, not the place named: {:.0}% lowered to {:.0}%",
                at.city.as_deref().unwrap_or(&at.country),
                point.p * 100.0,
                DOUBTED * 100.0
            ));
            point.p = DOUBTED;
        }
    }
    why
}

impl Library {
    /// The catalog's keyword paths with how many photos carry each, most used first: what
    /// a model is asked to choose from.
    pub fn vocabulary(&self) -> anyhow::Result<Vec<(String, u64)>> {
        let mut out: Vec<(String, u64)> =
            self.keywords()?.into_iter().filter(|n| n.uses > 0).map(|n| (n.path.to_pipe(), n.uses as u64)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(out)
    }

    pub fn suggestion(&self, task: Task, row: u64) -> anyhow::Result<Option<Entry>> {
        match self.key_of(row) {
            Some(key) => self.suggestions.get(task, &key),
            None => Ok(None),
        }
    }

    /// Of `rows`, those already described (or located) with this model and prompt: a run
    /// skips them.
    pub fn described(&self, task: Task, rows: &[u64], model: &str, prompt: u64) -> anyhow::Result<HashSet<u64>> {
        let keyed: Vec<(u64, String)> = rows.iter().filter_map(|&r| Some((r, self.key_of(r)?))).collect();
        // Only these photos' entries are read, not the whole table.
        let found = self.suggestions.get_many(task, keyed.iter().map(|(_, k)| k.as_str()))?;
        Ok(keyed
            .into_iter()
            .filter(|(_, k)| found.get(k).is_some_and(|e| e.model == model && e.prompt == prompt))
            .map(|(r, _)| r)
            .collect())
    }

    /// Stores what one call found for a scene's photos: the place checked against the
    /// gazetteer once, and against each photo's own neighbours in time; the call recorded
    /// once and the entries written together. The first photo keeps the call's cost; every
    /// photo keeps what the model was told, as any of them may stand for the scene's card.
    pub fn put_scene(&self, task: Task, rows: &[u64], entry: &Entry, record: &RunRecord) -> anyhow::Result<()> {
        let mut checked = entry.clone();
        if let Some(place) = checked.suggestion.place.as_mut() {
            checked.warnings.extend(check_place(place));
        }
        let mut entries = Vec::with_capacity(rows.len());
        for (i, &row) in rows.iter().enumerate() {
            let Some(key) = self.key_of(row) else { continue };
            let mut own = if i == 0 { checked.clone() } else { Entry { cost: 0.0, ..checked.clone() } };
            if task == Task::Locate
                && let Some(place) = own.suggestion.place.as_mut()
            {
                own.warnings.extend(self.check_reach(row, place)?);
            }
            entries.push((key, own));
        }
        self.suggestions.put_many(task, &entries, record)
    }

    /// Stores a photo's suggestion, its place checked against the gazetteer first and, a
    /// place found by a locating run, against where the photos taken around it were.
    pub fn put_suggestion(&self, task: Task, row: u64, entry: &Entry, record: &RunRecord) -> anyhow::Result<()> {
        let Some(key) = self.key_of(row) else { return Ok(()) };
        let entry = self.checked(task, row, entry)?;
        self.suggestions.put(task, &key, &entry, record)
    }

    /// [`Library::put_suggestion`] for a photo whose call is recorded with another (a
    /// contact sheet's).
    pub fn put_suggestion_only(&self, task: Task, row: u64, entry: &Entry) -> anyhow::Result<()> {
        let Some(key) = self.key_of(row) else { return Ok(()) };
        let entry = self.checked(task, row, entry)?;
        self.suggestions.put_entry(task, &key, &entry)
    }

    fn checked(&self, task: Task, row: u64, entry: &Entry) -> anyhow::Result<Entry> {
        let mut entry = entry.clone();
        if let Some(place) = entry.suggestion.place.as_mut() {
            entry.warnings.extend(check_place(place));
            if task == Task::Locate {
                entry.warnings.extend(self.check_reach(row, place)?);
            }
        }
        Ok(entry)
    }

    /// Records a call that gave no suggestion: it was paid for all the same.
    pub fn record_call(&self, row: u64, record: &RunRecord) -> anyhow::Result<()> {
        let Some(key) = self.key_of(row) else { return Ok(()) };
        self.suggestions.record(&key, record)
    }

    /// Records a call about no photo (lessons refined), so its cost counts with the rest.
    pub fn record_refine(&self, record: &RunRecord) -> anyhow::Result<()> {
        self.suggestions.record(REFINE_KEY, record)
    }

    /// Stored suggestions of photos in the library, with `status` or any, newest first.
    pub fn suggestions(&self, task: Task, status: Option<Status>) -> anyhow::Result<Vec<(u64, Entry)>> {
        let mut all = self.suggestions.all(task)?;
        all.retain(|_, e| status.is_none_or(|s| e.status == s));
        let rows: Vec<(u64, String)> = self
            .store
            .rows_of(all.keys().map(String::as_str))?
            .into_iter()
            .map(|(row, key)| (row, key.to_owned()))
            .collect();
        // Moved out, not cloned: the entries hold the model's whole answer.
        let mut out: Vec<(u64, Entry)> =
            rows.into_iter().filter_map(|(row, key)| Some((row, all.remove(&key)?))).collect();
        out.sort_by(|a, b| b.1.at.cmp(&a.1.at).then(a.0.cmp(&b.0)));
        Ok(out)
    }

    /// Sets the status of the photos' suggestions. Rejecting one under review is kept as a
    /// review too, in place of the photo's review before (it was reviewed again).
    pub fn set_suggestion_status(&self, task: Task, rows: &[u64], status: Status) -> anyhow::Result<()> {
        let keys: Vec<String> = rows.iter().filter_map(|&r| self.key_of(r)).collect();
        let mut reviews = Vec::new();
        if status == Status::Rejected {
            for key in &keys {
                if let Some(entry) = self.suggestions.get(task, key)?.filter(|e| e.status == Status::Proposed) {
                    reviews.push(review_of(task, key.clone(), entry, Status::Rejected, None));
                }
            }
        }
        self.suggestions.settle(task, &keys.iter().map(String::as_str).collect::<Vec<_>>(), status, &reviews)
    }

    /// The suggestions reviewed, newest first, until removed: what lessons are drawn from;
    /// and how many reviews could not be read.
    pub fn reviews(&self) -> anyhow::Result<(Vec<ReviewRow>, usize)> {
        let (list, unreadable) = self.suggestions.reviews()?;
        let rows: HashMap<&str, u64> = self
            .store
            .rows_of(list.iter().map(|(_, r)| r.key.as_str()))?
            .into_iter()
            .map(|(row, k)| (k, row))
            .collect();
        let list = list
            .iter()
            .map(|(seq, review)| ReviewRow {
                seq: *seq,
                row: rows.get(review.key.as_str()).copied(),
                review: review.clone(),
            })
            .collect();
        Ok((list, unreadable))
    }

    pub fn delete_reviews(&self, seqs: &[u64]) -> anyhow::Result<()> {
        self.suggestions.delete_reviews(seqs)
    }

    /// Calls made and what they cost in all, in US dollars.
    pub fn ai_spent(&self) -> anyhow::Result<(u64, f64)> {
        self.suggestions.spent()
    }

    /// Stores position marks (`None` takes one off) and keeps them in memory.
    fn set_marks(
        &self,
        coarse: &[(String, Option<Coarse>)],
        guessed: &[(String, Option<Guessed>)],
    ) -> anyhow::Result<()> {
        if coarse.is_empty() && guessed.is_empty() {
            return Ok(());
        }
        self.place_book.set_coarse(coarse)?;
        self.place_book.set_guessed(guessed)?;
        {
            let mut marks = self.coarse.write().unwrap();
            for (key, mark) in coarse {
                match mark {
                    Some(mark) => marks.insert(key.clone(), mark.clone()),
                    None => marks.remove(key),
                };
            }
        }
        {
            let mut marks = self.guessed.write().unwrap();
            for (key, mark) in guessed {
                match mark {
                    Some(mark) => marks.insert(key.clone(), *mark),
                    None => marks.remove(key),
                };
            }
        }
        self.fixes_changed();
        Ok(())
    }

    /// Whether a photo may be given a position from a suggestion: one with none, or for
    /// a locating run one with only an approximate position (its place's).
    fn may_place(&self, task: Task, photo: &c1model::Photo) -> bool {
        match photo.geo {
            None => true,
            Some(g) => {
                task == Task::Locate
                    && self.coarse.read().unwrap().get(&photo.id.key()).is_some_and(|c| c.holds(g.lat, g.lon))
            }
        }
    }

    /// Queues what the user took from suggestions, logged as coming from the model, and
    /// marks those suggestions accepted. Each photo's ops are made from the photo as it is
    /// now, so nothing edited since is undone; a caption, headline or rating that is no
    /// longer what the review showed is left alone (see [`Seen`]).
    pub fn accept_suggestions(&self, task: Task, picks: &[Accept]) -> anyhow::Result<Accepted> {
        let by_row: HashMap<u64, &Accept> = picks.iter().map(|p| (p.row, p)).collect();
        let rows: Vec<u64> = by_row.keys().copied().collect();
        // A position taken is marked before it is set: a mark on a photo not at its point
        // is ignored, a position without its mark would pass for one taken there. An
        // approximate one is marked as a place's position is (≈); a precise one as a
        // guess: shown as the photo's own, but never used to place or locate other photos.
        let mut coarse_marks = Vec::new();
        let mut guess_marks = Vec::new();
        // The position each photo may be given one over, as it was when marked: one set
        // since (in the panel, by a sync) is not replaced.
        let mut placeable: HashMap<u64, Option<GeoPoint>> = HashMap::new();
        for pick in picks {
            let Some([lat, lon]) = pick.point else { continue };
            let Some(photo) = self.photo(pick.row)? else { continue };
            if !self.may_place(task, &photo) {
                continue;
            }
            placeable.insert(pick.row, photo.geo);
            if pick.precise {
                guess_marks.push((photo.id.key(), Some(Guessed { lat, lon })));
                continue;
            }
            let (place, _) = taken_place(&photo.place, pick);
            let level = Level::named(&place).unwrap_or(Level::Country);
            let path = schema::place_tree(&place).pop().unwrap_or_default();
            coarse_marks.push((photo.id.key(), Some(Coarse { lat, lon, level, path, guess: true })));
        }
        self.set_marks(&coarse_marks, &guess_marks)?;
        let changed = std::sync::Mutex::new(HashSet::new());
        let undo = self.edit_photos_as(
            &rows,
            |row, photo| {
                let pick = by_row[&row];
                let mut ops = Vec::new();
                let mut stale = false;
                let paths: Vec<KeywordPath> = pick
                    .keywords
                    .iter()
                    .map(|k| KeywordPath::from_pipe(k))
                    .filter(|p| !p.0.is_empty() && !photo.keywords.contains(p))
                    .collect();
                if !paths.is_empty() {
                    ops.push(Op::AddKeywords { paths });
                }
                let mut iptc = photo.iptc.clone();
                if let Some(caption) = text(&pick.caption) {
                    if text(&photo.iptc.description) == text(&pick.seen.caption) {
                        iptc.description = Some(caption.to_string());
                    } else {
                        stale = true;
                    }
                }
                if let Some(headline) = text(&pick.headline) {
                    if text(&photo.iptc.headline) == text(&pick.seen.headline) {
                        iptc.headline = Some(headline.to_string());
                    } else {
                        stale = true;
                    }
                }
                if iptc != photo.iptc {
                    ops.push(Op::SetIptc { base: photo.iptc.clone(), value: iptc });
                }
                if let Some(rating) = pick.rating.filter(|r| (1..=5).contains(r))
                    && photo.rating != Some(rating)
                {
                    if photo.rating == pick.seen.rating {
                        ops.push(Op::SetRating { base: photo.rating, value: Some(rating) });
                    } else {
                        stale = true;
                    }
                }
                let (place, place_stale) = taken_place(&photo.place, pick);
                stale |= place_stale;
                if place != photo.place {
                    ops.push(Op::SetPlace { base: photo.place.clone(), value: place });
                }
                // Never over a position of the photo's own.
                if let Some([lat, lon]) = pick.point
                    && placeable.get(&row).is_some_and(|was| *was == photo.geo)
                    && photo.geo.is_none_or(|g| !same_spot((g.lat, g.lon), (lat, lon)))
                {
                    let origin = if pick.precise { GeoOrigin::Manual } else { GeoOrigin::Geocoded };
                    let value = GeoPoint { lat, lon, alt: None, origin };
                    ops.push(Op::SetGeo { base: photo.geo, value: Some(value) });
                }
                if stale {
                    changed.lock().unwrap().insert(row);
                }
                Ok(ops)
            },
            Origin::Ai,
        )?;
        // A photo now at the position taken loses the mark of the other kind it had (≈ on
        // one taken as precise, a guess on one taken as ≈); one the edit left where it was
        // keeps its marks.
        let (mut coarse_off, mut guess_off) = (Vec::new(), Vec::new());
        for pick in picks {
            let Some([lat, lon]) = pick.point.filter(|_| placeable.contains_key(&pick.row)) else { continue };
            let Some(photo) = self.photo(pick.row)? else { continue };
            if !photo.geo.is_some_and(|g| same_spot((g.lat, g.lon), (lat, lon))) {
                continue;
            }
            let key = photo.id.key();
            if pick.precise {
                if self.coarse.read().unwrap().contains_key(&key) {
                    coarse_off.push((key, None));
                }
            } else if self.guessed.read().unwrap().contains_key(&key) {
                guess_off.push((key, None));
            }
        }
        self.set_marks(&coarse_off, &guess_off)?;
        let changed = changed.into_inner().unwrap();
        let done: Vec<u64> = rows.iter().copied().filter(|r| !changed.contains(r)).collect();
        let mut keys = Vec::new();
        let mut reviews = Vec::new();
        for &row in &done {
            let Some(key) = self.key_of(row) else { continue };
            if let Some(entry) = self.suggestions.get(task, &key)? {
                reviews.push(review_of(task, key.clone(), entry, Status::Accepted, Some(Taken::from(by_row[&row]))));
            }
            keys.push(key);
        }
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        self.suggestions.settle(task, &keys, Status::Accepted, &reviews)?;
        let mut changed: Vec<u64> = changed.into_iter().collect();
        changed.sort_unstable();
        Ok(Accepted { undo, changed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use c1ai::{Guess, PointGuess};

    fn laski(lat: f64, lon: f64) -> PlaceGuess {
        let g = |name: &str, p: f32| Some(Guess { name: name.into(), p });
        PlaceGuess {
            country: g("Poland", 0.99),
            state: g("Małopolska", 0.98),
            city: g("Laski", 0.96),
            point: Some(PointGuess { lat, lon, p: 0.9, radius_km: 1.0 }),
            ..PlaceGuess::default()
        }
    }

    #[test]
    fn a_point_by_the_village_named_is_not_doubted_for_the_town_around_it() {
        // The piezometer in Laski, which the gazetteer finds in Bolesław's reach.
        let mut near = laski(50.3233, 19.4725);
        let why = check_place(&mut near);
        assert_eq!(near.point.unwrap().p, 0.9, "{why:?}");
        // In another region: doubted.
        let mut far = laski(52.2297, 21.0122);
        check_place(&mut far);
        assert_eq!(far.point.unwrap().p, DOUBTED, "Warsaw");
        // Kraków, in the region named, is in reach of the namesake rule: a city found where
        // the gazetteer knows the city named is still doubted (Wieliczka for Kraków).
        let g = |name: &str, p: f32| Some(Guess { name: name.into(), p });
        let mut krakow = PlaceGuess {
            country: g("Poland", 0.99),
            state: g("Małopolska", 0.98),
            city: g("Kraków", 0.96),
            point: Some(PointGuess { lat: 49.9839, lon: 20.0547, p: 0.9, radius_km: 1.0 }),
            ..PlaceGuess::default()
        };
        check_place(&mut krakow);
        assert_eq!(krakow.point.unwrap().p, DOUBTED, "Wieliczka");
    }

    #[test]
    fn a_point_by_a_border_is_where_the_country_named_says() {
        // Tin Shui Wai, in Hong Kong: Shenzhen's reach covers it better.
        let g = |name: &str, p: f32| Some(Guess { name: name.into(), p });
        let mut hk = PlaceGuess {
            country: g("Hong Kong", 0.99),
            city: g("Hong Kong", 0.99),
            location: g("Tin Shui Wai", 0.92),
            point: Some(PointGuess { lat: 22.4488, lon: 114.0031, p: 0.9, radius_km: 0.5 }),
            ..PlaceGuess::default()
        };
        let why = check_place(&mut hk);
        assert_eq!(hk.point.unwrap().p, 0.9, "{why:?}");
        assert_eq!(hk.country_code.as_deref(), Some("HK"));
        // Named as China, it is in Shenzhen's reach: as it was.
        let mut cn = PlaceGuess {
            country: g("China", 0.99),
            city: g("Shenzhen", 0.9),
            point: Some(PointGuess { lat: 22.4488, lon: 114.0031, p: 0.9, radius_km: 0.5 }),
            ..PlaceGuess::default()
        };
        check_place(&mut cn);
        assert_eq!(cn.point.unwrap().p, 0.9);
    }
}
