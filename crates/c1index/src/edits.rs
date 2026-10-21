//! What the user changed in the app and has not written to its source yet, and a log of
//! every change seen, from the app or from the source.
//!
//! Kept in its own file, apart from the store: the store is a copy of the sources and is
//! dropped and refilled when its format changes; these are the user's own edits. Values
//! are JSON so an older queue still reads after `Op` gains variants.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use c1model::{Change, Collection, CollectionChange, CollectionId, KeywordPath, KeywordRemap, Op, Photo, PhotoId};
use redb::{Database, MultimapTableDefinition, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

/// `<photo key>\u{1f}<field>` → JSON [`Pending`].
const PENDING: TableDefinition<&str, &[u8]> = TableDefinition::new("pending");
/// sequence number → JSON [`Event`].
const EVENTS: TableDefinition<u64, &[u8]> = TableDefinition::new("events");
/// sequence number → JSON [`Before`]: what the entry replaced, apart from the entry so
/// that reading the log does not read it, and a payload this build cannot read costs
/// only itself.
const BEFORES: TableDefinition<u64, &[u8]> = TableDefinition::new("event_before");
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
/// 1 when the user turned History off for this catalog: entries then keep nothing but
/// the change. Kept here, with the log, so every program that opens the library follows it.
const HISTORY_OFF: &str = "history_off";
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
}

/// One entry of the change log: when, from where, and what.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub at: i64,
    pub origin: Origin,
    #[serde(flatten)]
    pub what: What,
    /// Whether the entry kept what the change overwrote or removed (in `event_before`).
    /// Entries written before it existed say nothing, and are written back as they were.
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_before: bool,
    /// What the change overwrote or removed, for the kinds that cannot be taken back from
    /// the entry alone. Not part of the entry's JSON: it is stored apart, and read only by
    /// the reads that ask for it.
    #[serde(skip)]
    pub before: Option<Before>,
    /// What the first builds of the History tab kept in the entry itself, before it moved
    /// to `event_before`: read in its place (leniently, like the table's), never written.
    #[serde(default, rename = "before", skip_serializing)]
    inline_before: Option<serde_json::Value>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Puts what an entry kept into it: the copy in `event_before`, or the one an earlier
/// build wrote into the entry. A copy this build cannot read is left out (said on stderr).
fn fill_before(event: &mut Event, seq: u64, stored: Option<&[u8]>) {
    let inline = event.inline_before.take();
    event.before = match stored {
        Some(bytes) => read_before(seq, Some(bytes)),
        None => inline.and_then(|value| {
            serde_json::from_value(value)
                .inspect_err(|e| eprintln!("change log entry {seq}: what it replaced skipped: {e}"))
                .ok()
        }),
    };
}

/// What a change replaced, kept so it can be put back later.
// A photo is far bigger than the rest, but it is only held while an entry is written or read.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Before {
    /// The photo as it was, for a removed or deleted one.
    Photo(Box<Photo>),
    /// The albums a change replaced (as they were) and, for a delete, who was in them.
    Albums { albums: Vec<Collection>, members: Vec<(CollectionId, Vec<PhotoId>)> },
    /// The keywords each photo a keyword tree change rewrote had before it.
    Keywords(Vec<(PhotoId, Vec<KeywordPath>)>),
}

impl Event {
    pub fn new(at: i64, origin: Origin, what: What) -> Self {
        Self { at, origin, what, has_before: false, before: None, inline_before: None }
    }

    /// The same entry, with what it replaced.
    pub fn with_before(mut self, before: Before) -> Self {
        self.has_before = true;
        self.before = Some(before);
        self
    }

