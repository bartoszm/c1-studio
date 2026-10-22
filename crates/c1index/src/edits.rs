//! What the user changed in the app and has not written to its source yet, and a log of
//! every change seen, from the app or from the source.
//!
//! Kept in its own file, apart from the store: the store is a copy of the sources and is
//! dropped and refilled when its format changes; these are the user's own edits. Values
//! are JSON so an older queue still reads after `Op` gains variants.

use std::path::Path;

use anyhow::Context;
use c1model::{Change, Collection, CollectionChange, CollectionId, KeywordRemap, Op, PhotoId};
use redb::{Database, MultimapTableDefinition, ReadableDatabase, ReadableTable, TableDefinition};
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
/// sequence number → JSON [`CollectionChange`]: album changes written to a source and
/// logged when they were queued, until a sync sees them come back.
const WRITTEN: TableDefinition<u64, &[u8]> = TableDefinition::new("written_collections");
/// collection id → its path when last seen, for wording entries about albums since
/// deleted, or made here under an id the source replaced.
const ALBUM_NAMES: TableDefinition<&str, &str> = TableDefinition::new("album_names");
/// [`BY_PHOTO_UPTO`] and such.
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");
/// Events from this sequence number on are not in [`BY_PHOTO`] yet: written by a build
/// from before it, or not looked at since.
const BY_PHOTO_UPTO: &str = "by_photo_upto";

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
    /// Put back by restoring a backup of the source from this app.
    Restore,
    /// Suggested by a vision model and accepted in this app.
    Ai,
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
    /// A photo deleted for good from the app's trash: the source deleted its files, or
    /// put it in its own trash.
    Deleted { photo: PhotoId, name: String },
    /// An album, smart album or group made, renamed, moved or deleted.
    Collection { change: CollectionChange, label: String },
    /// A keyword tree change (rename, join, group, ungroup).
    Remap(KeywordRemap),
    /// Queued edits written to the source: `changes` to `photos` photos (those the source
    /// held already not counted), album changes and keyword tree changes.
    Written {
        photos: usize,
        changes: usize,
        #[serde(default)]
        albums: usize,
        #[serde(default)]
        keywords: usize,
    },
    /// A queued album or keyword tree change taken back before it was written.
    Undone { label: String },
}

impl What {
    /// The photo this entry is about, if it is about one.
    pub fn photo(&self) -> Option<&PhotoId> {
        match self {
            What::Change(change) => Some(&change.photo),
            What::Added { photo, .. } | What::Removed { photo, .. } | What::Deleted { photo, .. } => Some(photo),
            _ => None,
        }
    }

    /// The album a photo went into or out of, if that is what this entry records.
    pub fn album(&self) -> Option<&CollectionId> {
        match self {
            What::Change(Change {
                op: Op::AddToCollection { collection } | Op::RemoveFromCollection { collection },
                ..
            }) => Some(collection),
            _ => None,
        }
    }
}

