//! The library: a store and a search index kept in step, fed by [`PhotoSource`]s.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::Instant;

use anyhow::{Context, bail};
use c1model::change::{Check, diff};
use c1model::{
    Change, Collection, Condition, Cursor, MODEL_VERSION, Op, Photo, PhotoId, PhotoSource, SmartRule, SourceError,
    SourceEvent,
};
use serde::{Deserialize, Serialize};
use tantivy::collector::{Count, TopDocs};
use tantivy::directory::MmapDirectory;
use tantivy::query::{AllQuery, Query};
use tantivy::{DocAddress, Index, IndexReader, IndexWriter, Order, ReloadPolicy, Searcher, Term};

use crate::edits::{self, Edits, Event, Origin, Pending, now_ms, pending_key};
use crate::facets::{Counts, FacetCollector};
use crate::filter::{Distinct, Filter, Presence, Range, Trashed};
use crate::schema::{self, Dim, Fields, SCHEMA_VERSION};
use crate::store::Store;

/// Dimensions whose values substring filters match against.
const SUBSTRING_DIMS: [Dim; 7] =
    [Dim::Camera, Dim::Lens, Dim::City, Dim::State, Dim::Country, Dim::Style, Dim::KeywordTree];

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
    /// Count each dimension as if the filter did not pick values of it (see
    /// [`Filter::without`]), so a sidebar group keeps showing its alternatives.
    pub facets_ignore_own: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct QueryResult {
    /// How many photos matched, regardless of limit.
    pub total: usize,
    pub rows: Vec<u64>,
    pub facets: Counts,
    pub micros: u128,
}

/// A queued edit, with the row of its photo if the library still has it.
#[derive(Clone, Debug, Serialize)]
pub struct PendingEdit {
    pub key: String,
    pub row: Option<u64>,
    /// The photo's file name, for showing the edit.
    pub name: Option<String>,
    #[serde(flatten)]
    pub pending: Pending,
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
    /// Smart album id → its rule, for expanding a filter that picks one.
    smart: RwLock<HashMap<String, SmartRule>>,
    /// Edits made here and not yet written to their source, and the change log.
    edits: Edits,
    /// One writer at a time: syncs and edits both rewrite documents.
    writing: Mutex<()>,
}

fn source_err(e: anyhow::Error) -> SourceError {
    SourceError::Other(format!("{e:#}"))
}

