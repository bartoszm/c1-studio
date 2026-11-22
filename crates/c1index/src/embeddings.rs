//! What each photo looks like to the local model: its embedding, a list of numbers by
//! which photos are compared. Kept apart from the store, like analyses: an embedding
//! takes a quarter of a second to work out, tens of thousands take hours.

use std::collections::HashSet;
use std::path::Path;

use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};

use crate::db::OpenControl;

/// photo key → the numbers, four bytes each, little-endian.
const EMBEDDINGS: TableDefinition<&str, &[u8]> = TableDefinition::new("embeddings");
/// name → value.
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");
const MODEL: &str = "model";

pub struct Embeddings {
    db: Database,
}

impl Embeddings {
    pub fn open(path: &Path, ctl: &OpenControl) -> anyhow::Result<Self> {
        let db = crate::db::open(path, "embeddings", ctl)?;
        let tx = db.begin_write()?;
        tx.open_table(EMBEDDINGS)?;
        tx.open_table(META)?;
        tx.commit()?;
        Ok(Self { db })
    }

    /// Says the embeddings are `model`'s from here on: those of another model cannot be
    /// compared with them, and go.
    pub fn of_model(&self, model: &str) -> anyhow::Result<()> {
        // Nearly always they are: asked without writing.
        if self.db.begin_read()?.open_table(META)?.get(MODEL)?.is_some_and(|m| m.value() == model) {
            return Ok(());
        }
        let tx = self.db.begin_write()?;
        tx.open_table(EMBEDDINGS)?.retain(|_, _| false)?;
        tx.open_table(META)?.insert(MODEL, model)?;
        tx.commit()?;
        Ok(())
    }

    pub fn len(&self) -> anyhow::Result<u64> {
        Ok(self.db.begin_read()?.open_table(EMBEDDINGS)?.len()?)
    }

    pub fn is_empty(&self) -> anyhow::Result<bool> {
        Ok(self.len()? == 0)
    }

    /// The photos that have one.
    pub fn keys(&self) -> anyhow::Result<HashSet<String>> {
        let tx = self.db.begin_read()?;
        let mut out = HashSet::new();
        for entry in tx.open_table(EMBEDDINGS)?.iter()? {
            out.insert(entry?.0.value().to_owned());
        }
        Ok(out)
    }

    /// Hands `f` every photo's key and embedding.
    pub fn each(&self, mut f: impl FnMut(&str, &[f32])) -> anyhow::Result<()> {
        let tx = self.db.begin_read()?;
        let mut numbers: Vec<f32> = Vec::new();
        for entry in tx.open_table(EMBEDDINGS)?.iter()? {
            let (key, bytes) = entry?;
            numbers.clear();
            numbers.extend(bytes.value().as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
            f(key.value(), &numbers);
        }
        Ok(())
    }

    /// Keeps these, but for a photo that has one: its first stays, so that the photos
    /// learned from are all seen the same way.
    pub fn put(&self, batch: &[(String, Vec<f32>)]) -> anyhow::Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(EMBEDDINGS)?;
            let mut bytes = Vec::new();
            for (key, numbers) in batch {
                if table.get(key.as_str())?.is_some() {
                    continue;
                }
                bytes.clear();
                numbers.iter().for_each(|x| bytes.extend_from_slice(&x.to_le_bytes()));
                table.insert(key.as_str(), bytes.as_slice())?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeddings_are_kept_and_those_of_another_model_go() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("embeddings.redb");
        let book = Embeddings::open(&path, &Default::default()).unwrap();
        book.of_model("one").unwrap();
        assert!(book.is_empty().unwrap());
        book.put(&[("a".into(), vec![0.5, -1.25]), ("b".into(), vec![2.0, 3.0])]).unwrap();
        drop(book);

        let book = Embeddings::open(&path, &Default::default()).unwrap();
        book.of_model("one").unwrap();
        assert_eq!(book.len().unwrap(), 2);
        assert_eq!(book.keys().unwrap(), HashSet::from(["a".to_string(), "b".to_string()]));
        let mut seen = Vec::new();
        book.each(|key, numbers| seen.push((key.to_string(), numbers.to_vec()))).unwrap();
        assert_eq!(seen, [("a".to_string(), vec![0.5, -1.25]), ("b".to_string(), vec![2.0, 3.0])]);

        book.put(&[("a".into(), vec![9.0, 9.0]), ("c".into(), vec![1.0, 1.0])]).unwrap();
        assert_eq!(book.len().unwrap(), 3);
        let mut first = Vec::new();
        book.each(|key, numbers| first.extend(numbers.iter().copied().filter(|_| key == "a"))).unwrap();
        assert_eq!(first, [0.5, -1.25], "the first one a photo got stays");

        book.of_model("two").unwrap();
        assert!(book.is_empty().unwrap(), "another model's numbers mean something else");
    }
}
