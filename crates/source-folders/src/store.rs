//! The provider's own store (`folders.redb` in its `work_dir`): every file it found and the
//! id it gave it, what the file says about itself, the user's edits on top, albums, and the
//! images it made.
//!
//! The whole store is held in memory ([`State`]) and written through to redb: a library of
//! 100k photos is a few tens of megabytes. Records are JSON, so a later build reads an
//! earlier one's store: the ids in it are the truth the app's library, edits and trips hang
//! on, and cannot be made again.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{RwLock, RwLockReadGuard};

use c1model::{Collection, CollectionId, ImageTier, Result, SourceError};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::ids::ContentKey;
use crate::layer::UserLayer;
use crate::meta::{FileKind, FileMeta};

/// Bumped when a record changes shape in a way JSON defaults do not cover.
pub const STORE_VERSION: u32 = 1;

/// name → JSON value: `uuid`, `generation`, `made`, `albums`, `albums_gen`, `version`.
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");
/// photo id (`PhotoId::ext`) → JSON [`FileRec`].
const FILES: TableDefinition<&str, &str> = TableDefinition::new("files");
/// photo id → JSON [`UserLayer`].
const USER: TableDefinition<&str, &str> = TableDefinition::new("user");
/// photo id → generation it last changed in.
const CHANGED: TableDefinition<&str, u64> = TableDefinition::new("changed");
/// photo id → generation it was deleted in.
const TOMBS: TableDefinition<&str, u64> = TableDefinition::new("tombs");
/// `<photo id>/<tier>` → JSON [`ImageRec`].
const IMAGES: TableDefinition<&str, &str> = TableDefinition::new("images");

/// Whether a file's record is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileState {
    /// Found on the last scan.
    #[default]
    Present,
    /// Its root could not be reached on the last scan (a drive not plugged in): shown,
    /// marked offline, everything about it kept.
    Offline,
    /// Its root was taken out of the definition: not shown (deleted from the library), but
    /// kept, so it gets its id and edits back if the root (or a copy of it) comes back.
    Detached,
}

/// One photo file the source knows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileRec {
    /// [`crate::RootDef::id`].
    pub root: String,
    /// Below the root, `/`-separated, with the file name: `2024/rome/DSCF0001.RAF`.
    pub relpath: String,
    pub kind: FileKind,
    pub size: u64,
    /// Modification time, nanoseconds since the epoch.
    pub mtime_ns: i64,
    /// The XMP sidecar read with it, as a file name in the same folder.
    #[serde(default)]
    pub sidecar: Option<String>,
    #[serde(default)]
    pub sidecar_mtime_ns: Option<i64>,
    /// Identifies the content wherever the file goes (see [`crate::ids`]).
    pub content: ContentKey,
    /// Hash of the whole file, 32 hex digits: worked out only when another file has the
    /// same [`FileRec::content`].
    #[serde(default)]
    pub full: Option<String>,
    #[serde(default)]
    pub meta: FileMeta,
    #[serde(default)]
    pub state: FileState,
    /// When the source first saw it, Unix epoch milliseconds.
    pub imported_at: i64,
}

impl FileRec {
    /// The file name: `DSCF0001.RAF`.
    pub fn name(&self) -> &str {
        self.relpath.rsplit('/').next().unwrap_or(&self.relpath)
    }

    /// The folder below the root, `/`-separated; empty for the root itself.
    pub fn reldir(&self) -> &str {
        self.relpath.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
    }

    /// The file's path under `root` (the root's path), in this machine's native form.
    pub fn path_under(&self, root: &str) -> PathBuf {
        join_rel(Path::new(root), &self.relpath)
    }
}

/// `base` joined with a `/`-separated relative path, one component at a time, so the
/// separators come out native.
pub fn join_rel(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').filter(|c| !c.is_empty()).fold(base.to_owned(), |p, c| p.join(c))
}

/// An image the source made for a photo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageRec {
    pub path: PathBuf,
    /// The original's size and modification time when the image was made: a mismatch
    /// means it is out of date.
    pub src_size: u64,
    pub src_mtime_ns: i64,
    /// The image file's own size, for the store's size limit.
    pub bytes: u64,
}

/// Everything in the store.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// Made with the store; a cursor from another store is never taken as this one's.
    pub uuid: String,
    /// Generation of the last write; every write that changes anything bumps it.
    pub generation: u64,
    /// Albums made so far, for their ids (`a1`, `a2`, ...).
    pub made: u64,
    pub files: BTreeMap<String, FileRec>,
    /// (root id, relpath) → photo id.
    pub by_path: HashMap<(String, String), String>,
    pub user: HashMap<String, UserLayer>,
    /// The user's albums, groups and smart albums (folders are not stored: they follow the
    /// files).
    pub albums: Vec<Collection>,
    /// Generation the albums or the folders last changed in.
    pub albums_gen: u64,
    pub changed: HashMap<String, u64>,
    pub tombs: HashMap<String, u64>,
    pub images: HashMap<(String, ImageTier), ImageRec>,
}