/// An event as stored, or `None` (said on stderr) for one this build cannot read:
/// written by a newer one. One unreadable entry does not take the log with it.
fn read_event(seq: u64, bytes: &[u8]) -> Option<Event> {
    serde_json::from_slice(bytes).inspect_err(|e| eprintln!("change log entry {seq} skipped: {e}")).ok()
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
        tx.open_table(WRITTEN)?;
        tx.open_table(ALBUM_NAMES)?;
        {
            // The per-photo lookup came after the log, and a build from before it may
            // still add to the log: what it has not seen is looked up now.
            let events = tx.open_table(EVENTS)?;
            let mut by_photo = tx.open_multimap_table(BY_PHOTO)?;
            let mut meta = tx.open_table(META)?;
            let from = meta.get(BY_PHOTO_UPTO)?.map_or(0, |v| v.value());
            let mut upto = from;
            for entry in events.range(from..)? {
                let (seq, bytes) = entry?;
                upto = seq.value() + 1;
                if let Some(event) = read_event(seq.value(), bytes.value())
                    && let Some(photo) = event.what.photo()
                {
                    by_photo.insert(photo.key().as_str(), seq.value())?;
                }
            }
            if upto != from {
                meta.insert(BY_PHOTO_UPTO, upto)?;
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
        for entry in tx.open_table(EVENTS)?.iter()?.rev() {
            if out.len() == limit {
                break;
            }
            let (seq, bytes) = entry?;
            out.extend(read_event(seq.value(), bytes.value()));
        }
        Ok(out)
    }

    /// The names last seen for these albums, by id; ids never seen are left out.
    pub fn album_names<'a>(
        &self,
        ids: impl IntoIterator<Item = &'a CollectionId>,
    ) -> anyhow::Result<Vec<(CollectionId, String)>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(ALBUM_NAMES)?;
        let mut out = Vec::new();
        for id in ids {
            if let Some(name) = table.get(id.0.as_str())? {
                out.push((id.clone(), name.value().to_owned()));
            }
        }
        Ok(out)
    }

    /// Every event about one photo (by its key), newest first.
    pub fn photo_events(&self, key: &str) -> anyhow::Result<Vec<Event>> {
        let tx = self.db.begin_read()?;
        let events = tx.open_table(EVENTS)?;
        let mut out = Vec::new();
        for seq in tx.open_multimap_table(BY_PHOTO)?.get(key)?.rev() {
            let seq = seq?.value();
            if let Some(bytes) = events.get(seq)? {
                out.extend(read_event(seq, bytes.value()));
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
        self.tx.open_table(META)?.insert(BY_PHOTO_UPTO, next + 1)?;
        Ok(())
    }

    /// An album change just written to its source, to know it again when it comes back.
    pub fn add_written(&mut self, change: &CollectionChange) -> anyhow::Result<()> {
        let mut table = self.tx.open_table(WRITTEN)?;
        let next = table.last()?.map_or(0, |(k, _)| k.value() + 1);
        let bytes = serde_json::to_vec(change)?;
        table.insert(next, bytes.as_slice())?;
        Ok(())
    }

    /// Album changes written to a source and not seen back yet, in order.
    pub fn written(&self) -> anyhow::Result<Vec<CollectionChange>> {
        let mut out = Vec::new();
        for entry in self.tx.open_table(WRITTEN)?.iter()? {
            out.push(serde_json::from_slice(entry?.1.value())?);
        }
        Ok(out)
    }

    pub fn clear_written(&mut self) -> anyhow::Result<()> {
        self.tx.open_table(WRITTEN)?.retain(|_, _| false)?;
        Ok(())
    }

    /// Remembers what these albums are called now.
    pub fn name_albums(&mut self, collections: &[Collection]) -> anyhow::Result<()> {
        let mut table = self.tx.open_table(ALBUM_NAMES)?;
        for c in collections {
            let name = if c.path.is_empty() { &c.name } else { &c.path };
            if table.get(c.id.0.as_str())?.is_none_or(|v| v.value() != name) {
                table.insert(c.id.0.as_str(), name.as_str())?;
            }
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

        let written = r#"{"at":5,"origin":"App","written":{"photos":2,"changes":3}}"#;
        let event: Event = serde_json::from_str(written).unwrap();
        assert_eq!(event.what, What::Written { photos: 2, changes: 3, albums: 0, keywords: 0 });
    }

    #[test]
    fn a_queued_position_reads_back_exactly() {
        use c1model::{GeoOrigin, GeoPoint};
        let dir = tempfile::tempdir().unwrap();
        let edits = Edits::open(&dir.path().join("edits.redb")).unwrap();
        // A camera's `19,56.5857667E`: the default float parsing reads it one step off,
        // and the edit then no longer matches the catalog it was made against.
        let camera = GeoPoint {
            lat: 50.0 + 3.0607333 / 60.0,
            lon: 19.0 + 56.5857667 / 60.0,
            alt: None,
            origin: GeoOrigin::Exif,
        };
        let moved = GeoPoint { lat: 50.050445, lon: 19.943949, alt: None, origin: GeoOrigin::Manual };
        let photo = PhotoId::new(&c1model::SourceId("c1".into()), "7".to_string());
        let pending = Pending {
            change: Change { photo: photo.clone(), op: Op::SetGeo { base: Some(camera), value: Some(moved) } },
            at: 5,
            theirs: None,
        };
        let mut batch = edits.batch().unwrap();
        batch.put(&pending_key(&photo, "geo"), &pending).unwrap();
        batch.commit().unwrap();
        assert_eq!(edits.pending().unwrap(), [(pending_key(&photo, "geo"), pending)]);
    }

    #[test]
    fn entries_another_build_wrote_are_looked_up_or_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edits.redb");
        let photo = PhotoId::new(&c1model::SourceId("c1".into()), "7".to_string());
        let rated = |value| Event {
            at: 5,
            origin: Origin::Source,
            what: What::Change(Change { photo: photo.clone(), op: Op::SetRating { base: None, value } }),
        };
        {
            let edits = Edits::open(&path).unwrap();
            let mut batch = edits.batch().unwrap();
            batch.log(&rated(Some(3))).unwrap();
            batch.commit().unwrap();
        }
        {
            // A build from before the per-photo lookup adds to the log, and one from
            // after this one adds a kind this one does not know.
            let db = Database::create(&path).unwrap();
            let tx = db.begin_write().unwrap();
            {
                let mut events = tx.open_table(EVENTS).unwrap();
                events.insert(1, serde_json::to_vec(&rated(Some(4))).unwrap().as_slice()).unwrap();
                events.insert(2, br#"{"at":6,"origin":"App","teleported":{}}"#.as_slice()).unwrap();
            }
            tx.commit().unwrap();
        }
        let edits = Edits::open(&path).unwrap();
        let ratings: Vec<Event> = edits.photo_events(&photo.key()).unwrap();
        assert_eq!(ratings, [rated(Some(4)), rated(Some(3))]);
        assert_eq!(edits.events(10).unwrap().len(), 2);
    }
}