    /// Every photo this entry is about: the one it names, and those it kept in `before`.
    pub fn photos(&self) -> Vec<&PhotoId> {
        let mut out: Vec<&PhotoId> = self.what.photo().into_iter().collect();
        match &self.before {
            Some(Before::Photo(photo)) => out.push(&photo.id),
            Some(Before::Albums { members, .. }) => out.extend(members.iter().flat_map(|(_, ids)| ids)),
            Some(Before::Keywords(photos)) => out.extend(photos.iter().map(|(id, _)| id)),
            None => {}
        }
        out
    }
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
pub(crate) fn read_event(seq: u64, bytes: &[u8]) -> Option<Event> {
    let mut event: Event =
        serde_json::from_slice(bytes).inspect_err(|e| eprintln!("change log entry {seq} skipped: {e}")).ok()?;
    event.has_before |= event.inline_before.is_some();
    Some(event)
}

/// What an entry kept, or `None` (said on stderr) when it is missing or this build cannot
/// read it: the entry itself still reads.
fn read_before(seq: u64, bytes: Option<&[u8]>) -> Option<Before> {
    serde_json::from_slice(bytes?)
        .inspect_err(|e| eprintln!("change log entry {seq}: what it replaced skipped: {e}"))
        .ok()
}

/// When an entry was made, without reading the rest of it.
fn read_at(seq: u64, bytes: &[u8]) -> Option<i64> {
    #[derive(Deserialize)]
    struct At {
        at: i64,
    }
    serde_json::from_slice::<At>(bytes)
        .inspect_err(|e| eprintln!("change log entry {seq} skipped: {e}"))
        .ok()
        .map(|a| a.at)
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
    /// Entries keep what they replaced (for the History tab). Off, they are logged as 0.5
    /// logged them.
    keep_before: AtomicBool,
}

pub struct EditsBatch {
    tx: redb::WriteTransaction,
    keep_before: bool,
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
        tx.open_table(BEFORES)?;
        let keep_before;
        {
            // The per-photo lookup came after the log, and a build from before it may
            // still add to the log: what it has not seen is looked up now.
            let events = tx.open_table(EVENTS)?;
            let mut by_photo = tx.open_multimap_table(BY_PHOTO)?;
            let mut meta = tx.open_table(META)?;
            keep_before = meta.get(HISTORY_OFF)?.is_none_or(|v| v.value() == 0);
            let from = meta.get(BY_PHOTO_UPTO)?.map_or(0, |v| v.value());
            let mut upto = from;
            for entry in events.range(from..)? {
                let (seq, bytes) = entry?;
                upto = seq.value() + 1;
                if let Some(event) = read_event(seq.value(), bytes.value()) {
                    for photo in event.photos() {
                        by_photo.insert(photo.key().as_str(), seq.value())?;
                    }
                }
            }
            if upto != from {
                meta.insert(BY_PHOTO_UPTO, upto)?;
            }
        }
        tx.commit()?;
        Ok(Self { db, keep_before: AtomicBool::new(keep_before) })
    }

    pub fn batch(&self) -> anyhow::Result<EditsBatch> {
        Ok(EditsBatch { tx: self.db.begin_write()?, keep_before: self.keeps_before() })
    }

