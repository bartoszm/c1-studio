//! What a vision model suggested for photos (stage 5), kept until the user accepts or
//! rejects it, a record of every call made (the set to judge prompts and models by), and
//! of every suggestion reviewed with what the user took from it: the corrections that
//! lessons for the prompt are drawn from.
//!
//! Kept in its own file (`ai.redb`), like analyses: calls cost money, so suggestions
//! outlive the store, which is wiped when the model changes. Photos are named by their
//! key, which a rebuilt store keeps. A photo's description and the place a locating run
//! found for it are kept apart ([`Task`]): one never replaces the other.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::db::OpenControl;
pub use c1ai::Task;
use c1ai::{RunRecord, Suggestion};
use c1model::Place;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

/// photo key → JSON [`Entry`]: the newest description per photo. JSON with fields
/// defaulted, so fields can be added without moving the old entries over.
const SUGGESTIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("suggestions_v3");
/// photo key → JSON [`Entry`]: the newest place a locating run found per photo.
const LOCATIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("locations");
/// The descriptions as postcard ([`v2::Entry`]), moved over on opening.
const SUGGESTIONS_V2: TableDefinition<&str, &[u8]> = TableDefinition::new("suggestions_v2");
/// The same with a place of a city and a country ([`v1::Entry`]), moved over on opening.
const SUGGESTIONS_V1: TableDefinition<&str, &[u8]> = TableDefinition::new("suggestions");

/// Where a task's entries are kept.
fn table(task: Task) -> TableDefinition<'static, &'static str, &'static [u8]> {
    match task {
        Task::Describe => SUGGESTIONS,
        Task::Locate => LOCATIONS,
    }
}

fn decode(bytes: &[u8]) -> Option<Entry> {
    serde_json::from_slice(bytes).ok()
}
/// sequence → postcard (photo key, [`RunRecord`]): every call, never rewritten.
const RECORDS: TableDefinition<u64, &[u8]> = TableDefinition::new("records");
/// sequence → JSON [`Review`]: the last time each photo's suggestion was accepted or
/// rejected, until the user removes it. JSON, not postcard, with every field defaulted:
/// fields can be added without moving the old ones over.
const REVIEWS: TableDefinition<u64, &[u8]> = TableDefinition::new("reviews");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    #[default]
    Proposed,
    Accepted,
    Rejected,
}

/// A photo's suggestion as stored: what was suggested, how, and what the user did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    /// Epoch milliseconds of the call.
    pub at: i64,
    pub provider: String,
    pub model: String,
    pub prompt: u64,
    pub suggestion: Suggestion,
    pub warnings: Vec<String>,
    pub cost: f64,
    pub status: Status,
    /// What the model was told of the photo besides the image (a locating run's clues
    /// from the catalog), one line each.
    pub context: Vec<String>,
    /// Photos located together, in one call, share this: the first one's row then.
    pub group: Option<u64>,
}

/// What the user took from a suggestion, as the review had it when accepted: the names
/// edited, keywords added and taken off.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Taken {
    /// Pipe paths ticked, those the photo had included.
    pub keywords: Vec<String>,
    pub caption: Option<String>,
    pub headline: Option<String>,
    pub rating: Option<u8>,
    /// The place levels taken; `None` where none was.
    pub place: Place,
    /// The guessed position was taken.
    pub point: bool,
    /// ... as a precise one, not marked approximate.
    pub precise: bool,
}

/// A suggestion reviewed: what was suggested, and what the user did with it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Review {
    /// Epoch milliseconds of the review.
    pub at: i64,
    /// The photo's key.
    pub key: String,
    pub model: String,
    pub prompt: u64,
    /// What the suggestion was for: a description, or a place found.
    pub task: Task,
    /// Accepted or rejected.
    pub status: Status,
    pub suggestion: Suggestion,
    /// For one accepted.
    pub taken: Option<Taken>,
}

pub struct SuggestionBook {
    db: Database,
}

