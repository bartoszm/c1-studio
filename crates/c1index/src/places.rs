//! Where places are: what the gazetteer has no answer for, as an online lookup found it,
//! and which photos were given a position that is only a guess.
//!
//! Kept in its own file (`places.redb`), like trips: it outlives the store, which is wiped
//! when the model changes.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use c1geo::{Approx, Level};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A place's path, as [`crate::schema::place_tree`] writes it → a looked-up [`Looked`].
const ONLINE: TableDefinition<&str, &[u8]> = TableDefinition::new("online");
/// A photo key → the [`Coarse`] position it was given.
const COARSE: TableDefinition<&str, &[u8]> = TableDefinition::new("coarse");
/// A photo key → the [`Guessed`] position its neighbours in time gave it.
const GUESSED: TableDefinition<&str, &[u8]> = TableDefinition::new("guessed");

/// What an online lookup of a place found: a position, or nothing (not asked again).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Looked {
    pub found: Option<Approx>,
    /// When it was looked up (epoch ms).
    pub at: i64,
}

/// Whether a photo still sits where it was put: moved since, it is placed for real.
pub(crate) fn same_spot(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

/// A position given from a place's name, not taken where the photo was: it is shown
/// as approximate and never used to place other photos.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Coarse {
    pub lat: f64,
    pub lon: f64,
    pub level: Level,
    /// The place it was given from, as [`crate::schema::place_tree`] writes it.
    #[serde(default)]
    pub path: String,
}

impl Coarse {
    pub fn holds(&self, lat: f64, lon: f64) -> bool {
        same_spot((self.lat, self.lon), (lat, lon))
    }
}

/// A position worked out from the photos taken just before and after: shown as any
/// other, but never used to place more photos (a guess would spread from guess to guess).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guessed {
    pub lat: f64,
    pub lon: f64,
}

impl Guessed {
    pub fn holds(&self, lat: f64, lon: f64) -> bool {
        same_spot((self.lat, self.lon), (lat, lon))
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
        tx.open_table(GUESSED)?;
        tx.commit()?;
        Ok(Self { db })
    }

    /// Every entry of a table this build can read.
    fn all<T: DeserializeOwned>(&self, table: TableDefinition<&str, &[u8]>) -> anyhow::Result<HashMap<String, T>> {
        let tx = self.db.begin_read()?;
        let mut out = HashMap::new();
        for entry in tx.open_table(table)?.iter()? {
            let (key, bytes) = entry?;
            if let Ok(value) = serde_json::from_slice(bytes.value()) {
                out.insert(key.value().to_owned(), value);
            }
        }
        Ok(out)
    }

    /// Sets (`Some`) or clears (`None`) entries of a table.
    fn set<T: Serialize>(
        &self,
        table: TableDefinition<&str, &[u8]>,
        items: &[(String, Option<T>)],
    ) -> anyhow::Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(table)?;
            for (key, value) in items {
                match value {
                    Some(v) => {
                        table.insert(key.as_str(), serde_json::to_vec(v)?.as_slice())?;
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

    pub fn online(&self) -> anyhow::Result<HashMap<String, Looked>> {
        self.all(ONLINE)
    }

    pub fn put_online(&self, items: &[(String, Looked)]) -> anyhow::Result<()> {
        let items: Vec<(String, Option<Looked>)> = items.iter().map(|(k, v)| (k.clone(), Some(*v))).collect();
        self.set(ONLINE, &items)
    }

    pub fn coarse(&self) -> anyhow::Result<HashMap<String, Coarse>> {
        self.all(COARSE)
    }

    /// Sets (`Some`) or clears (`None`) photos' coarse positions, by photo key.
    pub fn set_coarse(&self, items: &[(String, Option<Coarse>)]) -> anyhow::Result<()> {
        self.set(COARSE, items)
    }

    pub fn guessed(&self) -> anyhow::Result<HashMap<String, Guessed>> {
        self.all(GUESSED)
    }

    pub fn set_guessed(&self, items: &[(String, Option<Guessed>)]) -> anyhow::Result<()> {
        self.set(GUESSED, items)
    }
}
