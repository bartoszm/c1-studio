//! What each photo looks like (stage 3 analysis), kept apart from the store: it is
//! measured from pixels, slow to make again, and must survive the store being rebuilt.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use c1analysis::{Analysis, ClassSettings};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

/// photo key → postcard [`Record`].
const ANALYSES: TableDefinition<&str, &[u8]> = TableDefinition::new("analyses");
/// name → value, for the class settings.
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");
const SETTINGS: &str = "class_settings";

/// An analysis as stored, with what it was measured from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// [`c1analysis::VERSION`] when measured.
    pub version: u32,
    /// Size and modification time of the image measured: a new render is measured again.
    pub stamp: (u64, u64),
    pub analysis: Analysis,
}

/// A photo's analysis, and the duplicate group and series it falls in (`dup:<row>`,
/// `series:<row>`, named after their first photo's row).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Look {
    pub analysis: Analysis,
    #[serde(skip)]
    pub stamp: (u64, u64),
    pub duplicates: Option<String>,
    pub series: Option<String>,
    /// Not the sharpest of its group (see [`crate::filter::Filter::stacked`]).
    #[serde(skip)]
    pub stacked: bool,
}

pub struct Looks {
    db: Database,
}

impl Looks {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let db = Database::create(path).with_context(|| format!("opening {}", path.display()))?;
        let tx = db.begin_write()?;
        tx.open_table(ANALYSES)?;
        tx.open_table(META)?;
        tx.commit()?;
        Ok(Self { db })
    }

    /// Where this library draws the lines between classes; the defaults until changed.
    pub fn settings(&self) -> anyhow::Result<ClassSettings> {
        let tx = self.db.begin_read()?;
        let meta = tx.open_table(META)?;
        Ok(match meta.get(SETTINGS)? {
            Some(json) => serde_json::from_str::<ClassSettings>(json.value()).unwrap_or_default().sanitized(),
            None => ClassSettings::default(),
        })
    }

    pub fn set_settings(&self, settings: &ClassSettings) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        tx.open_table(META)?.insert(SETTINGS, serde_json::to_string(settings)?.as_str())?;
        tx.commit()?;
        Ok(())
    }

    /// Every stored analysis of the current version, by photo key.
    pub fn all(&self) -> anyhow::Result<HashMap<String, Record>> {
        let tx = self.db.begin_read()?;
        let mut out = HashMap::new();
        for entry in tx.open_table(ANALYSES)?.iter()? {
            let (key, bytes) = entry?;
            // One that no longer reads is measured again.
            if let Ok(record) = postcard::from_bytes::<Record>(bytes.value())
                && record.version == c1analysis::VERSION
            {
                out.insert(key.value().to_owned(), record);
            }
        }
        Ok(out)
    }

    pub fn put(&self, records: &[(String, Record)]) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(ANALYSES)?;
            for (key, record) in records {
                table.insert(key.as_str(), postcard::to_stdvec(record)?.as_slice())?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
