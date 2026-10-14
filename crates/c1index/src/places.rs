//! Where places are: what the gazetteer has no answer for, as an online lookup found it,
//! and which photos were given only their place's position.
//!
//! Kept in its own file (`places.redb`), like trips: it outlives the store, which is wiped
//! when the model changes.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use c1geo::{Approx, Level};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

/// A place's path, as [`crate::schema::place_tree`] writes it → a looked-up [`Looked`].
const ONLINE: TableDefinition<&str, &[u8]> = TableDefinition::new("online");
/// A photo key → the [`Coarse`] position it was given.
const COARSE: TableDefinition<&str, &[u8]> = TableDefinition::new("coarse");

/// What an online lookup of a place found: a position, or nothing (not asked again).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Looked {
    pub found: Option<Approx>,
    /// When it was looked up (epoch ms).
    pub at: i64,
}

/// A position given from a place's name, not taken where the photo was: it is shown
/// as approximate and never used to place other photos.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Coarse {
    pub lat: f64,
    pub lon: f64,
    pub level: Level,
}

impl Coarse {
    /// Whether the photo still sits where it was put: moved since, it is placed for real.
    pub fn holds(&self, lat: f64, lon: f64) -> bool {
        (self.lat - lat).abs() < 1e-6 && (self.lon - lon).abs() < 1e-6
    }
}

pub struct PlaceBook {
    db: Database,
}

impl PlaceBook {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let db = Database::create(path).with_context(|| format!("opening places {}", path.display()))?;
        let tx = db.begin_write()?;
        tx.open_table(ONLINE)?;
        tx.open_table(COARSE)?;
        tx.commit()?;
        Ok(Self { db })
    }

    pub fn online(&self) -> anyhow::Result<HashMap<String, Looked>> {
        let tx = self.db.begin_read()?;
        let mut out = HashMap::new();
        for entry in tx.open_table(ONLINE)?.iter()? {
            let (key, bytes) = entry?;
            if let Ok(looked) = serde_json::from_slice(bytes.value()) {
                out.insert(key.value().to_owned(), looked);
            }
        }
        Ok(out)
    }

    pub fn put_online(&self, items: &[(String, Looked)]) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(ONLINE)?;
            for (path, looked) in items {
                table.insert(path.as_str(), serde_json::to_vec(looked)?.as_slice())?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn coarse(&self) -> anyhow::Result<HashMap<String, Coarse>> {
        let tx = self.db.begin_read()?;
        let mut out = HashMap::new();
        for entry in tx.open_table(COARSE)?.iter()? {
            let (key, bytes) = entry?;
            if let Ok(coarse) = serde_json::from_slice(bytes.value()) {
                out.insert(key.value().to_owned(), coarse);
            }
        }
        Ok(out)
    }

    /// Sets (`Some`) or clears (`None`) photos' coarse positions, by photo key.
    pub fn set_coarse(&self, items: &[(String, Option<Coarse>)]) -> anyhow::Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(COARSE)?;
            for (key, coarse) in items {
                match coarse {
                    Some(c) => {
                        table.insert(key.as_str(), serde_json::to_vec(c)?.as_slice())?;
                    }
                    None => {
                        table.remove(key.as_str())?;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
}