impl SuggestionBook {
    pub fn open(path: &Path, ctl: &OpenControl) -> anyhow::Result<Self> {
        let db = crate::db::open(path, "suggestions", ctl)?;
        let tx = db.begin_write()?;
        {
            let mut table = tx.open_table(SUGGESTIONS)?;
            tx.open_table(LOCATIONS)?;
            tx.open_table(RECORDS)?;
            tx.open_table(REVIEWS)?;
            // Postcard does not say what it holds, so the older entries are read as they
            // were written and stored again in the new shape: the newer shape first, as
            // one already in the new table is newer (an older build ran in between).
            for row in tx.open_table(SUGGESTIONS_V2)?.iter()? {
                let (key, bytes) = row?;
                if table.get(key.value())?.is_none()
                    && let Ok(entry) = postcard::from_bytes::<v2::Entry>(bytes.value())
                {
                    table.insert(key.value(), serde_json::to_vec(&Entry::from(entry))?.as_slice())?;
                }
            }
            for row in tx.open_table(SUGGESTIONS_V1)?.iter()? {
                let (key, bytes) = row?;
                if table.get(key.value())?.is_none()
                    && let Ok(entry) = postcard::from_bytes::<v1::Entry>(bytes.value())
                {
                    table.insert(key.value(), serde_json::to_vec(&Entry::from(entry))?.as_slice())?;
                }
            }
        }
        tx.delete_table(SUGGESTIONS_V1)?;
        tx.delete_table(SUGGESTIONS_V2)?;
        tx.commit()?;
        Ok(Self { db })
    }

    /// Every stored suggestion of a task, by photo key. One that no longer reads is left
    /// out (its photo is described again when asked).
    pub fn all(&self, task: Task) -> anyhow::Result<HashMap<String, Entry>> {
        let tx = self.db.begin_read()?;
        let mut out = HashMap::new();
        for entry in tx.open_table(table(task))?.iter()? {
            let (key, bytes) = entry?;
            if let Some(e) = decode(bytes.value()) {
                out.insert(key.value().to_owned(), e);
            }
        }
        Ok(out)
    }

