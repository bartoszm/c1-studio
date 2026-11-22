//! What the user decided while choosing the best of alike photos: the photos picked as
//! the ones to keep. Kept apart from the store like the other marks of C1 Studio's own: a
//! pick is the user's work, and outlives the store being rebuilt. Photos are named by key.

use std::collections::HashSet;
use std::path::Path;

use crate::db::OpenControl;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

/// photo key → when it was picked (epoch ms).
const KEEPERS: TableDefinition<&str, i64> = TableDefinition::new("keepers");

pub struct CullBook {
    db: Database,
}

impl CullBook {
    pub fn open(path: &Path, ctl: &OpenControl) -> anyhow::Result<Self> {
        let db = crate::db::open(path, "cull", ctl)?;
        let tx = db.begin_write()?;
        tx.open_table(KEEPERS)?;
        tx.commit()?;
        Ok(Self { db })
    }

    /// Every photo picked as the one to keep of its group, by key.
    pub fn keepers(&self) -> anyhow::Result<HashSet<String>> {
        let tx = self.db.begin_read()?;
        let mut out = HashSet::new();
        for entry in tx.open_table(KEEPERS)?.iter()? {
            out.insert(entry?.0.value().to_owned());
        }
        Ok(out)
    }

    /// Picks (`true`) these photos, by key, or takes the pick back (`false`).
    pub fn set_keepers(&self, keys: &[String], kept: bool, at: i64) -> anyhow::Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(KEEPERS)?;
            for key in keys {
                if kept {
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
    fn picks_are_kept_until_taken_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cull.redb");
        let book = CullBook::open(&path, &Default::default()).unwrap();
        book.set_keepers(&["a".into(), "b".into()], true, 1).unwrap();
        book.set_keepers(&["a".into()], false, 2).unwrap();
        drop(book);
        let book = CullBook::open(&path, &Default::default()).unwrap();
        assert_eq!(book.keepers().unwrap(), HashSet::from(["b".to_owned()]));
    }
}
