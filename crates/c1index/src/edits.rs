//! What the user changed in the app and has not written to its source yet, and a log of
//! every change seen, from the app or from the source.
//!
//! Kept in its own file, apart from the store: the store is a copy of the sources and is
//! dropped and refilled when its format changes; these are the user's own edits. Values
//! are JSON so an older queue still reads after `Op` gains variants.

use std::path::Path;

use anyhow::Context;
use c1model::{Change, KeywordRemap, Op, PhotoId};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

/// `<photo key>\u{1f}<field>` → JSON [`Pending`].
const PENDING: TableDefinition<&str, &[u8]> = TableDefinition::new("pending");
/// sequence number → JSON [`Event`].
const EVENTS: TableDefinition<u64, &[u8]> = TableDefinition::new("events");
/// sequence number → JSON [`QueuedRemap`]: keyword tree changes, applied in order.
const REMAPS: TableDefinition<u64, &[u8]> = TableDefinition::new("remaps");

/// A keyword tree change waiting to be written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueuedRemap {
    pub remap: KeywordRemap,
    pub at: i64,
}

/// One field of one photo, edited in the app, waiting to be written to its source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    /// `base` is what the source held when the field was first edited; `value` is what
    /// the user wants now.
    pub change: Change,
    /// Milliseconds since the epoch, of the last edit.
    pub at: i64,
    /// Set when the source changed the same field to something else meanwhile: the op
    /// from `base` to the source's new value. The user picks one side before it is written.
    pub theirs: Option<Op>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    /// Edited in this app.
    App,
    /// Seen in the source by a sync (Capture One, or another program).
    Source,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub at: i64,
    pub origin: Origin,
    pub change: Change,
}

pub fn pending_key(photo: &PhotoId, field: &str) -> String {
    format!("{}\u{1f}{field}", photo.key())
}

/// The photo key a pending key belongs to.
pub fn photo_key(pending_key: &str) -> &str {
    pending_key.rsplit_once('\u{1f}').map_or(pending_key, |(photo, _)| photo)
}

pub struct Edits {
    db: Database,
}

pub struct EditsBatch {
    tx: redb::WriteTransaction,
}

impl Edits {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let db = Database::create(path).with_context(|| format!("opening edits {}", path.display()))?;
        let tx = db.begin_write()?;
        tx.open_table(PENDING)?;
        tx.open_table(EVENTS)?;
        tx.open_table(REMAPS)?;
        tx.commit()?;
        Ok(Self { db })
    }

    pub fn batch(&self) -> anyhow::Result<EditsBatch> {
        Ok(EditsBatch { tx: self.db.begin_write()? })
    }

    /// Every pending edit with its key, in key order (so a photo's fields sit together).
    pub fn pending(&self) -> anyhow::Result<Vec<(String, Pending)>> {
        let tx = self.db.begin_read()?;
        let mut out = Vec::new();
        for entry in tx.open_table(PENDING)?.iter()? {
            let (key, bytes) = entry?;
            out.push((key.value().to_owned(), serde_json::from_slice(bytes.value())?));
        }
        Ok(out)
    }

    /// Queued keyword tree changes, in the order they apply.
    pub fn remaps(&self) -> anyhow::Result<Vec<(u64, QueuedRemap)>> {
        let tx = self.db.begin_read()?;
        let mut out = Vec::new();
        for entry in tx.open_table(REMAPS)?.iter()? {
            let (seq, bytes) = entry?;
            out.push((seq.value(), serde_json::from_slice(bytes.value())?));
        }
        Ok(out)
    }

    /// The newest `limit` events, newest first.
    pub fn events(&self, limit: usize) -> anyhow::Result<Vec<Event>> {
        let tx = self.db.begin_read()?;
        let mut out = Vec::new();
        for entry in tx.open_table(EVENTS)?.iter()?.rev().take(limit) {
            let (_, bytes) = entry?;
            out.push(serde_json::from_slice(bytes.value())?);
        }
        Ok(out)
    }
}

impl EditsBatch {
    pub fn get(&self, key: &str) -> anyhow::Result<Option<Pending>> {
        let table = self.tx.open_table(PENDING)?;
        let Some(bytes) = table.get(key)? else { return Ok(None) };
        Ok(Some(serde_json::from_slice(bytes.value())?))
    }

    pub fn put(&mut self, key: &str, pending: &Pending) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(pending)?;
        self.tx.open_table(PENDING)?.insert(key, bytes.as_slice())?;
        Ok(())
    }

    pub fn remove(&mut self, key: &str) -> anyhow::Result<()> {
        self.tx.open_table(PENDING)?.remove(key)?;
        Ok(())
    }

    pub fn add_remap(&mut self, remap: &QueuedRemap) -> anyhow::Result<u64> {
        let mut table = self.tx.open_table(REMAPS)?;
        let next = table.last()?.map_or(0, |(k, _)| k.value() + 1);
        let bytes = serde_json::to_vec(remap)?;
        table.insert(next, bytes.as_slice())?;
        Ok(next)
    }

    pub fn get_remap(&self, seq: u64) -> anyhow::Result<Option<QueuedRemap>> {
        let table = self.tx.open_table(REMAPS)?;
        let Some(bytes) = table.get(seq)? else { return Ok(None) };
        Ok(Some(serde_json::from_slice(bytes.value())?))
    }

    pub fn remove_remap(&mut self, seq: u64) -> anyhow::Result<()> {
        self.tx.open_table(REMAPS)?.remove(seq)?;
        Ok(())
    }

    /// Every queued entry with its key (for rewriting keys after a remap).
    pub fn all(&self) -> anyhow::Result<Vec<(String, Pending)>> {
        let mut out = Vec::new();
        for entry in self.tx.open_table(PENDING)?.iter()? {
            let (key, bytes) = entry?;
            out.push((key.value().to_owned(), serde_json::from_slice(bytes.value())?));
        }
        Ok(out)
    }

    pub fn log(&mut self, event: &Event) -> anyhow::Result<()> {
        let mut table = self.tx.open_table(EVENTS)?;
        let next = table.last()?.map_or(0, |(k, _)| k.value() + 1);
        let bytes = serde_json::to_vec(event)?;
        table.insert(next, bytes.as_slice())?;
        Ok(())
    }

    pub fn commit(self) -> anyhow::Result<()> {
        self.tx.commit()?;
        Ok(())
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
