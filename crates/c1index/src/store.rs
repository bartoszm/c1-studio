//! The durable copy of everything the sources reported, in redb (an embedded,
//! transactional key–value store). The search index is derived from this and can be
//! rebuilt from it at any time.
//!
//! Every photo gets a dense `row` number on first sight. Rows are what the index and the
//! UI pass around: a `u64` per photo instead of a string id.

use std::path::Path;

use anyhow::Context;
use c1model::{Collection, KeywordNode, Photo};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

/// row → postcard-encoded [`Photo`].
const PHOTOS: TableDefinition<u64, &[u8]> = TableDefinition::new("photos");
/// row → fingerprint of the photo as last stored.
const FINGERPRINTS: TableDefinition<u64, u64> = TableDefinition::new("fingerprints");
/// photo key (`PhotoId::key`) → row.
const KEYS: TableDefinition<&str, u64> = TableDefinition::new("keys");
/// source id → postcard `Vec<Collection>`.
const COLLECTIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("collections");
/// source id → postcard `Vec<KeywordNode>`.
const KEYWORDS: TableDefinition<&str, &[u8]> = TableDefinition::new("keywords");
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");

pub struct Store {
    db: Database,
}

/// A batch of writes, committed atomically.
pub struct Batch<'a> {
    tx: redb::WriteTransaction,
    _store: &'a Store,
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let db = Database::create(path).with_context(|| format!("opening store {}", path.display()))?;
        // Create every table up front so read transactions never see one missing.
        let tx = db.begin_write()?;
        tx.open_table(PHOTOS)?;
        tx.open_table(FINGERPRINTS)?;
        tx.open_table(KEYS)?;
        tx.open_table(COLLECTIONS)?;
        tx.open_table(KEYWORDS)?;
        tx.open_table(META)?;
        tx.commit()?;
        Ok(Self { db })
    }

    pub fn batch(&self) -> anyhow::Result<Batch<'_>> {
        Ok(Batch { tx: self.db.begin_write()?, _store: self })
    }

    pub fn meta(&self, key: &str) -> anyhow::Result<Option<String>> {
        let tx = self.db.begin_read()?;
        Ok(tx.open_table(META)?.get(key)?.map(|v| v.value().to_owned()))
    }

    pub fn photo(&self, row: u64) -> anyhow::Result<Option<Photo>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(PHOTOS)?;
        let Some(bytes) = table.get(row)? else { return Ok(None) };
        Ok(Some(postcard::from_bytes(bytes.value())?))
    }

    /// Every stored photo, in row order.
    pub fn for_each_photo(&self, mut f: impl FnMut(u64, Photo) -> anyhow::Result<()>) -> anyhow::Result<()> {
        let tx = self.db.begin_read()?;
        for entry in tx.open_table(PHOTOS)?.iter()? {
            let (row, bytes) = entry?;
            f(row.value(), postcard::from_bytes(bytes.value())?)?;
        }
        Ok(())
    }

    pub fn row_of(&self, key: &str) -> anyhow::Result<Option<u64>> {
        let tx = self.db.begin_read()?;
        Ok(tx.open_table(KEYS)?.get(key)?.map(|v| v.value()))
    }

    /// Row → photo key, for every row.
    pub fn keys(&self) -> anyhow::Result<Vec<(u64, String)>> {
        let tx = self.db.begin_read()?;
        let mut out = Vec::new();
        for entry in tx.open_table(KEYS)?.iter()? {
            let (key, row) = entry?;
            out.push((row.value(), key.value().to_owned()));
        }
        Ok(out)
    }

    pub fn collections(&self) -> anyhow::Result<Vec<Collection>> {
        self.per_source(COLLECTIONS)
    }

    pub fn keywords(&self) -> anyhow::Result<Vec<KeywordNode>> {
        self.per_source(KEYWORDS)
    }

    fn per_source<T: serde::de::DeserializeOwned>(&self, def: TableDefinition<&str, &[u8]>) -> anyhow::Result<Vec<T>> {
        let tx = self.db.begin_read()?;
        let mut out = Vec::new();
        for entry in tx.open_table(def)?.iter()? {
            let (_, bytes) = entry?;
            out.extend(postcard::from_bytes::<Vec<T>>(bytes.value())?);
        }
        Ok(out)
    }
}

