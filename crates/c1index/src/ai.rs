//! What a vision model suggested for photos (stage 5), kept until the user accepts or
//! rejects it, and a record of every call made: the set to judge prompts and models by.
//!
//! Kept in its own file (`ai.redb`), like analyses: calls cost money, so suggestions
//! outlive the store, which is wiped when the model changes. Photos are named by their
//! key, which a rebuilt store keeps.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use c1ai::{RunRecord, Suggestion};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

/// photo key → postcard [`Entry`]: the newest suggestion per photo.
const SUGGESTIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("suggestions");
/// sequence → postcard (photo key, [`RunRecord`]): every call, never rewritten.
const RECORDS: TableDefinition<u64, &[u8]> = TableDefinition::new("records");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    #[default]
    Proposed,
    Accepted,
    Rejected,
}

/// A photo's suggestion as stored: what was suggested, how, and what the user did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
}

pub struct SuggestionBook {
    db: Database,
}

impl SuggestionBook {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let db = Database::create(path).with_context(|| format!("opening {}", path.display()))?;
        let tx = db.begin_write()?;
        tx.open_table(SUGGESTIONS)?;
        tx.open_table(RECORDS)?;
        tx.commit()?;
        Ok(Self { db })
    }

    /// Every stored suggestion, by photo key. One that no longer reads is left out (its
    /// photo is described again when asked).
    pub fn all(&self) -> anyhow::Result<HashMap<String, Entry>> {
        let tx = self.db.begin_read()?;
        let mut out = HashMap::new();
        for entry in tx.open_table(SUGGESTIONS)?.iter()? {
            let (key, bytes) = entry?;
            if let Ok(e) = postcard::from_bytes::<Entry>(bytes.value()) {
                out.insert(key.value().to_owned(), e);
            }
        }
        Ok(out)
    }

    pub fn get(&self, key: &str) -> anyhow::Result<Option<Entry>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(SUGGESTIONS)?;
        Ok(table.get(key)?.and_then(|b| postcard::from_bytes(b.value()).ok()))
    }

    /// Stores a photo's new suggestion, replacing the one before, with the call's record.
    pub fn put(&self, key: &str, entry: &Entry, record: &RunRecord) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            tx.open_table(SUGGESTIONS)?.insert(key, postcard::to_stdvec(entry)?.as_slice())?;
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

    /// Sets the status of the photos' suggestions; photos without one are skipped.
    pub fn set_status(&self, keys: &[&str], status: Status) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(SUGGESTIONS)?;
            for key in keys {
                let Some(mut entry) = table.get(*key)?.and_then(|b| postcard::from_bytes::<Entry>(b.value()).ok())
                else {
                    continue;
                };
                entry.status = status;
                table.insert(*key, postcard::to_stdvec(&entry)?.as_slice())?;
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
        }
    }

    #[test]
    fn suggestions_are_replaced_and_every_call_is_counted() {
        let dir = tempfile::tempdir().unwrap();
        let book = SuggestionBook::open(&dir.path().join("ai.redb")).unwrap();
        let record = RunRecord { cost: 0.002, ..RunRecord::default() };
        book.put("c1:1", &entry(Status::Proposed), &record).unwrap();
        book.put("c1:1", &entry(Status::Proposed), &record).unwrap();
        book.record("c1:2", &RunRecord { cost: 0.001, ..RunRecord::default() }).unwrap();
        assert_eq!(book.all().unwrap().len(), 1);
        let (calls, cost) = book.spent().unwrap();
        assert_eq!(calls, 3);
        assert!((cost - 0.005).abs() < 1e-12);

        book.set_status(&["c1:1", "c1:9"], Status::Rejected).unwrap();
        assert_eq!(book.get("c1:1").unwrap().unwrap().status, Status::Rejected);
    }
}