    /// Whether entries logged from now on keep what they replaced. Entries already
    /// logged are left as they are either way. Kept with the log, so it holds for every
    /// program that opens it, and takes effect only once it is stored.
    pub fn set_keep_before(&self, on: bool) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        tx.open_table(META)?.insert(HISTORY_OFF, u64::from(!on))?;
        tx.commit()?;
        self.keep_before.store(on, Ordering::Relaxed);
        Ok(())
    }

    pub fn keeps_before(&self) -> bool {
        self.keep_before.load(Ordering::Relaxed)
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
        let befores = tx.open_table(BEFORES)?;
        let mut out = Vec::new();
        for entry in tx.open_table(EVENTS)?.iter()?.rev() {
            if out.len() == limit {
                break;
            }
            let (seq, bytes) = entry?;
            if let Some(mut event) = read_event(seq.value(), bytes.value()) {
                if event.has_before {
                    fill_before(&mut event, seq.value(), befores.get(seq.value())?.as_ref().map(|b| b.value()));
                }
                out.push(event);
            }
        }
        Ok(out)
    }

    /// The sequence number of the newest entry, or `None` for an empty log.
    pub fn last_seq(&self) -> anyhow::Result<Option<u64>> {
        let tx = self.db.begin_read()?;
        Ok(tx.open_table(EVENTS)?.last()?.map(|(k, _)| k.value()))
    }

    /// The sequence number of the first entry made after `ms` (`at > ms`), or one past
    /// the newest when there is none. Sequence numbers have no gaps and `at` never goes
    /// down along them, so a search over them finds it without reading the whole log.
    pub fn seq_after(&self, ms: i64) -> anyhow::Result<u64> {
        let tx = self.db.begin_read()?;
        let events = tx.open_table(EVENTS)?;
        let (Some((first, _)), Some((last, _))) = (events.first()?, events.last()?) else { return Ok(0) };
        let (mut lo, mut hi) = (first.value(), last.value() + 1);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            // An entry that cannot be read does not tell when it was made: it counts as
            // before, which keeps the search going to the right.
            let at = events.get(mid)?.and_then(|bytes| read_at(mid, bytes.value()));
            if at.is_some_and(|at| at > ms) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        Ok(lo)
    }

    /// The entries with these sequence numbers, oldest first. Unreadable ones are left out.
    /// What an entry kept is read only for those `with_before` says yes to: it can be a
    /// whole photo, and most reads of the log have no use for it.
    pub fn range(
        &self,
        seqs: std::ops::Range<u64>,
        with_before: &dyn Fn(&Event) -> bool,
    ) -> anyhow::Result<Vec<(u64, Event)>> {
        let tx = self.db.begin_read()?;
        let befores = tx.open_table(BEFORES)?;
        let mut out = Vec::new();
        for entry in tx.open_table(EVENTS)?.range(seqs)? {
            let (seq, bytes) = entry?;
            let seq = seq.value();
            let Some(mut event) = read_event(seq, bytes.value()) else { continue };
            if event.has_before && with_before(&event) {
                fill_before(&mut event, seq, befores.get(seq)?.as_ref().map(|b| b.value()));
            }
            event.inline_before = None;
            out.push((seq, event));
        }
        Ok(out)
    }

    /// When the newest entry was made.
    pub fn last_at(&self) -> anyhow::Result<Option<i64>> {
        let tx = self.db.begin_read()?;
        let events = tx.open_table(EVENTS)?;
        let Some((seq, bytes)) = events.last()? else { return Ok(None) };
        Ok(read_at(seq.value(), bytes.value()))
    }

    /// When the first entry was made.
    pub fn first_at(&self) -> anyhow::Result<Option<i64>> {
        let tx = self.db.begin_read()?;
        let events = tx.open_table(EVENTS)?;
        let Some((seq, bytes)) = events.first()? else { return Ok(None) };
        Ok(read_at(seq.value(), bytes.value()))
    }

    /// When each entry from `seq` on was made, with its number, for counting by day.
    pub fn times_from(&self, seq: u64, each: &mut dyn FnMut(u64, i64)) -> anyhow::Result<()> {
        let tx = self.db.begin_read()?;
        for entry in tx.open_table(EVENTS)?.range(seq..)? {
            let (seq, bytes) = entry?;
            if let Some(at) = read_at(seq.value(), bytes.value()) {
                each(seq.value(), at);
            }
        }
        Ok(())
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
        let befores = tx.open_table(BEFORES)?;
        let mut out = Vec::new();
        for seq in tx.open_multimap_table(BY_PHOTO)?.get(key)?.rev() {
            let seq = seq?.value();
            if let Some(bytes) = events.get(seq)?
                && let Some(mut event) = read_event(seq, bytes.value())
            {
                if event.has_before {
                    fill_before(&mut event, seq, befores.get(seq)?.as_ref().map(|b| b.value()));
                }
                out.push(event);
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
        let (next, newest) = match table.last()? {
            Some((k, bytes)) => (k.value() + 1, read_at(k.value(), bytes.value())),
            None => (0, None),
        };
        // `at` never goes down along the log, which a search by time relies on: a clock set
        // back (or a sync that took its time before logging) does not break it.
        let mut event = event.clone();
        if let Some(newest) = newest {
            event.at = event.at.max(newest);
        }
        // With History off, the entry is what 0.5 wrote.
        if !self.keep_before {
            event.before = None;
            event.has_before = false;
        }
        let bytes = serde_json::to_vec(&event)?;
        table.insert(next, bytes.as_slice())?;
        if let Some(before) = &event.before {
            self.tx.open_table(BEFORES)?.insert(next, serde_json::to_vec(before)?.as_slice())?;
        }
        let mut by_photo = self.tx.open_multimap_table(BY_PHOTO)?;
        for photo in event.photos() {
            by_photo.insert(photo.key().as_str(), next)?;
        }
        drop(by_photo);
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
        let rated = |value| {
            let change = Change { photo: photo.clone(), op: Op::SetRating { base: None, value } };
            Event::new(5, Origin::Source, What::Change(change))
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

    fn written_at(at: i64) -> Event {
        Event::new(at, Origin::App, What::Undone { label: format!("at {at}") })
    }

    fn log_of(times: &[i64]) -> (tempfile::TempDir, Edits) {
        let dir = tempfile::tempdir().unwrap();
        let edits = Edits::open(&dir.path().join("edits.redb")).unwrap();
        let mut batch = edits.batch().unwrap();
        for &at in times {
            batch.log(&written_at(at)).unwrap();
        }
        batch.commit().unwrap();
        (dir, edits)
    }

    #[test]
    fn the_first_entry_after_a_time_is_found_by_search() {
        let (_d, edits) = log_of(&[10, 20, 20, 20, 30]);
        assert_eq!(edits.last_seq().unwrap(), Some(4));
        // Equal times: an entry at the time itself is not after it.
        assert_eq!(edits.seq_after(20).unwrap(), 4);
        assert_eq!(edits.seq_after(19).unwrap(), 1);
        assert_eq!(edits.seq_after(10).unwrap(), 1);
        // Beyond either end.
        assert_eq!(edits.seq_after(i64::MIN).unwrap(), 0);
        assert_eq!(edits.seq_after(9).unwrap(), 0);
        assert_eq!(edits.seq_after(30).unwrap(), 5);
        assert_eq!(edits.seq_after(i64::MAX).unwrap(), 5);
    }

    #[test]
    fn an_empty_log_has_nothing_after_any_time() {
        let (_d, edits) = log_of(&[]);
        assert_eq!(edits.last_seq().unwrap(), None);
        assert_eq!(edits.seq_after(0).unwrap(), 0);
        assert!(edits.range(0..10, &|_| false).unwrap().is_empty());
    }

    #[test]
    fn a_range_skips_entries_it_cannot_read() {
        let (dir, edits) = log_of(&[1, 2, 3]);
        drop(edits);
        {
            let db = Database::create(dir.path().join("edits.redb")).unwrap();
            let tx = db.begin_write().unwrap();
            tx.open_table(EVENTS).unwrap().insert(1, br#"{"at":2,"origin":"App","teleported":{}}"#.as_slice()).unwrap();
            tx.commit().unwrap();
        }
        let edits = Edits::open(&dir.path().join("edits.redb")).unwrap();
        let seqs: Vec<u64> = edits.range(0..3, &|_| false).unwrap().into_iter().map(|(seq, _)| seq).collect();
        assert_eq!(seqs, [0, 2]);
        assert_eq!(edits.range(1..3, &|_| false).unwrap().len(), 1);
        // Searching passes it too: it still says when it was made.
        assert_eq!(edits.seq_after(2).unwrap(), 2);
    }

    /// What 0.5 read: an entry without `before`.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Event05 {
        at: i64,
        origin: Origin,
        #[serde(flatten)]
        what: What,
    }

    #[test]
    fn entries_with_what_they_replaced_still_read_in_older_builds() {
        let photo = PhotoId::new(&c1model::SourceId("c1".into()), "7".to_string());
        let removed = What::Removed { photo: photo.clone(), name: "a.raf".into() };
        let kept = Before::Keywords(vec![(photo.clone(), vec![KeywordPath::from_pipe("a|b")])]);
        let new = Event::new(5, Origin::App, removed.clone()).with_before(kept.clone());
        let json = serde_json::to_string(&new).unwrap();
        // What it replaced is stored apart: the entry says only that it has some.
        assert!(json.contains("\"has_before\":true") && !json.contains("a|b"), "{json}");
        assert!(serde_json::from_str::<Event>(&json).unwrap().has_before);
        // The build before reads it as the entry it was.
        let old: Event05 = serde_json::from_str(&json).unwrap();
        assert_eq!(old, Event05 { at: 5, origin: Origin::App, what: removed });
        // And an old entry has no `before`, and is written back without one, byte for byte.
        let text = r#"{"at":5,"origin":"App","undone":{"label":"x"}}"#;
        let event: Event = serde_json::from_str(text).unwrap();
        assert_eq!(event.before, None);
        assert_eq!(serde_json::to_string(&event).unwrap(), text);
    }

    #[test]
    fn what_a_log_entry_kept_is_in_its_photos_history() {
        let dir = tempfile::tempdir().unwrap();
        let edits = Edits::open(&dir.path().join("edits.redb")).unwrap();
        let (a, b) = (
            PhotoId::new(&c1model::SourceId("c1".into()), "1".to_string()),
            PhotoId::new(&c1model::SourceId("c1".into()), "2".to_string()),
        );
        let remap = KeywordRemap::rename(KeywordPath::from_pipe("x"), "y");
        let event = Event::new(1, Origin::App, What::Remap(remap))
            .with_before(Before::Keywords(vec![(a.clone(), vec![KeywordPath::from_pipe("x")]), (b.clone(), vec![])]));
        let mut batch = edits.batch().unwrap();
        batch.log(&event).unwrap();
        batch.commit().unwrap();
        assert_eq!(edits.photo_events(&a.key()).unwrap(), std::slice::from_ref(&event));
        assert_eq!(edits.photo_events(&b.key()).unwrap(), [event]);
    }

    #[test]
    fn what_an_entry_kept_is_read_when_asked_and_a_bad_copy_costs_only_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edits.redb");
        let photo = PhotoId::new(&c1model::SourceId("c1".into()), "1".to_string());
        let kept = Before::Keywords(vec![(photo.clone(), vec![KeywordPath::from_pipe("x")])]);
        let remap = || What::Remap(KeywordRemap::rename(KeywordPath::from_pipe("x"), "y"));
        {
            let edits = Edits::open(&path).unwrap();
            let mut batch = edits.batch().unwrap();
            batch.log(&Event::new(1, Origin::App, remap()).with_before(kept.clone())).unwrap();
            batch.log(&Event::new(2, Origin::App, remap()).with_before(kept.clone())).unwrap();
            batch.commit().unwrap();
            // Only the entries the read asks for have it.
            let read = edits.range(0..2, &|e| e.at == 1).unwrap();
            assert_eq!((read[0].1.before.clone(), read[1].1.before.clone()), (Some(kept.clone()), None));
            assert!(read[1].1.has_before);
        }
        {
            // A later build changed what a photo looks like: this copy no longer reads.
            let db = Database::create(&path).unwrap();
            let tx = db.begin_write().unwrap();
            tx.open_table(BEFORES).unwrap().insert(1, br#"{"Photo":{"future":true}}"#.as_slice()).unwrap();
            tx.commit().unwrap();
        }
        let edits = Edits::open(&path).unwrap();
        let events = edits.events(10).unwrap();
        assert_eq!(events.len(), 2, "the entry reads without it");
        assert_eq!((events[0].before.clone(), events[1].before.clone()), (None, Some(kept)));
    }

    #[test]
    fn times_never_go_down_along_the_log() {
        // A clock set back, or a sync that logs with the time it began.
        let (_d, edits) = log_of(&[10, 30, 20, 40]);
        let read = edits.range(0..4, &|_| false).unwrap();
        assert_eq!(read.iter().map(|(_, e)| e.at).collect::<Vec<_>>(), [10, 30, 30, 40]);
        assert_eq!(edits.seq_after(30).unwrap(), 3);
        assert_eq!((edits.first_at().unwrap(), edits.last_at().unwrap()), (Some(10), Some(40)));
    }

    fn a_photo(n: u32) -> Photo {
        use c1model::{AssetId, EditInfo, Exif, FileInfo, Iptc, Place};
        Photo {
            id: PhotoId::new(&c1model::SourceId("c1".into()), n.to_string()),
            asset: AssetId(n.to_string()),
            variant_index: 0,
            file: FileInfo { name: format!("{n}.raf"), ext: "raf".into(), path: None, size: None, offline: false },
            captured_at: None,
            imported_at: None,
            exif: Exif::default(),
            rating: Some(3),
            color: None,
            keywords: vec![],
            iptc: Iptc::default(),
            place: Place::default(),
            geo: None,
            collections: vec![],
            edit: EditInfo::default(),
            trashed: false,
        }
    }

    fn removed(n: u32) -> Event {
        let photo = a_photo(n);
        Event::new(5, Origin::Source, What::Removed { photo: photo.id.clone(), name: photo.file.name.clone() })
            .with_before(Before::Photo(Box::new(photo)))
    }

    /// Writes raw entries into the log, as another build would.
    fn put_raw(path: &Path, entries: &[(u64, &str)]) {
        let db = Database::create(path).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut events = tx.open_table(EVENTS).unwrap();
            for (seq, json) in entries {
                events.insert(*seq, json.as_bytes()).unwrap();
            }
        }
        tx.commit().unwrap();
    }

    #[test]
    fn what_the_first_history_builds_kept_in_the_entry_still_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edits.redb");
        drop(Edits::open(&path).unwrap());
        // The first builds of the History tab wrote `before` into the entry itself.
        let photo = serde_json::to_string(&Before::Photo(Box::new(a_photo(7)))).unwrap();
        let whole = format!(
            r#"{{"at":5,"origin":"Source","removed":{{"photo":{{"source":"c1","ext":"7"}},"name":"7.raf"}},"before":{photo}}}"#
        );
        // And one of a shape this build cannot read: the entry still does.
        let odd = r#"{"at":6,"origin":"Source","removed":{"photo":{"source":"c1","ext":"8"},"name":"8.raf"},"before":{"Photo":{"future":true}}}"#;
        put_raw(&path, &[(0, &whole), (1, odd)]);
        let edits = Edits::open(&path).unwrap();
        let events = edits.events(10).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.has_before && matches!(e.what, What::Removed { .. })));
        assert_eq!(
            (events[1].before.clone(), events[0].before.clone()),
            (Some(Before::Photo(Box::new(a_photo(7)))), None)
        );
        // Reads that do not ask for it do not carry it.
        let page = edits.range(0..2, &|_| false).unwrap();
        assert!(page.iter().all(|(_, e)| e.before.is_none() && e.has_before));
        assert_eq!(page[0].1, Event { has_before: true, ..Event::new(5, Origin::Source, events[1].what.clone()) });
    }

    #[test]
    fn with_history_off_entries_are_what_0_5_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let edits = Edits::open(&dir.path().join("edits.redb")).unwrap();
        edits.set_keep_before(false).unwrap();
        let mut batch = edits.batch().unwrap();
        batch.log(&removed(7)).unwrap();
        batch.commit().unwrap();
        let tx = edits.db.begin_read().unwrap();
        let bytes = tx.open_table(EVENTS).unwrap().get(0).unwrap().unwrap().value().to_vec();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            r#"{"at":5,"origin":"Source","removed":{"photo":{"source":"c1","ext":"7"},"name":"7.raf"}}"#
        );
        assert!(tx.open_table(BEFORES).unwrap().first().unwrap().is_none());
        drop(tx);
        // Still in the photo's history.
        let id = PhotoId::new(&c1model::SourceId("c1".into()), "7".to_string());
        assert_eq!(edits.photo_events(&id.key()).unwrap().len(), 1);

        // Back on, entries keep it again; the one logged while off stays as it was.
        edits.set_keep_before(true).unwrap();
        let mut batch = edits.batch().unwrap();
        batch.log(&removed(8)).unwrap();
        batch.commit().unwrap();
        let both = edits.events(2).unwrap();
        assert_eq!((both[0].before.is_some(), both[1].before.is_some()), (true, false));
    }

    #[test]
    fn turning_history_off_is_kept_with_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edits.redb");
        assert!(Edits::open(&path).unwrap().keeps_before());
        Edits::open(&path).unwrap().set_keep_before(false).unwrap();
        // Another program opening the same log follows it.
        let again = Edits::open(&path).unwrap();
        assert!(!again.keeps_before());
        again.set_keep_before(true).unwrap();
        drop(again);
        assert!(Edits::open(&path).unwrap().keeps_before());
    }
}