impl Batch<'_> {
    pub fn row_of(&self, key: &str) -> anyhow::Result<Option<u64>> {
        Ok(self.tx.open_table(KEYS)?.get(key)?.map(|v| v.value()))
    }

    /// The photo as this batch would leave it.
    pub fn photo(&self, row: u64) -> anyhow::Result<Option<Photo>> {
        let table = self.tx.open_table(PHOTOS)?;
        let Some(bytes) = table.get(row)? else { return Ok(None) };
        Ok(Some(postcard::from_bytes(bytes.value())?))
    }

    pub fn fingerprint(&self, row: u64) -> anyhow::Result<Option<u64>> {
        Ok(self.tx.open_table(FINGERPRINTS)?.get(row)?.map(|v| v.value()))
    }

    /// Allocates the next row number.
    pub fn next_row(&mut self) -> anyhow::Result<u64> {
        let mut meta = self.tx.open_table(META)?;
        let next = meta.get("next_row")?.map_or(Ok(0), |v| v.value().parse::<u64>())?;
        meta.insert("next_row", (next + 1).to_string().as_str())?;
        Ok(next)
    }

    pub fn put_photo(&mut self, row: u64, photo: &Photo, fingerprint: u64) -> anyhow::Result<()> {
        let bytes = postcard::to_stdvec(photo)?;
        self.tx.open_table(PHOTOS)?.insert(row, bytes.as_slice())?;
        self.tx.open_table(FINGERPRINTS)?.insert(row, fingerprint)?;
        self.tx.open_table(KEYS)?.insert(photo.id.key().as_str(), row)?;
        Ok(())
    }

    pub fn delete_photo(&mut self, row: u64, key: &str) -> anyhow::Result<()> {
        self.tx.open_table(PHOTOS)?.remove(row)?;
        self.tx.open_table(FINGERPRINTS)?.remove(row)?;
        self.tx.open_table(KEYS)?.remove(key)?;
        Ok(())
    }

    /// The collections last stored for one source.
    pub fn collections_of(&self, source: &str) -> anyhow::Result<Vec<Collection>> {
        let table = self.tx.open_table(COLLECTIONS)?;
        let Some(bytes) = table.get(source)? else { return Ok(Vec::new()) };
        Ok(postcard::from_bytes(bytes.value())?)
    }

    pub fn put_collections(&mut self, source: &str, collections: &[Collection]) -> anyhow::Result<()> {
        let bytes = postcard::to_stdvec(collections)?;
        self.tx.open_table(COLLECTIONS)?.insert(source, bytes.as_slice())?;
        Ok(())
    }

    /// The keyword tree last stored for one source.
    pub fn keywords_of(&self, source: &str) -> anyhow::Result<Vec<KeywordNode>> {
        let table = self.tx.open_table(KEYWORDS)?;
        let Some(bytes) = table.get(source)? else { return Ok(Vec::new()) };
        Ok(postcard::from_bytes(bytes.value())?)
    }

    pub fn put_keywords(&mut self, source: &str, keywords: &[KeywordNode]) -> anyhow::Result<()> {
        let bytes = postcard::to_stdvec(keywords)?;
        self.tx.open_table(KEYWORDS)?.insert(source, bytes.as_slice())?;
        Ok(())
    }

    pub fn metas_with_prefix(&self, prefix: &str) -> anyhow::Result<Vec<(String, String)>> {
        let table = self.tx.open_table(META)?;
        let mut out = Vec::new();
        for entry in table.iter()? {
            let (k, v) = entry?;
            if k.value().starts_with(prefix) {
                out.push((k.value().to_owned(), v.value().to_owned()));
            }
        }
        Ok(out)
    }

    pub fn remove_meta(&mut self, key: &str) -> anyhow::Result<()> {
        self.tx.open_table(META)?.remove(key)?;
        Ok(())
    }

    pub fn set_meta(&mut self, key: &str, value: &str) -> anyhow::Result<()> {
        self.tx.open_table(META)?.insert(key, value)?;
        Ok(())
    }

    pub fn commit(self) -> anyhow::Result<()> {
        self.tx.commit()?;
        Ok(())
    }
}
