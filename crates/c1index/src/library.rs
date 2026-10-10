//! The library: a store and a search index kept in step, fed by [`PhotoSource`]s.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Instant;

use anyhow::{Context, bail};
use c1model::{Collection, Cursor, Photo, PhotoId, PhotoSource, SourceError, SourceEvent};
use serde::{Deserialize, Serialize};
use tantivy::collector::{Count, TopDocs};
use tantivy::directory::MmapDirectory;
use tantivy::query::{AllQuery, Query};
use tantivy::{DocAddress, Index, IndexReader, IndexWriter, Order, ReloadPolicy, Searcher, Term};

use crate::facets::{Counts, FacetCollector};
use crate::filter::{Distinct, Filter};
use crate::schema::{self, Dim, Fields, SCHEMA_VERSION};
use crate::store::Store;

/// Dimensions whose values substring filters match against.
const SUBSTRING_DIMS: [Dim; 6] = [Dim::Camera, Dim::Lens, Dim::City, Dim::State, Dim::Country, Dim::Style];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    Date,
    #[default]
    DateDesc,
    Rating,
    RatingDesc,
    Imported,
    ImportedDesc,
    Filename,
    FilenameDesc,
    Row,
}

impl Sort {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        Ok(match text.trim_start_matches('-') {
            "date" if text.starts_with('-') => Sort::DateDesc,
            "date" => Sort::Date,
            "date-desc" => Sort::DateDesc,
            "rating" if text.starts_with('-') => Sort::RatingDesc,
            "rating" => Sort::Rating,
            "rating-desc" => Sort::RatingDesc,
            "imported" if text.starts_with('-') => Sort::ImportedDesc,
            "imported" => Sort::Imported,
            "imported-desc" => Sort::ImportedDesc,
            "filename" if text.starts_with('-') => Sort::FilenameDesc,
            "filename" => Sort::Filename,
            "filename-desc" => Sort::FilenameDesc,
            "id" | "row" => Sort::Row,
            other => bail!("unknown sort {other:?}"),
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QueryOptions {
    pub sort: Sort,
    /// `None` returns every match.
    pub limit: Option<usize>,
    pub offset: usize,
    /// Dimensions to count over the matches.
    pub facets: Vec<Dim>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct QueryResult {
    /// How many photos matched, regardless of limit.
    pub total: usize,
    pub rows: Vec<u64>,
    pub facets: Counts,
    pub micros: u128,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SyncReport {
    pub full: bool,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub deleted: usize,
    pub millis: u128,
}

pub struct Library {
    dir: PathBuf,
    store: Store,
    index: Index,
    fields: Fields,
    reader: IndexReader,
    /// row → photo key, for turning results into ids without touching the store.
    keys: RwLock<Vec<Option<String>>>,
    distinct: RwLock<Distinct>,
}

fn source_err(e: anyhow::Error) -> SourceError {
    SourceError::Other(format!("{e:#}"))
}

impl Library {
    pub fn open(dir: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let store = Store::open(&dir.join("store.redb"))?;
        let index_dir = dir.join("index");

        let current = store.meta("index_schema")?.and_then(|v| v.parse::<u32>().ok());
        let rebuild = current != Some(SCHEMA_VERSION);
        if rebuild && index_dir.exists() {
            std::fs::remove_dir_all(&index_dir)?;
        }
        std::fs::create_dir_all(&index_dir)?;
        let (schema, fields) = schema::build_schema();
        let mut index = Index::open_or_create(MmapDirectory::open(&index_dir)?, schema)?;
        // Search segments in parallel: queries matching most of the library are bound by
        // counting facets, which splits cleanly across segments.
        index.set_default_multithread_executor()?;
        schema::register_tokenizers(&index);
        let reader = index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into()?;

        let library = Self {
            dir,
            store,
            index,
            fields,
            reader,
            keys: RwLock::new(Vec::new()),
            distinct: RwLock::new(Distinct::new()),
        };
        if rebuild {
            library.reindex()?;
        }
        library.refresh()?;
        Ok(library)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn writer(&self) -> anyhow::Result<IndexWriter> {
        Ok(self.index.writer(128 * 1024 * 1024)?)
    }

    /// Rebuilds the search index from the store (after a schema change).
    pub fn reindex(&self) -> anyhow::Result<()> {
        let mut writer = self.writer()?;
        writer.delete_all_documents()?;
        self.store.for_each_photo(|row, photo| {
            writer.add_document(schema::document(&self.fields, row, &photo))?;
            Ok(())
        })?;
        writer.commit()?;
        let mut batch = self.store.batch()?;
        batch.set_meta("index_schema", &SCHEMA_VERSION.to_string())?;
        batch.commit()
    }

    /// Reloads the searcher and the in-memory lookups after a commit.
    fn refresh(&self) -> anyhow::Result<()> {
        self.reader.reload()?;
        let mut keys = Vec::new();
        for (row, key) in self.store.keys()? {
            let row = row as usize;
            if keys.len() <= row {
                keys.resize(row + 1, None);
            }
            keys[row] = Some(key);
        }
        *self.keys.write().unwrap() = keys;

        let searcher = self.reader.searcher();
        let counts = searcher.search(&AllQuery, &FacetCollector::new(&SUBSTRING_DIMS))?;
        *self.distinct.write().unwrap() =
            counts.into_iter().map(|(dim, values)| (dim, values.into_iter().map(|(v, _)| v).collect())).collect();
        Ok(())
    }

    /// Pulls a source's current state into the library.
    pub fn sync(&self, source: &dyn PhotoSource) -> anyhow::Result<SyncReport> {
        let started = Instant::now();
        let source_id = source.info().id.0;
        let cursor_key = format!("cursor:{source_id}");
        let since = self.store.meta(&cursor_key)?.map(Cursor);
        let known: Vec<(u64, String)> = self.store.keys()?;

        let mut batch = self.store.batch()?;
        let mut writer = self.writer()?;
        let mut report = SyncReport::default();
        let mut seen: HashSet<u64> = HashSet::new();
        let mut end: Option<Cursor> = None;
        let row_field = self.fields.row;

        source.snapshot(since.as_ref(), &mut |event| {
            match event {
                SourceEvent::BeginFull => report.full = true,
                SourceEvent::BeginIncremental => {}
                SourceEvent::Upsert(photo) => {
                    let key = photo.id.key();
                    let fingerprint = photo.fingerprint();
                    let row = match batch.row_of(&key).map_err(source_err)? {
                        Some(row) if batch.fingerprint(row).map_err(source_err)? == Some(fingerprint) => {
                            report.unchanged += 1;
                            seen.insert(row);
                            return Ok(());
                        }
                        Some(row) => {
                            writer.delete_term(Term::from_field_u64(row_field, row));
                            report.updated += 1;
                            row
                        }
                        None => {
                            report.added += 1;
                            batch.next_row().map_err(source_err)?
                        }
                    };
                    seen.insert(row);
                    batch.put_photo(row, &photo, fingerprint).map_err(source_err)?;
                    writer
                        .add_document(schema::document(&self.fields, row, &photo))
                        .map_err(|e| SourceError::Other(e.to_string()))?;
                }
                SourceEvent::Delete(id) => {
                    let key = id.key();
                    if let Some(row) = batch.row_of(&key).map_err(source_err)? {
                        writer.delete_term(Term::from_field_u64(row_field, row));
                        batch.delete_photo(row, &key).map_err(source_err)?;
                        report.deleted += 1;
                    }
                }
                SourceEvent::Collections(collections) => {
                    batch.put_collections(&source_id, &collections).map_err(source_err)?;
                }
                SourceEvent::Keywords(keywords) => {
                    batch.put_keywords(&source_id, &keywords).map_err(source_err)?;
                }
                SourceEvent::End(cursor) => end = Some(cursor),
            }
            Ok(())
        })?;

        let Some(end) = end else { bail!("source {source_id} ended its snapshot without a cursor") };
        if report.full {
            // Anything of this source not seen in a full snapshot is gone.
            let prefix = format!("{source_id}\u{1f}");
            for (row, key) in known.iter().filter(|(row, key)| key.starts_with(&prefix) && !seen.contains(row)) {
                writer.delete_term(Term::from_field_u64(row_field, *row));
                batch.delete_photo(*row, key)?;
                report.deleted += 1;
            }
        }
        batch.set_meta(&cursor_key, &end.0)?;
        batch.set_meta("index_schema", &SCHEMA_VERSION.to_string())?;

        // Index first: if we crash between the two commits the store still holds the old
        // fingerprints, so the next sync redoes the work instead of losing it.
        writer.commit()?;
        batch.commit()?;
        self.refresh()?;
        report.millis = started.elapsed().as_millis();
        Ok(report)
    }

    pub fn searcher(&self) -> Searcher {
        self.reader.searcher()
    }

    pub fn len(&self) -> u64 {
        self.reader.searcher().num_docs()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn query(&self, filter: &Filter, options: &QueryOptions) -> anyhow::Result<QueryResult> {
        let started = Instant::now();
        let query = filter.to_query(&self.index, &self.fields, &self.distinct.read().unwrap())?;
        let searcher = self.reader.searcher();
        let mut result = self.run(&searcher, query.as_ref(), options)?;
        result.micros = started.elapsed().as_micros();
        Ok(result)
    }

    fn run(&self, searcher: &Searcher, query: &dyn Query, options: &QueryOptions) -> anyhow::Result<QueryResult> {
        let facets = FacetCollector::new(&options.facets);
        let limit = options.limit.unwrap_or(searcher.num_docs() as usize);
        if limit == 0 || searcher.num_docs() == 0 {
            let (total, facets) = searcher.search(query, &(Count, facets))?;
            return Ok(QueryResult { total, facets, ..QueryResult::default() });
        }
        let top = TopDocs::with_limit(limit).and_offset(options.offset);
        fn addrs<T>(hits: Vec<(T, DocAddress)>) -> Vec<DocAddress> {
            hits.into_iter().map(|(_, a)| a).collect()
        }
        let (total, docs, facets) = match options.sort {
            Sort::Date | Sort::DateDesc => {
                let order = if options.sort == Sort::Date { Order::Asc } else { Order::Desc };
                let (n, hits, f) =
                    searcher.search(query, &(Count, top.order_by_fast_field::<i64>("captured_sort", order), facets))?;
                (n, addrs(hits), f)
            }
            Sort::Imported | Sort::ImportedDesc => {
                let order = if options.sort == Sort::Imported { Order::Asc } else { Order::Desc };
                let (n, hits, f) =
                    searcher.search(query, &(Count, top.order_by_fast_field::<i64>("imported", order), facets))?;
                (n, addrs(hits), f)
            }
            Sort::Rating | Sort::RatingDesc => {
                let order = if options.sort == Sort::Rating { Order::Asc } else { Order::Desc };
                let (n, hits, f) =
                    searcher.search(query, &(Count, top.order_by_fast_field::<u64>("rating", order), facets))?;
                (n, addrs(hits), f)
            }
            Sort::Filename | Sort::FilenameDesc => {
                let order = if options.sort == Sort::Filename { Order::Asc } else { Order::Desc };
                let (n, hits, f) =
                    searcher.search(query, &(Count, top.order_by_string_fast_field("filename", order), facets))?;
                (n, addrs(hits), f)
            }
            Sort::Row => {
                let (n, hits, f) =
                    searcher.search(query, &(Count, top.order_by_fast_field::<u64>("row", Order::Asc), facets))?;
                (n, addrs(hits), f)
            }
        };
        Ok(QueryResult { total, rows: self.rows_of(searcher, &docs)?, facets, micros: 0 })
    }

    fn rows_of(&self, searcher: &Searcher, docs: &[DocAddress]) -> anyhow::Result<Vec<u64>> {
        let mut columns = Vec::new();
        for segment in searcher.segment_readers() {
            columns.push(segment.fast_fields().u64("row")?);
        }
        Ok(docs.iter().filter_map(|a| columns[a.segment_ord as usize].first(a.doc_id)).collect())
    }

    pub fn photo_id(&self, row: u64) -> Option<PhotoId> {
        let keys = self.keys.read().unwrap();
        keys.get(row as usize)?.as_deref().and_then(PhotoId::from_key)
    }

    pub fn photo(&self, row: u64) -> anyhow::Result<Option<Photo>> {
        self.store.photo(row)
    }

    pub fn collections(&self) -> anyhow::Result<Vec<Collection>> {
        self.store.collections()
    }

    pub fn keywords(&self) -> anyhow::Result<Vec<c1model::KeywordNode>> {
        self.store.keywords()
    }

    /// `#875`, an exact name, a case-insensitive name, or a unique path suffix — the
    /// rules the Python `--album` used.
    pub fn resolve_collection(&self, name: &str) -> anyhow::Result<String> {
        if let Some(pk) = name.strip_prefix('#') {
            return Ok(pk.to_owned());
        }
        let all = self.collections()?;
        let lower = name.to_lowercase();
        let tries: [&dyn Fn(&Collection) -> bool; 3] =
            [&|c| c.name == name, &|c| c.name.to_lowercase() == lower, &|c| c.path.to_lowercase().ends_with(&lower)];
        for matches in tries {
            let found: Vec<&Collection> = all.iter().filter(|c| matches(c)).collect();
            match found.as_slice() {
                [] => continue,
                [one] => return Ok(one.id.0.clone()),
                many => bail!(
                    "{name:?} matches several collections, use #id: {}",
                    many.iter().map(|c| format!("#{} {}", c.id.0, c.path)).collect::<Vec<_>>().join(", ")
                ),
            }
        }
        bail!("no collection named {name:?}")
    }
}