impl State {
    /// The photo id of the file at `relpath` under `root`.
    pub fn at_path(&self, root: &str, relpath: &str) -> Option<&str> {
        self.by_path.get(&(root.to_owned(), relpath.to_owned())).map(String::as_str)
    }
}

pub struct Store {
    db: Database,
    state: RwLock<State>,
}

pub(crate) fn other(e: impl std::fmt::Display) -> SourceError {
    SourceError::Other(format!("folder store: {e}"))
}

fn tier_key(ext: &str, tier: ImageTier) -> String {
    format!("{ext}/{tier:?}")
}

fn tier_of(key: &str) -> Option<(String, ImageTier)> {
    let (ext, tier) = key.rsplit_once('/')?;
    let tier = match tier {
        "Thumb" => ImageTier::Thumb,
        "Preview" => ImageTier::Preview,
        "Original" => ImageTier::Original,
        _ => return None,
    };
    Some((ext.to_owned(), tier))
}

fn json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(other)
}

fn parse<T: DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(other)
}

impl Store {
    /// Opens the store at `path`, making it if there is none.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let db = Database::create(path).map_err(other)?;
        let tx = db.begin_write().map_err(other)?;
        {
            for table in [META, FILES, USER, IMAGES] {
                tx.open_table(table).map_err(other)?;
            }
            tx.open_table(CHANGED).map_err(other)?;
            tx.open_table(TOMBS).map_err(other)?;
            let mut meta = tx.open_table(META).map_err(other)?;
            if meta.get("uuid").map_err(other)?.is_none() {
                meta.insert("uuid", json(&uuid::Uuid::new_v4().simple().to_string())?.as_str()).map_err(other)?;
                meta.insert("version", json(&STORE_VERSION)?.as_str()).map_err(other)?;
            }
        }
        tx.commit().map_err(other)?;
        let store = Self { db, state: RwLock::new(State::default()) };
        store.reload()?;
        Ok(store)
    }

    /// Reads everything from disk again, dropping what is held in memory.
    pub fn reload(&self) -> Result<()> {
        let state = self.load()?;
        *self.state.write().unwrap() = state;
        Ok(())
    }

    fn load(&self) -> Result<State> {
        let tx = self.db.begin_read().map_err(other)?;
        let mut state = State::default();
        let meta = tx.open_table(META).map_err(other)?;
        let get =
            |key: &str| -> Result<Option<String>> { Ok(meta.get(key).map_err(other)?.map(|v| v.value().to_owned())) };
        state.uuid = get("uuid")?.map(|v| parse(&v)).transpose()?.unwrap_or_default();
        state.generation = get("generation")?.map(|v| parse(&v)).transpose()?.unwrap_or(0);
        state.made = get("made")?.map(|v| parse(&v)).transpose()?.unwrap_or(0);
        state.albums = get("albums")?.map(|v| parse(&v)).transpose()?.unwrap_or_default();
        state.albums_gen = get("albums_gen")?.map(|v| parse(&v)).transpose()?.unwrap_or(0);
        for entry in tx.open_table(FILES).map_err(other)?.iter().map_err(other)? {
            let (ext, rec) = entry.map_err(other)?;
            let rec: FileRec = parse(rec.value())?;
            state.by_path.insert((rec.root.clone(), rec.relpath.clone()), ext.value().to_owned());
            state.files.insert(ext.value().to_owned(), rec);
        }
        for entry in tx.open_table(USER).map_err(other)?.iter().map_err(other)? {
            let (ext, layer) = entry.map_err(other)?;
            state.user.insert(ext.value().to_owned(), parse(layer.value())?);
        }
        for entry in tx.open_table(CHANGED).map_err(other)?.iter().map_err(other)? {
            let (ext, generation) = entry.map_err(other)?;
            state.changed.insert(ext.value().to_owned(), generation.value());
        }
        for entry in tx.open_table(TOMBS).map_err(other)?.iter().map_err(other)? {
            let (ext, generation) = entry.map_err(other)?;
            state.tombs.insert(ext.value().to_owned(), generation.value());
        }
        for entry in tx.open_table(IMAGES).map_err(other)?.iter().map_err(other)? {
            let (key, rec) = entry.map_err(other)?;
            if let Some(key) = tier_of(key.value()) {
                state.images.insert(key, parse(rec.value())?);
            }
        }
        Ok(state)
    }

    /// Reads the store as it is now. Writers wait until `f` returns: keep it short.
    pub fn read(&self) -> RwLockReadGuard<'_, State> {
        self.state.read().unwrap()
    }

    /// Runs `f` as one transaction: all of its changes land, on disk and in memory, or none
    /// do. Readers wait meanwhile, so do slow work (reading files) before.
    pub fn write<T>(&self, f: impl FnOnce(&mut Writer) -> Result<T>) -> Result<T> {
        let mut state = self.state.write().unwrap();
        let tx = self.db.begin_write().map_err(other)?;
        let generation = state.generation + 1;
        let mut writer = Writer { tx, state: &mut state, generation, touched: false };
        let result = f(&mut writer).and_then(|value| {
            let Writer { tx, state, generation, touched } = writer;
            if touched {
                state.generation = generation;
                let mut meta = tx.open_table(META).map_err(other)?;
                meta.insert("generation", json(&generation)?.as_str()).map_err(other)?;
                meta.insert("made", json(&state.made)?.as_str()).map_err(other)?;
                drop(meta);
            }
            tx.commit().map_err(other)?;
            Ok(value)
        });
        if result.is_err() {
            // What was changed in memory is not on disk: take the disk's word.
            drop(state);
            self.reload()?;
        }
        result
    }
}

