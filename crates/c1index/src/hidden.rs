//! Photos the user hid (sensitive, private): kept out of every view until shown.
//!
//! A mark of C1 Studio's own, never written to the source. Kept in its own file
//! (`hidden.redb`), like places and trips: it outlives the store, which is wiped when the
//! model changes. Photos are named by their key, which a rebuilt store keeps.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

/// A photo key → when it was hidden (epoch ms).
const HIDDEN: TableDefinition<&str, i64> = TableDefinition::new("hidden");

pub struct HiddenBook {
    db: Database,
}

impl HiddenBook {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let db = Database::create(path).with_context(|| format!("opening hidden {}", path.display()))?;
        let tx = db.begin_write()?;
        tx.open_table(HIDDEN)?;
        tx.commit()?;
        Ok(Self { db })
    }

    /// Every hidden photo, by key.
    pub fn all(&self) -> anyhow::Result<HashSet<String>> {
        let tx = self.db.begin_read()?;
        let mut out = HashSet::new();
        for entry in tx.open_table(HIDDEN)?.iter()? {
            out.insert(entry?.0.value().to_owned());
        }
        Ok(out)
    }

    /// Hides (`true`) or shows again (`false`) these photos, by key.
    pub fn set(&self, keys: &[String], hidden: bool, at: i64) -> anyhow::Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(HIDDEN)?;
            for key in keys {
                if hidden {
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
    fn the_book_keeps_what_is_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hidden.redb");
        let book = HiddenBook::open(&path).unwrap();
        book.set(&["a".into(), "b".into()], true, 1).unwrap();
        book.set(&["a".into()], false, 2).unwrap();
        drop(book);
        let book = HiddenBook::open(&path).unwrap();
        assert_eq!(book.all().unwrap(), HashSet::from(["b".to_owned()]));
    }
}