impl Library {
    pub fn open(dir: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let store_path = dir.join("store.redb");
        let index_dir = dir.join("index");
        // A store written with another model cannot be read back. It only copies the
        // sources, so it is dropped and the next sync fills it again.
        let mut store = Store::open(&store_path)?;
        if store.meta("model")?.and_then(|v| v.parse::<u32>().ok()) != Some(MODEL_VERSION) {
            if !store.keys()?.is_empty() || store.meta("index_schema")?.is_some() {
                drop(store);
                std::fs::remove_file(&store_path)?;
                if index_dir.exists() {
                    std::fs::remove_dir_all(&index_dir)?;
                }
                store = Store::open(&store_path)?;
            }
            let mut batch = store.batch()?;
            batch.set_meta("model", &MODEL_VERSION.to_string())?;
            batch.commit()?;
        }

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
        let edits = Edits::open(&dir.join("edits.redb"))?;

        let library = Self {
            dir,
            store,
            index,
            fields,
            reader,
            keys: RwLock::new(Vec::new()),
            distinct: RwLock::new(Distinct::new()),
            smart: RwLock::new(HashMap::new()),
            edits,
            writing: Mutex::new(()),
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
        let _writing = self.writing.lock().unwrap();
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
        *self.smart.write().unwrap() =
            self.store.collections()?.into_iter().filter_map(|c| Some((c.id.0, c.rule?))).collect();
        Ok(())
    }

    /// Pulls a source's current state into the library.
    pub fn sync(&self, source: &dyn PhotoSource) -> anyhow::Result<SyncReport> {
        let _writing = self.writing.lock().unwrap();
        let started = Instant::now();
        // Queued edits stay on top of whatever the source sends.
        let mut queued: HashMap<String, Vec<(String, Pending)>> = HashMap::new();
        for (key, pending) in self.edits.pending()? {
            queued.entry(edits::photo_key(&key).to_owned()).or_default().push((key, pending));
        }
        let mut edits = self.edits.batch()?;
        let now = now_ms();
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
                SourceEvent::Upsert(mut photo) => {
                    let key = photo.id.key();
                    // The source's own state; queued edits laid over it below do not count.
                    let fingerprint = photo.fingerprint();
                    let (row, old) = match batch.row_of(&key).map_err(source_err)? {
                        Some(row) if batch.fingerprint(row).map_err(source_err)? == Some(fingerprint) => {
                            report.unchanged += 1;
                            seen.insert(row);
                            return Ok(());
                        }
                        Some(row) => {
                            writer.delete_term(Term::from_field_u64(row_field, row));
                            report.updated += 1;
                            (row, batch.photo(row).map_err(source_err)?)
                        }
                        None => {
                            report.added += 1;
                            (batch.next_row().map_err(source_err)?, None)
                        }
                    };
                    for (pending_key, mut pending) in queued.remove(&key).unwrap_or_default() {
                        match pending.change.op.check(&photo) {
                            // The source holds what the user wants: nothing left to write.
                            Check::NoOp => edits.remove(&pending_key).map_err(source_err)?,
                            Check::Apply => {
                                if pending.theirs.take().is_some() {
                                    edits.put(&pending_key, &pending).map_err(source_err)?;
                                }
                                pending.change.op.apply_to(&mut photo);
                            }
                            // Changed on both sides: keep the user's value until they choose.
                            Check::Conflict(_) => {
                                let theirs = pending.change.op.valued_from(&photo);
                                if pending.theirs.as_ref() != Some(&theirs) {
                                    pending.theirs = Some(theirs);
                                    edits.put(&pending_key, &pending).map_err(source_err)?;
                                }
                                pending.change.op.apply_to(&mut photo);
                            }
                        }
                    }
                    if let Some(old) = old {
                        for op in diff(&old, &photo) {
                            let change = Change { photo: photo.id.clone(), op };
                            edits.log(&Event { at: now, origin: Origin::Source, change }).map_err(source_err)?;
                        }
                    }
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
        edits.commit()?;
        self.refresh()?;
        report.millis = started.elapsed().as_millis();
        Ok(report)
    }

    /// Applies set-field edits (rating, colour, ...) to photos by row and queues them for
    /// their source. The library shows them at once. Returns, per row, the op that puts
    /// back what the photo showed before, for undo.
    pub fn edit(&self, changes: &[(u64, Op)]) -> anyhow::Result<Vec<(u64, Op)>> {
        let _writing = self.writing.lock().unwrap();
        let mut writer = self.writer()?;
        let mut batch = self.store.batch()?;
        let mut edits = self.edits.batch()?;
        let now = now_ms();
        let mut undo = Vec::new();
        for (row, op) in changes.iter().flat_map(|(row, op)| op.split().into_iter().map(move |op| (*row, op))) {
            let Some(what) = op.queue_key() else { bail!("{op:?} cannot be queued yet") };
            let Some(photo) = batch.photo(row)? else { continue };
            let mut after = photo.clone();
            op.apply_to(&mut after);
            if after == photo {
                continue;
            }
            // From what the photo shows now to what was asked.
            let shown = op.based_on(&photo);
            let key = pending_key(&photo.id, &what);
            let previous = edits.get(&key)?;
            // A field's base stays what the source held when it was first edited. A
            // keyword added then removed (or the other way round) is back where it was.
            let wanted = match &previous {
                Some(p) => p.change.op.retarget(&op),
                None => shown.clone(),
            };
            let undone = |p: &Pending| p.theirs.is_none() && (wanted.is_identity() || p.change.op.reverted() == op);
            match previous {
                Some(p) if undone(&p) => edits.remove(&key)?,
                previous => {
                    let theirs = previous.and_then(|p| p.theirs);
                    let change = Change { photo: photo.id.clone(), op: wanted };
                    edits.put(&key, &Pending { change, at: now, theirs })?;
                }
            }
            undo.push((row, shown.reverted()));
            edits.log(&Event {
                at: now,
                origin: Origin::App,
                change: Change { photo: photo.id.clone(), op: shown },
            })?;
            let fingerprint = batch.fingerprint(row)?.unwrap_or_default();
            batch.put_photo(row, &after, fingerprint)?;
            writer.delete_term(Term::from_field_u64(self.fields.row, row));
            writer.add_document(schema::document(&self.fields, row, &after))?;
        }
        writer.commit()?;
        batch.commit()?;
        edits.commit()?;
        self.reader.reload()?;
        Ok(undo)
    }

    /// Every queued edit, a photo's fields together.
    pub fn pending(&self) -> anyhow::Result<Vec<PendingEdit>> {
        self.edits
            .pending()?
            .into_iter()
            .map(|(key, pending)| {
                let row = self.store.row_of(edits::photo_key(&key))?;
                let name = match row {
                    Some(row) => self.store.photo(row)?.map(|p| p.file.name),
                    None => None,
                };
                Ok(PendingEdit { key, row, name, pending })
            })
            .collect()
    }

    /// The newest `limit` entries of the change log, newest first.
    pub fn events(&self, limit: usize) -> anyhow::Result<Vec<Event>> {
        self.edits.events(limit)
    }

    /// Drops queued edits and shows the source's value again: the one it held when the
    /// field was first edited, or its newer one if it changed meanwhile.
    pub fn discard(&self, keys: &[String]) -> anyhow::Result<()> {
        self.drop_queued(keys, |pending| pending.theirs.clone().unwrap_or_else(|| pending.change.op.reverted()))
    }

    /// For edits the source changed too: keep the user's value (it will be written over
    /// the source's) or take the source's (the edit is dropped).
    pub fn resolve(&self, keys: &[String], keep_mine: bool) -> anyhow::Result<()> {
        if !keep_mine {
            return self.discard(keys);
        }
        let _writing = self.writing.lock().unwrap();
        let mut edits = self.edits.batch()?;
        for key in keys {
            let Some(mut pending) = edits.get(key)? else { continue };
            if let Some(theirs) = pending.theirs.take() {
                // From the source's value now to the user's.
                pending.change.op = theirs.reverted().retarget(&pending.change.op);
                edits.put(key, &pending)?;
            }
        }
        edits.commit()
    }

    /// After a source took `written`: the queued edits it covers are done.
    pub fn settle(&self, written: &[Change]) -> anyhow::Result<()> {
        let _writing = self.writing.lock().unwrap();
        let mut edits = self.edits.batch()?;
        for change in written {
            for op in change.op.split() {
                let Some(what) = op.queue_key() else { continue };
                let key = pending_key(&change.photo, &what);
                let change = &Change { photo: change.photo.clone(), op };
                if edits.get(&key)?.is_some_and(|p| p.change == *change) {
                    edits.remove(&key)?;
                }
            }
        }
        edits.commit()
    }

    /// Removes queued edits and applies to each shown photo the op `restore` gives.
    fn drop_queued(&self, keys: &[String], restore: impl Fn(&Pending) -> Op) -> anyhow::Result<()> {
        let _writing = self.writing.lock().unwrap();
        let mut writer = self.writer()?;
        let mut batch = self.store.batch()?;
        let mut edits = self.edits.batch()?;
        let now = now_ms();
        for key in keys {
            let Some(pending) = edits.get(key)? else { continue };
            edits.remove(key)?;
            let Some(row) = batch.row_of(edits::photo_key(key))? else { continue };
            let Some(mut photo) = batch.photo(row)? else { continue };
            let op = restore(&pending);
            let shown = op.based_on(&photo);
            let mut after = photo.clone();
            op.apply_to(&mut after);
            if after == photo {
                continue;
            }
            edits.log(&Event {
                at: now,
                origin: Origin::App,
                change: Change { photo: photo.id.clone(), op: shown },
            })?;
            op.apply_to(&mut photo);
            let fingerprint = batch.fingerprint(row)?.unwrap_or_default();
            batch.put_photo(row, &photo, fingerprint)?;
            writer.delete_term(Term::from_field_u64(self.fields.row, row));
            writer.add_document(schema::document(&self.fields, row, &photo))?;
        }
        writer.commit()?;
        batch.commit()?;
        edits.commit()?;
        self.reader.reload()?;
        Ok(())
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
        let distinct = self.distinct.read().unwrap();
        let smart = self.smart.read().unwrap();
        let compile = |f: &Filter| expand(f, &smart).and_then(|f| f.to_query(&self.index, &self.fields, &distinct));
        let query = compile(filter)?;
        let searcher = self.reader.searcher();
        let mut result = self.run(&searcher, query.as_ref(), options)?;
        if options.facets_ignore_own {
            // One more counting pass per group the filter picks from; the rest are
            // already right.
            for &dim in &options.facets {
                let own = filter.without(dim);
                if own == *filter {
                    continue;
                }
                let query = compile(&own)?;
                let mut counts = searcher.search(query.as_ref(), &FacetCollector::new(&[dim]))?;
                result.facets.insert(dim, counts.remove(&dim).unwrap_or_default());
            }
        }
        result.micros = started.elapsed().as_micros();
        Ok(result)
    }

    /// How many photos each smart album holds among those `filter` selects, setting
    /// aside the albums it picks. Albums whose rule cannot be evaluated are left out.
    pub fn smart_counts(&self, filter: &Filter) -> anyhow::Result<Vec<(String, u64)>> {
        let distinct = self.distinct.read().unwrap();
        let smart = self.smart.read().unwrap();
        let searcher = self.reader.searcher();
        let base = filter.without(Dim::Collection);
        let mut out = Vec::new();
        for id in smart.iter().filter(|(_, rule)| rule.supported()).map(|(id, _)| id) {
            let f = Filter { collections: vec![id.clone()], ..base.clone() };
            let query = expand(&f, &smart)?.to_query(&self.index, &self.fields, &distinct)?;
            out.push((id.clone(), searcher.search(query.as_ref(), &Count)? as u64));
        }
        Ok(out)
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
                    searcher.search(query, &(Count, top.order_by_fast_field::<i64>("imported_sort", order), facets))?;
                (n, addrs(hits), f)
            }
            Sort::Rating | Sort::RatingDesc => {
                let order = if options.sort == Sort::Rating { Order::Asc } else { Order::Desc };
                let (n, hits, f) =
                    searcher.search(query, &(Count, top.order_by_fast_field::<u64>("rating_sort", order), facets))?;
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

/// Replaces smart albums picked in `filter.collections` (here and in nested filters)
/// with their rules.
fn expand(filter: &Filter, smart: &HashMap<String, SmartRule>) -> anyhow::Result<Filter> {
    let mut f = filter.clone();
    let mut rules = Vec::new();
    f.collections.retain(|id| match smart.get(id) {
        Some(rule) => {
            rules.push((id.clone(), rule));
            false
        }
        None => true,
    });
    for (id, rule) in rules {
        f.all_of.push(smart_filter(rule).with_context(|| format!("smart album #{id}"))?);
    }
    f.all_of = f.all_of.iter().map(|s| expand(s, smart)).collect::<anyhow::Result<_>>()?;
    f.any_of = f.any_of.iter().map(|s| expand(s, smart)).collect::<anyhow::Result<_>>()?;
    Ok(f)
}

/// A smart album's rule as a filter. Trashed photos are left to the outer filter.
pub fn smart_filter(rule: &SmartRule) -> anyhow::Result<Filter> {
    let mut parts = Vec::new();
    for condition in &rule.conditions {
        let mut f = Filter { trashed: Trashed::Include, ..Filter::default() };
        match condition {
            Condition::KeywordContains(text) => f.keywords_containing = vec![text.clone()],
            Condition::Rating { min, max } => {
                let range = Range { min: Some(f64::from(*min)), max: Some(f64::from(*max)), ..Range::default() };
                f.rating = Some(Presence::Values(range));
            }
            Condition::Year(year) => f.year = Some(*year),
            Condition::Unsupported(what) => bail!("cannot evaluate the condition {what}"),
        }
        parts.push(f);
    }
    let mut f = Filter { trashed: Trashed::Include, ..Filter::default() };
    if rule.match_all {
        f.all_of = parts;
    } else {
        f.any_of = parts;
    }
    Ok(f)
}
