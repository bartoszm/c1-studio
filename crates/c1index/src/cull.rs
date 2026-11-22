//! What choosing the best of alike photos leaves behind: the photos the user picked as
//! the ones to keep, and how a model saw each photo it looked at. Kept apart from the
//! store like the other marks of C1 Studio's own: a pick is the user's work, a model's
//! view is slow to make again, and both outlive the store being rebuilt. Photos are named
//! by key.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::db::OpenControl;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

/// photo key → when it was picked (epoch ms).
const KEEPERS: TableDefinition<&str, i64> = TableDefinition::new("keepers");
/// photo key → postcard [`View`].
const VIEWS: TableDefinition<&str, &[u8]> = TableDefinition::new("views");

/// How the local model saw a photo: its embedding, and what that came to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// The version of the scoring (`c1ai::aesthetic::VERSION`) that made `scores`: another
    /// version's are worked out again from the embedding.
    pub version: u32,
    /// Size and modification time of the image looked at, as the analysis stamps it: a new
    /// render is looked at again.
    pub stamp: (u64, u64),
    /// The picture's embedding: what the scores are worked out from again when the way
    /// of scoring changes, without the model looking a second time.
    pub embedding: Vec<f32>,
    /// One score per axis in the order of `c1ai::aesthetic::AXES`, each 0 to 1.
    pub scores: [f32; 4],
}

/// A view as the model made it, of the photo at `row`: [`crate::Library::put_views`] adds
/// the stamp of the render it is of.
#[derive(Clone, Debug, PartialEq)]
pub struct NewView {
    pub row: u64,
    /// See [`View::version`].
    pub version: u32,
    pub embedding: Vec<f32>,
    /// See [`View::scores`].
    pub scores: [f32; 4],
}

pub struct CullBook {
    db: Database,
}

impl CullBook {
    pub fn open(path: &Path, ctl: &OpenControl) -> anyhow::Result<Self> {
        let db = crate::db::open(path, "cull", ctl)?;
        let tx = db.begin_write()?;
        tx.open_table(KEEPERS)?;
        tx.open_table(VIEWS)?;
        tx.commit()?;
        Ok(Self { db })
    }

    /// The model's view of each of these photos it has looked at, by key.
    pub fn views<'a>(&self, keys: impl IntoIterator<Item = &'a str>) -> anyhow::Result<HashMap<String, View>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(VIEWS)?;
        let mut out = HashMap::new();
        for key in keys {
            // One that no longer reads is looked at again.
            if let Some(bytes) = table.get(key)?
                && let Ok(view) = postcard::from_bytes::<View>(bytes.value())
            {
                out.insert(key.to_owned(), view);
            }
        }
        Ok(out)
    }

    pub fn put_views(&self, views: &[(String, View)]) -> anyhow::Result<()> {
        if views.is_empty() {
            return Ok(());
        }
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(VIEWS)?;
            for (key, view) in views {
                table.insert(key.as_str(), postcard::to_stdvec(view)?.as_slice())?;
            }
        }
        tx.commit()?;
        Ok(())
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

    #[test]
    fn a_view_is_kept_and_read_back_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cull.redb");
        let book = CullBook::open(&path, &Default::default()).unwrap();
        let view = View { version: 1, stamp: (10, 20), embedding: vec![0.25, -0.5], scores: [0.1, 0.2, 0.3, 0.4] };
        book.put_views(&[("a".into(), view.clone())]).unwrap();
        drop(book);
        let book = CullBook::open(&path, &Default::default()).unwrap();
        let read = book.views(["a", "missing"]).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read["a"], view);
    }
}
