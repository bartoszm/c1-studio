//! What the user changed in the app and has not written to its source yet, and a log of
//! every change seen, from the app or from the source.
//!
//! Kept in its own file, apart from the store: the store is a copy of the sources and is
//! dropped and refilled when its format changes; these are the user's own edits. Values
//! are JSON so an older queue still reads after `Op` gains variants.

use std::path::Path;

use anyhow::Context;
use c1model::{Change, CollectionChange, KeywordRemap, Op, PhotoId};
use redb::{
    Database, MultimapTableDefinition, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition,
};
use serde::{Deserialize, Serialize};

/// `<photo key>\u{1f}<field>` → JSON [`Pending`].
const PENDING: TableDefinition<&str, &[u8]> = TableDefinition::new("pending");
/// sequence number → JSON [`Event`].
const EVENTS: TableDefinition<u64, &[u8]> = TableDefinition::new("events");
/// photo key → sequence numbers of the events about that photo.
const BY_PHOTO: MultimapTableDefinition<&str, u64> = MultimapTableDefinition::new("events_by_photo");
/// sequence number → JSON [`QueuedRemap`]: keyword tree changes, applied in order.
const REMAPS: TableDefinition<u64, &[u8]> = TableDefinition::new("remaps");
/// sequence number → JSON [`QueuedCollection`]: album changes, applied in order.
const COLLECTIONS: TableDefinition<u64, &[u8]> = TableDefinition::new("collections");

/// A keyword tree change waiting to be written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueuedRemap {
    pub remap: KeywordRemap,
    pub at: i64,
}

/// An album change waiting to be written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueuedCollection {
    pub change: CollectionChange,
    /// What it does, worded when it was made (a later rename does not reword it).
    pub label: String,
    pub at: i64,
    /// Queued photo edits it made pointless (photos put into an album it deletes), kept
    /// so undoing it brings them back.
    #[serde(default)]
    pub dropped: Vec<(String, Pending)>,
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
    /// Placed by a GPX track in this app.
    Gpx,
}

/// One entry of the change log: when, from where, and what.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub at: i64,
    pub origin: Origin,
    #[serde(flatten)]
    pub what: What,
}

/// What an [`Event`] records. Entries written before the other kinds existed hold only a
/// `change`, and still read as [`What::Change`].
// Most entries are changes, and they are only passed through on their way to JSON.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum What {
    /// One field of one photo.
    Change(Change),
    /// A photo the source did not have before.
    Added { photo: PhotoId, name: String },
    /// A photo the source no longer has; `name` keeps the entry readable.
    Removed { photo: PhotoId, name: String },
    /// An album, smart album or group made, renamed, moved or deleted.
    Collection { change: CollectionChange, label: String },
    /// A keyword tree change (rename, join, group, ungroup).
    Remap(KeywordRemap),
    /// Queued edits written to the source.
    Written { photos: usize, changes: usize },
    /// A queued album or keyword tree change taken back before it was written.
    Undone { label: String },
}

impl What {
    /// The photo this entry is about, if it is about one.
    pub fn photo(&self) -> Option<&PhotoId> {
        match self {
            What::Change(change) => Some(&change.photo),
            What::Added { photo, .. } | What::Removed { photo, .. } => Some(photo),
            _ => None,
        }
    }
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
        tx.open_table(REMAPS)?;
        tx.open_table(COLLECTIONS)?;
        {
            // The per-photo lookup came after the log: fill it once from what is there.
            let events = tx.open_table(EVENTS)?;
            let mut by_photo = tx.open_multimap_table(BY_PHOTO)?;
            if by_photo.is_empty()? && !events.is_empty()? {
                for entry in events.iter()? {
                    let (seq, bytes) = entry?;
                    let event: Event = serde_json::from_slice(bytes.value())?;
                    if let Some(photo) = event.what.photo() {
                        by_photo.insert(photo.key().as_str(), seq.value())?;
                    }
                }
            }
        }
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

    /// Queued album changes, in the order they apply.
    pub fn collections(&self) -> anyhow::Result<Vec<(u64, QueuedCollection)>> {
        let tx = self.db.begin_read()?;
        let mut out = Vec::new();
        for entry in tx.open_table(COLLECTIONS)?.iter()? {
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

    /// Every event about one photo (by its key), newest first.
    pub fn photo_events(&self, key: &str) -> anyhow::Result<Vec<Event>> {
        let tx = self.db.begin_read()?;
        let events = tx.open_table(EVENTS)?;
        let mut out = Vec::new();
        for seq in tx.open_multimap_table(BY_PHOTO)?.get(key)?.rev() {
            if let Some(bytes) = events.get(seq?.value())? {
                out.push(serde_json::from_slice(bytes.value())?);
            }
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

    pub fn add_collection(&mut self, queued: &QueuedCollection) -> anyhow::Result<u64> {
        let mut table = self.tx.open_table(COLLECTIONS)?;
        let next = table.last()?.map_or(0, |(k, _)| k.value() + 1);
        let bytes = serde_json::to_vec(queued)?;
        table.insert(next, bytes.as_slice())?;
        Ok(next)
    }

    /// Queued album changes, in order.
    pub fn collections(&self) -> anyhow::Result<Vec<(u64, QueuedCollection)>> {
        let mut out = Vec::new();
        for entry in self.tx.open_table(COLLECTIONS)?.iter()? {
            let (seq, bytes) = entry?;
            out.push((seq.value(), serde_json::from_slice(bytes.value())?));
        }
        Ok(out)
    }

    pub fn remove_collection(&mut self, seq: u64) -> anyhow::Result<()> {
        self.tx.open_table(COLLECTIONS)?.remove(seq)?;
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
        if let Some(photo) = event.what.photo() {
            self.tx.open_multimap_table(BY_PHOTO)?.insert(photo.key().as_str(), next)?;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_written_before_kinds_still_read() {
        let old = r#"{"at":5,"origin":"Source","change":{"photo":{"source":"c1","ext":"7"},"op":{"SetRating":{"base":null,"value":3}}}}"#;
        let event: Event = serde_json::from_str(old).unwrap();
        let What::Change(change) = &event.what else { panic!("{event:?}") };
        assert_eq!(change.op, Op::SetRating { base: None, value: Some(3) });
        // And they are still written that way.
        assert_eq!(serde_json::to_string(&event).unwrap(), old);
    }
}
