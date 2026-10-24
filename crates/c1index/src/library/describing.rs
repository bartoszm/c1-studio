//! Suggestions from a vision model (stage 5): stored per photo until the user accepts
//! them into the edit queue, as edits from [`Origin::Ai`], or rejects them.

use std::collections::{HashMap, HashSet};

use c1ai::{PlaceGuess, RunRecord};
use c1geo::{Level, gazetteer};
use c1model::{GeoOrigin, GeoPoint, KeywordPath, Op, Place};
use serde::{Deserialize, Serialize};

use super::Library;
use crate::ai::{Entry, Review, Status, Taken};
use crate::edits::Origin;
use crate::places::Coarse;
use crate::schema;

/// How likely a place level the gazetteer does not know (or knows elsewhere) is left at
/// most: a hint for the review, never a tick by default.
const DOUBTED: f32 = 0.5;

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
    /// A position taken, `[lat, lon]`: given only to a photo with none, and marked as
    /// approximate (≈), like a place's position.
    pub point: Option<[f64; 2]>,
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

fn review_of(key: String, entry: Entry, status: Status, taken: Option<Taken>) -> Review {
    Review { at: now_ms(), key, model: entry.model, prompt: entry.prompt, status, suggestion: entry.suggestion, taken }
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
    if let Some(point) = guess.point.as_mut()
        && let Some(at) = g.reverse(point.lat, point.lon)
    {
        let differ = |named: Option<u32>, there: Option<u32>| named.is_some() && there.is_some() && named != there;
        if (differ(found.country, at.ids.country) || differ(found.city, at.ids.city)) && point.p > DOUBTED {
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

    pub fn suggestion(&self, row: u64) -> anyhow::Result<Option<Entry>> {
        match self.key_of(row) {
            Some(key) => self.suggestions.get(&key),
            None => Ok(None),
        }
    }

    /// Of `rows`, those already described with this model and prompt: a run skips them.
    pub fn described(&self, rows: &[u64], model: &str, prompt: u64) -> anyhow::Result<HashSet<u64>> {
        let all = self.suggestions.all()?;
        Ok(rows
            .iter()
            .copied()
            .filter(|&row| {
                self.key_of(row).and_then(|k| all.get(&k)).is_some_and(|e| e.model == model && e.prompt == prompt)
            })
            .collect())
    }

    /// Stores a photo's suggestion, its place checked against the gazetteer first.
    pub fn put_suggestion(&self, row: u64, entry: &Entry, record: &RunRecord) -> anyhow::Result<()> {
        let Some(key) = self.key_of(row) else { return Ok(()) };
        let mut entry = entry.clone();
        if let Some(place) = entry.suggestion.place.as_mut() {
            entry.warnings.extend(check_place(place));
        }
        self.suggestions.put(&key, &entry, record)
    }

    /// [`Library::put_suggestion`] for a photo whose call is recorded with another (a
    /// contact sheet's).
    pub fn put_suggestion_only(&self, row: u64, entry: &Entry) -> anyhow::Result<()> {
        let Some(key) = self.key_of(row) else { return Ok(()) };
        let mut entry = entry.clone();
        if let Some(place) = entry.suggestion.place.as_mut() {
            entry.warnings.extend(check_place(place));
        }
        self.suggestions.put_entry(&key, &entry)
    }

    /// Records a call that gave no suggestion: it was paid for all the same.
    pub fn record_call(&self, row: u64, record: &RunRecord) -> anyhow::Result<()> {
        let Some(key) = self.key_of(row) else { return Ok(()) };
        self.suggestions.record(&key, record)
    }

    /// Stored suggestions of photos in the library, with `status` or any, newest first.
    pub fn suggestions(&self, status: Option<Status>) -> anyhow::Result<Vec<(u64, Entry)>> {
        let all = self.suggestions.all()?;
        let mut out: Vec<(u64, Entry)> = self
            .store
            .rows_of(all.keys().map(String::as_str))?
            .into_iter()
            .filter_map(|(row, key)| all.get(key).map(|e| (row, e.clone())))
            .filter(|(_, e)| status.is_none_or(|s| e.status == s))
            .collect();
        out.sort_by(|a, b| b.1.at.cmp(&a.1.at).then(a.0.cmp(&b.0)));
        Ok(out)
    }

    /// Sets the status of the photos' suggestions. Rejecting one under review is kept as a
    /// review too.
    pub fn set_suggestion_status(&self, rows: &[u64], status: Status) -> anyhow::Result<()> {
        let keys: Vec<String> = rows.iter().filter_map(|&r| self.key_of(r)).collect();
        if status == Status::Rejected {
            let mut reviews = Vec::new();
            for key in &keys {
                if let Some(entry) = self.suggestions.get(key)?.filter(|e| e.status == Status::Proposed) {
                    reviews.push(review_of(key.clone(), entry, Status::Rejected, None));
                }
            }
            self.suggestions.add_reviews(&reviews)?;
        }
        self.suggestions.set_status(&keys.iter().map(String::as_str).collect::<Vec<_>>(), status)
    }

    /// The suggestions reviewed, newest first, until removed: what lessons are drawn from.
    pub fn reviews(&self) -> anyhow::Result<Vec<ReviewRow>> {
        let list = self.suggestions.reviews()?;
        let rows: HashMap<&str, u64> = self
            .store
            .rows_of(list.iter().map(|(_, r)| r.key.as_str()))?
            .into_iter()
            .map(|(row, k)| (k, row))
            .collect();
        Ok(list
            .iter()
            .map(|(seq, review)| ReviewRow {
                seq: *seq,
                row: rows.get(review.key.as_str()).copied(),
                review: review.clone(),
            })
            .collect())
    }

    pub fn delete_reviews(&self, seqs: &[u64]) -> anyhow::Result<()> {
        self.suggestions.delete_reviews(seqs)
    }

    /// Calls made and what they cost in all, in US dollars.
    pub fn ai_spent(&self) -> anyhow::Result<(u64, f64)> {
        self.suggestions.spent()
    }

    /// Queues what the user took from suggestions, logged as coming from the model, and
    /// marks those suggestions accepted. Each photo's ops are made from the photo as it is
    /// now, so nothing edited since is undone; a caption, headline or rating that is no
    /// longer what the review showed is left alone (see [`Seen`]).
    pub fn accept_suggestions(&self, picks: &[Accept]) -> anyhow::Result<Accepted> {
        let by_row: HashMap<u64, &Accept> = picks.iter().map(|p| (p.row, p)).collect();
        let rows: Vec<u64> = by_row.keys().copied().collect();
        // A position taken is marked approximate before it is set, as a place's position
        // is: a mark on a photo not at its point is ignored, a position without its mark
        // would pass for one taken there.
        let mut marks = Vec::new();
        for pick in picks {
            let Some([lat, lon]) = pick.point else { continue };
            let Some(photo) = self.photo(pick.row)? else { continue };
            if photo.geo.is_some() {
                continue;
            }
            let (place, _) = taken_place(&photo.place, pick);
            let level = Level::named(&place).unwrap_or(Level::Country);
            let path = schema::place_tree(&place).pop().unwrap_or_default();
            marks.push((photo.id.key(), Some(Coarse { lat, lon, level, path })));
        }
        self.place_book.set_coarse(&marks)?;
        self.coarse.write().unwrap().extend(marks.into_iter().filter_map(|(k, c)| Some((k, c?))));
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
                // Never over a position the photo has.
                if let Some([lat, lon]) = pick.point
                    && photo.geo.is_none()
                {
                    let value = GeoPoint { lat, lon, alt: None, origin: GeoOrigin::Geocoded };
                    ops.push(Op::SetGeo { base: None, value: Some(value) });
                }
                if stale {
                    changed.lock().unwrap().insert(row);
                }
                Ok(ops)
            },
            Origin::Ai,
        )?;
        let changed = changed.into_inner().unwrap();
        let done: Vec<u64> = rows.iter().copied().filter(|r| !changed.contains(r)).collect();
        let mut reviews = Vec::new();
        for &row in &done {
            let Some(key) = self.key_of(row) else { continue };
            if let Some(entry) = self.suggestions.get(&key)? {
                reviews.push(review_of(key, entry, Status::Accepted, Some(Taken::from(by_row[&row]))));
            }
        }
        self.suggestions.add_reviews(&reviews)?;
        self.set_suggestion_status(&done, Status::Accepted)?;
        let mut changed: Vec<u64> = changed.into_iter().collect();
        changed.sort_unstable();
        Ok(Accepted { undo, changed })
    }
}