    pub fn get(&self, task: Task, key: &str) -> anyhow::Result<Option<Entry>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(table(task))?;
        Ok(table.get(key)?.and_then(|b| decode(b.value())))
    }

    /// Stores a photo's new suggestion without a call of its own: one of a contact sheet's
    /// photos, the call recorded with another.
    pub fn put_entry(&self, task: Task, key: &str, entry: &Entry) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        tx.open_table(table(task))?.insert(key, serde_json::to_vec(entry)?.as_slice())?;
        tx.commit()?;
        Ok(())
    }

    /// Stores a photo's new suggestion, replacing the one before, with the call's record.
    pub fn put(&self, task: Task, key: &str, entry: &Entry, record: &RunRecord) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            tx.open_table(table(task))?.insert(key, serde_json::to_vec(entry)?.as_slice())?;
            let mut records = tx.open_table(RECORDS)?;
            let next = records.last()?.map(|(k, _)| k.value() + 1).unwrap_or(0);
            records.insert(next, postcard::to_stdvec(&(key, record))?.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Records a call whose answer was not usable (declined, cut off): it cost money too.
    pub fn record(&self, key: &str, record: &RunRecord) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut records = tx.open_table(RECORDS)?;
            let next = records.last()?.map(|(k, _)| k.value() + 1).unwrap_or(0);
            records.insert(next, postcard::to_stdvec(&(key, record))?.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Sets the status of the photos' suggestions (photos without one are skipped) and keeps
    /// `reviews`, in one go: a review is never kept for a status that was not set. A photo's
    /// review replaces the one it had (reviewed again: the last word counts), each under the
    /// next number.
    pub fn settle(&self, task: Task, keys: &[&str], status: Status, reviews: &[Review]) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(table(task))?;
            for key in keys {
                let Some(mut entry) = table.get(*key)?.and_then(|b| decode(b.value())) else {
                    continue;
                };
                entry.status = status;
                table.insert(*key, serde_json::to_vec(&entry)?.as_slice())?;
            }
            if !reviews.is_empty() {
                let mut table = tx.open_table(REVIEWS)?;
                let again: HashSet<(&str, Task)> = reviews.iter().map(|r| (r.key.as_str(), r.task)).collect();
                // Only the key and the task are read of the others.
                #[derive(Deserialize)]
                struct Keyed {
                    #[serde(default)]
                    key: String,
                    #[serde(default)]
                    task: Task,
                }
                table.retain(|_, bytes| {
                    serde_json::from_slice::<Keyed>(bytes).map_or(true, |r| !again.contains(&(r.key.as_str(), r.task)))
                })?;
                let first = table.last()?.map(|(k, _)| k.value() + 1).unwrap_or(0);
                for (seq, review) in (first..).zip(reviews) {
                    table.insert(seq, serde_json::to_vec(review)?.as_slice())?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Every review kept, by its number, newest first, and how many could not be read.
    pub fn reviews(&self) -> anyhow::Result<(Vec<(u64, Review)>, usize)> {
        let tx = self.db.begin_read()?;
        let mut out = Vec::new();
        let mut unreadable = 0;
        for entry in tx.open_table(REVIEWS)?.iter()?.rev() {
            let (seq, bytes) = entry?;
            match serde_json::from_slice::<Review>(bytes.value()) {
                Ok(review) => out.push((seq.value(), review)),
                Err(_) => unreadable += 1,
            }
        }
        Ok((out, unreadable))
    }

    pub fn delete_reviews(&self, seqs: &[u64]) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(REVIEWS)?;
            for seq in seqs {
                table.remove(*seq)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Calls made and what they cost in all, in US dollars.
    pub fn spent(&self) -> anyhow::Result<(u64, f64)> {
        let tx = self.db.begin_read()?;
        let mut calls = 0;
        let mut cost = 0.0;
        for entry in tx.open_table(RECORDS)?.iter()? {
            let (_, bytes) = entry?;
            if let Ok((_, record)) = postcard::from_bytes::<(String, RunRecord)>(bytes.value()) {
                calls += 1;
                cost += record.cost;
            }
        }
        Ok((calls, cost))
    }
}

/// Entries as stored in postcard before locating runs: a place without its evidence.
mod v2 {
    use c1ai::{Guess, PointGuess};
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct Entry {
        pub at: i64,
        pub provider: String,
        pub model: String,
        pub prompt: u64,
        pub suggestion: Suggestion,
        pub warnings: Vec<String>,
        pub cost: f64,
        pub status: super::Status,
    }

    #[derive(Deserialize)]
    pub struct Suggestion {
        pub caption_en: Option<String>,
        pub caption_pl: Option<String>,
        pub headline: Option<String>,
        pub subject: Option<String>,
        pub keywords: Vec<String>,
        pub keywords_new: Vec<String>,
        pub rating: Option<u8>,
        pub quality: Option<u8>,
        pub composition: Option<u8>,
        pub confidence: f32,
        pub place: Option<PlaceGuess>,
    }

    #[derive(Deserialize)]
    pub struct PlaceGuess {
        pub country: Option<Guess>,
        pub state: Option<Guess>,
        pub city: Option<Guess>,
        pub location: Option<Guess>,
        pub country_code: Option<String>,
        pub point: Option<PointGuess>,
    }
}

impl From<v2::Entry> for Entry {
    fn from(e: v2::Entry) -> Self {
        let s = e.suggestion;
        let place = s.place.map(|p| c1ai::PlaceGuess {
            country: p.country,
            state: p.state,
            city: p.city,
            location: p.location,
            country_code: p.country_code,
            point: p.point,
            ..c1ai::PlaceGuess::default()
        });
        Entry {
            at: e.at,
            provider: e.provider,
            model: e.model,
            prompt: e.prompt,
            suggestion: Suggestion {
                caption_en: s.caption_en,
                caption_pl: s.caption_pl,
                headline: s.headline,
                subject: s.subject,
                keywords: s.keywords,
                keywords_new: s.keywords_new,
                rating: s.rating,
                quality: s.quality,
                composition: s.composition,
                confidence: s.confidence,
                place,
            },
            warnings: e.warnings,
            cost: e.cost,
            status: e.status,
            ..Entry::default()
        }
    }
}

/// Entries as stored by the first builds of the AI tab, whose place was a city and a
/// country with one confidence.
mod v1 {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct Entry {
        pub at: i64,
        pub provider: String,
        pub model: String,
        pub prompt: u64,
        pub suggestion: Suggestion,
        pub warnings: Vec<String>,
        pub cost: f64,
        pub status: super::Status,
    }

    #[derive(Deserialize)]
    pub struct Suggestion {
        pub caption_en: Option<String>,
        pub caption_pl: Option<String>,
        pub headline: Option<String>,
        pub subject: Option<String>,
        pub keywords: Vec<String>,
        pub keywords_new: Vec<String>,
        pub rating: Option<u8>,
        pub quality: Option<u8>,
        pub composition: Option<u8>,
        pub confidence: f32,
        pub place: Option<PlaceGuess>,
    }

    #[derive(Deserialize)]
    pub struct PlaceGuess {
        pub city: Option<String>,
        pub country: Option<String>,
        pub confidence: f32,
    }
}

impl From<v1::Entry> for Entry {
    fn from(e: v1::Entry) -> Self {
        let s = e.suggestion;
        let place = s.place.map(|p| {
            let guess = |name: Option<String>| name.map(|name| c1ai::Guess { name, p: p.confidence });
            c1ai::PlaceGuess { country: guess(p.country), city: guess(p.city), ..c1ai::PlaceGuess::default() }
        });
        Entry {
            at: e.at,
            provider: e.provider,
            model: e.model,
            prompt: e.prompt,
            suggestion: Suggestion {
                caption_en: s.caption_en,
                caption_pl: s.caption_pl,
                headline: s.headline,
                subject: s.subject,
                keywords: s.keywords,
                keywords_new: s.keywords_new,
                rating: s.rating,
                quality: s.quality,
                composition: s.composition,
                confidence: s.confidence,
                place,
            },
            warnings: e.warnings,
            cost: e.cost,
            status: e.status,
            ..Entry::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(status: Status) -> Entry {
        Entry {
            at: 1,
            provider: "claude".into(),
            model: "claude-haiku-5-5".into(),
            prompt: 7,
            suggestion: Suggestion { headline: Some("Tram at dusk".into()), ..Suggestion::default() },
            warnings: vec![],
            cost: 0.002,
            status,
            ..Entry::default()
        }
    }

    #[test]
    fn suggestions_are_replaced_and_every_call_is_counted() {
        let dir = tempfile::tempdir().unwrap();
        let book = SuggestionBook::open(&dir.path().join("ai.redb"), &Default::default()).unwrap();
        let record = RunRecord { cost: 0.002, ..RunRecord::default() };
        book.put(Task::Describe, "c1:1", &entry(Status::Proposed), &record).unwrap();
        book.put(Task::Describe, "c1:1", &entry(Status::Proposed), &record).unwrap();
        book.record("c1:2", &RunRecord { cost: 0.001, ..RunRecord::default() }).unwrap();
        assert_eq!(book.all(Task::Describe).unwrap().len(), 1);
        let (calls, cost) = book.spent().unwrap();
        assert_eq!(calls, 3);
        assert!((cost - 0.005).abs() < 1e-12);

        book.settle(Task::Describe, &["c1:1", "c1:9"], Status::Rejected, &[]).unwrap();
        assert_eq!(book.get(Task::Describe, "c1:1").unwrap().unwrap().status, Status::Rejected);
    }

    #[test]
    fn a_place_found_is_kept_apart_from_the_description() {
        let dir = tempfile::tempdir().unwrap();
        let book = SuggestionBook::open(&dir.path().join("ai.redb"), &Default::default()).unwrap();
        let record = RunRecord::default();
        book.put(Task::Describe, "c1:1", &entry(Status::Proposed), &record).unwrap();
        let located = Entry { context: vec!["4 min after a photo at 50.06, 19.94".into()], ..entry(Status::Proposed) };
        book.put(Task::Locate, "c1:1", &located, &record).unwrap();
        book.settle(Task::Locate, &["c1:1"], Status::Accepted, &[]).unwrap();
        assert_eq!(book.get(Task::Describe, "c1:1").unwrap().unwrap().status, Status::Proposed);
        let found = book.get(Task::Locate, "c1:1").unwrap().unwrap();
        assert_eq!((found.status, found.context.len()), (Status::Accepted, 1));
        assert_eq!(book.spent().unwrap().0, 2);
    }

    #[test]
    fn suggestions_stored_as_postcard_are_kept() {
        #[derive(Serialize)]
        struct OldPlace(
            Option<c1ai::Guess>,
            Option<c1ai::Guess>,
            Option<c1ai::Guess>,
            Option<c1ai::Guess>,
            Option<String>,
            Option<c1ai::PointGuess>,
        );
        #[derive(Serialize)]
        #[allow(clippy::type_complexity)]
        struct OldSuggestion(
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Vec<String>,
            Vec<String>,
            Option<u8>,
            Option<u8>,
            Option<u8>,
            f32,
            Option<OldPlace>,
        );
        #[derive(Serialize)]
        struct OldEntry(i64, String, String, u64, OldSuggestion, Vec<String>, f64, Status);
        let city = c1ai::Guess { name: "Kraków".into(), p: 0.7 };
        let point = c1ai::PointGuess { lat: 50.06, lon: 19.94, p: 0.5, radius_km: 4.0 };
        let old = OldEntry(
            1,
            "claude".into(),
            "claude-haiku-5-5".into(),
            7,
            OldSuggestion(
                None,
                None,
                Some("Tram at dusk".into()),
                None,
                vec!["street".into()],
                vec![],
                Some(4),
                None,
                None,
                0.8,
                Some(OldPlace(None, None, Some(city.clone()), None, Some("PL".into()), Some(point.clone()))),
            ),
            vec!["a warning".into()],
            0.002,
            Status::Proposed,
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai.redb");
        {
            let db = Database::create(&path).unwrap();
            let tx = db.begin_write().unwrap();
            tx.open_table(SUGGESTIONS_V2)
                .unwrap()
                .insert("c1:1", postcard::to_stdvec(&old).unwrap().as_slice())
                .unwrap();
            tx.commit().unwrap();
        }
        let book = SuggestionBook::open(&path, &Default::default()).unwrap();
        let entry = book.get(Task::Describe, "c1:1").unwrap().expect("moved over");
        assert_eq!(entry.suggestion.keywords, ["street"]);
        assert_eq!(entry.warnings, ["a warning"]);
        let place = entry.suggestion.place.unwrap();
        assert_eq!((place.city, place.point, place.country_code.as_deref()), (Some(city), Some(point), Some("PL")));
    }

    #[test]
    fn suggestions_from_the_first_builds_are_kept() {
        // Written as the first builds did: the same fields, the place a city and a country.
        #[derive(Serialize)]
        struct OldPlace(Option<String>, Option<String>, f32);
        #[derive(Serialize)]
        #[allow(clippy::type_complexity)]
        struct OldSuggestion(
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Vec<String>,
            Vec<String>,
            Option<u8>,
            Option<u8>,
            Option<u8>,
            f32,
            Option<OldPlace>,
        );
        #[derive(Serialize)]
        struct OldEntry(i64, String, String, u64, OldSuggestion, Vec<String>, f64, Status);
        let old = OldEntry(
            1,
            "claude".into(),
            "claude-haiku-5-5".into(),
            7,
            OldSuggestion(
                None,
                None,
                Some("Tram at dusk".into()),
                None,
                vec![],
                vec![],
                Some(4),
                None,
                None,
                0.8,
                Some(OldPlace(Some("Kraków".into()), Some("Poland".into()), 0.6)),
            ),
            vec![],
            0.002,
            Status::Accepted,
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai.redb");
        {
            let db = Database::create(&path).unwrap();
            let tx = db.begin_write().unwrap();
            tx.open_table(SUGGESTIONS_V1)
                .unwrap()
                .insert("c1:1", postcard::to_stdvec(&old).unwrap().as_slice())
                .unwrap();
            tx.commit().unwrap();
        }
        let book = SuggestionBook::open(&path, &Default::default()).unwrap();
        let entry = book.get(Task::Describe, "c1:1").unwrap().expect("moved over");
        assert_eq!((entry.status, entry.suggestion.rating), (Status::Accepted, Some(4)));
        let place = entry.suggestion.place.unwrap();
        assert_eq!(place.city, Some(c1ai::Guess { name: "Kraków".into(), p: 0.6 }));
        assert_eq!(place.country.map(|g| g.name).as_deref(), Some("Poland"));
        drop(book);
        assert_eq!(
            SuggestionBook::open(&path, &Default::default()).unwrap().all(Task::Describe).unwrap().len(),
            1,
            "opened again: still there"
        );
    }
}