/// Changes made inside [`Store::write`]. Each one is made in memory and in the transaction.
pub struct Writer<'a> {
    tx: WriteTransaction,
    state: &'a mut State,
    generation: u64,
    touched: bool,
}

impl Writer<'_> {
    /// The generation this write commits as.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn state(&self) -> &State {
        self.state
    }

    /// The transaction, for tables of a module's own.
    pub fn tx(&self) -> &WriteTransaction {
        &self.tx
    }

    /// Marks a photo changed in this generation, so an incremental snapshot sends it.
    pub fn touch(&mut self, ext: &str) -> Result<()> {
        self.touched = true;
        self.tx.open_table(CHANGED).map_err(other)?.insert(ext, self.generation).map_err(other)?;
        self.state.changed.insert(ext.to_owned(), self.generation);
        Ok(())
    }

    /// Adds or replaces a file's record (and touches it). A photo id that was deleted
    /// before is alive again.
    pub fn put_file(&mut self, ext: &str, rec: FileRec) -> Result<()> {
        if let Some(old) = self.state.files.get(ext) {
            let old_key = (old.root.clone(), old.relpath.clone());
            if self.state.by_path.get(&old_key).is_some_and(|e| e == ext) {
                self.state.by_path.remove(&old_key);
            }
        }
        self.tx.open_table(FILES).map_err(other)?.insert(ext, json(&rec)?.as_str()).map_err(other)?;
        if self.state.tombs.remove(ext).is_some() {
            self.tx.open_table(TOMBS).map_err(other)?.remove(ext).map_err(other)?;
        }
        self.state.by_path.insert((rec.root.clone(), rec.relpath.clone()), ext.to_owned());
        self.state.files.insert(ext.to_owned(), rec);
        self.touch(ext)
    }

    /// Forgets a file for good, with the user's edits on it, and records the deletion
    /// for incremental snapshots. Its images' records go too (not the image files).
    pub fn remove_file(&mut self, ext: &str) -> Result<()> {
        self.touched = true;
        if let Some(old) = self.state.files.remove(ext) {
            let key = (old.root, old.relpath);
            if self.state.by_path.get(&key).is_some_and(|e| e == ext) {
                self.state.by_path.remove(&key);
            }
        }
        self.tx.open_table(FILES).map_err(other)?.remove(ext).map_err(other)?;
        self.state.user.remove(ext);
        self.tx.open_table(USER).map_err(other)?.remove(ext).map_err(other)?;
        self.state.changed.remove(ext);
        self.tx.open_table(CHANGED).map_err(other)?.remove(ext).map_err(other)?;
        self.tomb(ext)?;
        for tier in [ImageTier::Thumb, ImageTier::Preview] {
            self.put_image(ext, tier, None)?;
        }
        Ok(())
    }

    /// Records that the photo is no longer shown (deleted from the library) in this
    /// generation, without forgetting its record (a detached root's files).
    pub fn tomb(&mut self, ext: &str) -> Result<()> {
        self.touched = true;
        self.tx.open_table(TOMBS).map_err(other)?.insert(ext, self.generation).map_err(other)?;
        self.state.tombs.insert(ext.to_owned(), self.generation);
        Ok(())
    }

    /// Sets (or with `None` clears) the user's edits on a photo, and touches it.
    pub fn put_user(&mut self, ext: &str, layer: Option<UserLayer>) -> Result<()> {
        let mut table = self.tx.open_table(USER).map_err(other)?;
        match layer.filter(|l| !l.is_empty()) {
            Some(layer) => {
                table.insert(ext, json(&layer)?.as_str()).map_err(other)?;
                self.state.user.insert(ext.to_owned(), layer);
            }
            None => {
                table.remove(ext).map_err(other)?;
                self.state.user.remove(ext);
            }
        }
        drop(table);
        self.touch(ext)
    }

    pub fn set_albums(&mut self, albums: Vec<Collection>) -> Result<()> {
        self.tx.open_table(META).map_err(other)?.insert("albums", json(&albums)?.as_str()).map_err(other)?;
        self.state.albums = albums;
        self.folders_changed()
    }

    /// The folder tree (or the albums) changed: the next incremental snapshot sends the
    /// collections again.
    pub fn folders_changed(&mut self) -> Result<()> {
        self.touched = true;
        self.tx
            .open_table(META)
            .map_err(other)?
            .insert("albums_gen", json(&self.generation)?.as_str())
            .map_err(other)?;
        self.state.albums_gen = self.generation;
        Ok(())
    }

    /// A new album id: `a1`, `a2`, ...
    pub fn next_album_id(&mut self) -> CollectionId {
        self.touched = true;
        self.state.made += 1;
        CollectionId(format!("a{}", self.state.made))
    }

    /// Raises the album counter to `made`; never lowers it, so no album id is given twice.
    pub(crate) fn raise_made(&mut self, made: u64) {
        if made > self.state.made {
            self.touched = true;
            self.state.made = made;
        }
    }

    /// Records (or with `None` forgets) an image made for a photo. Does not touch the
    /// photo: its pixels are not part of what a snapshot sends.
    pub fn put_image(&mut self, ext: &str, tier: ImageTier, rec: Option<ImageRec>) -> Result<()> {
        let key = tier_key(ext, tier);
        let mut table = self.tx.open_table(IMAGES).map_err(other)?;
        match rec {
            Some(rec) => {
                table.insert(key.as_str(), json(&rec)?.as_str()).map_err(other)?;
                self.state.images.insert((ext.to_owned(), tier), rec);
            }
            None => {
                if self.state.images.remove(&(ext.to_owned(), tier)).is_some() {
                    table.remove(key.as_str()).map_err(other)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn rec(root: &str, relpath: &str) -> FileRec {
        FileRec {
            root: root.into(),
            relpath: relpath.into(),
            kind: FileKind::Jpeg,
            size: 10,
            mtime_ns: 1,
            sidecar: None,
            sidecar_mtime_ns: None,
            content: ContentKey { size: 10, hash: "00".into() },
            full: None,
            meta: FileMeta::default(),
            state: FileState::Present,
            imported_at: 0,
        }
    }

    #[test]
    fn writes_land_on_disk_and_failures_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("folders.redb");
        let store = Store::open(&path).unwrap();
        let uuid = store.read().uuid.clone();
        assert_eq!(uuid.len(), 32);
        store
            .write(|w| {
                w.put_file("aa", rec("r", "x/1.jpg"))?;
                w.put_user("aa", Some(UserLayer { rating: Some(Some(4)), ..UserLayer::default() }))?;
                let id = w.next_album_id();
                assert_eq!(id.0, "a1");
                Ok(())
            })
            .unwrap();
        let failed: Result<()> = store.write(|w| {
            w.put_file("bb", rec("r", "y.jpg"))?;
            Err(SourceError::Other("no".into()))
        });
        assert!(failed.is_err());
        assert!(!store.read().files.contains_key("bb"), "a failed write leaves nothing in memory");
        drop(store);
        let store = Store::open(&path).unwrap();
        let state = store.read();
        assert_eq!(state.uuid, uuid);
        assert_eq!(state.generation, 1);
        assert_eq!(state.made, 1);
        assert_eq!(state.at_path("r", "x/1.jpg"), Some("aa"));
        assert_eq!(state.user["aa"].rating, Some(Some(4)));
        assert_eq!(state.changed["aa"], 1);
        drop(state);
        store.write(|w| w.remove_file("aa")).unwrap();
        let state = store.read();
        assert!(state.files.is_empty() && state.user.is_empty() && state.by_path.is_empty());
        assert_eq!(state.tombs["aa"], 2);
    }

    #[test]
    fn a_write_that_changes_nothing_keeps_the_generation() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("s.redb")).unwrap();
        store.write(|_| Ok(())).unwrap();
        assert_eq!(store.read().generation, 0);
    }

    #[test]
    fn rel_paths_join_natively() {
        let p = join_rel(Path::new("/r"), "a/b/c.jpg");
        assert_eq!(p, Path::new("/r").join("a").join("b").join("c.jpg"));
        let r = rec("r", "a/b/c.jpg");
        assert_eq!((r.name(), r.reldir()), ("c.jpg", "a/b"));
        assert_eq!(rec("r", "c.jpg").reldir(), "");
    }
}
