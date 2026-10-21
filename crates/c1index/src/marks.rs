//! Marks of C1 Studio's own on photos, never written to the source: photos the user hid
//! (sensitive, private: kept out of every view until shown), and photos the user deleted
//! (in the app's trash until restored, or until the trash is emptied and the source deletes
//! them for good).
//!
//! Each mark is kept in its own file (`hidden.redb`, `bin.redb`), like places and trips: it
//! outlives the store, which is wiped when the model changes. Photos are named by their
//! key, which a rebuilt store keeps.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

/// A set of photos with one mark: photo key → when it was marked (epoch ms).
pub struct MarkBook {
    db: Database,
    table: TableDefinition<'static, &'static str, i64>,
}

impl MarkBook {
    /// Photos the user hid.
    pub fn hidden(path: &Path) -> anyhow::Result<Self> {
        Self::open(path, "hidden")
    }

    /// Photos the user deleted, in the app's trash.
    pub fn bin(path: &Path) -> anyhow::Result<Self> {
        Self::open(path, "bin")
    }

    fn open(path: &Path, table: &'static str) -> anyhow::Result<Self> {
        let db = Database::create(path).with_context(|| format!("opening {table} {}", path.display()))?;
        let table = TableDefinition::new(table);
        let tx = db.begin_write()?;
        tx.open_table(table)?;
        tx.commit()?;
        Ok(Self { db, table })
    }

    /// Every marked photo, by key.
    pub fn all(&self) -> anyhow::Result<HashSet<String>> {
        let tx = self.db.begin_read()?;
        let mut out = HashSet::new();
        for entry in tx.open_table(self.table)?.iter()? {
            out.insert(entry?.0.value().to_owned());
        }
        Ok(out)
    }

    /// Marks (`true`) or unmarks (`false`) these photos, by key.
    pub fn set(&self, keys: &[String], marked: bool, at: i64) -> anyhow::Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(self.table)?;
            for key in keys {
                if marked {
                    table.insert(key.as_str(), at)?;
                } else {
                    table.remove(key.as_str())?;
                }
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
    fn the_book_keeps_what_is_marked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hidden.redb");
        let book = MarkBook::hidden(&path).unwrap();
        book.set(&["a".into(), "b".into()], true, 1).unwrap();
        book.set(&["a".into()], false, 2).unwrap();
        drop(book);
        let book = MarkBook::hidden(&path).unwrap();
        assert_eq!(book.all().unwrap(), HashSet::from(["b".to_owned()]));
    }
}
