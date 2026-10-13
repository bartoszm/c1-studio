//! The library: a store and a search index kept in step, fed by [`PhotoSource`]s.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::Instant;

use anyhow::{Context, bail};
use c1model::change::{Check, collection_problem, descendants, diff, membership_problem};
use c1model::{
    Change, Collection, CollectionChange, CollectionId, Condition, Cursor, KeywordNode, KeywordPath, KeywordRemap,
    MODEL_VERSION, Op, Photo, PhotoId, PhotoSource, SmartRule, SourceError, SourceEvent,
};
use serde::{Deserialize, Serialize};
use tantivy::collector::{Count, DocSetCollector, TopDocs};
use tantivy::directory::MmapDirectory;
use tantivy::query::{AllQuery, Query};
use tantivy::{DocAddress, Index, IndexReader, IndexWriter, Order, ReloadPolicy, Searcher, Term};

use crate::edits::{self, Edits, Event, Origin, Pending, QueuedCollection, QueuedRemap, now_ms, pending_key};
use crate::facets::{Counts, FacetCollector};
use crate::filter::{Distinct, Filter, Presence, Range, Trashed};
use crate::geo::{self, Located, MapResult, MapView};
use crate::looks::{Look, Looks, Record};
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

/// Photos shown for a group of alike photos.
const GROUP_SHOWN: usize = 6;

/// A duplicate group or series: its id (a [`Filter::group`] value), how many of its photos
/// the filter selects, and the first few.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Group {
    pub id: String,
    pub size: usize,
    pub rows: Vec<u64>,
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
    /// What photos look like (stage 3), stored, and in memory by photo key with the
    /// duplicate groups and series they fall in.
    looks: Looks,
    seen: RwLock<HashMap<String, Look>>,
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

        let rebuild = store.meta("index_schema")? != Some(index_version());
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
        let looks = Looks::open(&dir.join("analysis.redb"))?;
        let seen = looks
            .all()?
            .into_iter()
            .map(|(key, r)| (key, Look { analysis: r.analysis, stamp: r.stamp, duplicates: None, series: None }))
            .collect();

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
            looks,
            seen: RwLock::new(seen),
            writing: Mutex::new(()),
        };
        // Groups as worked out now: the index is told of any that differ from last time.
        let regrouped = library.group_looks()?;
        if !rebuild {
            library.reindex_keys(&regrouped)?;
        }
        if rebuild {
            library.reindex()?;
        }
        library.refresh()?;
        Ok(library)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A photo's search document, with what its analysis found.
    fn doc(&self, row: u64, photo: &Photo) -> tantivy::TantivyDocument {
        let seen = self.seen.read().unwrap();
        schema::document(&self.fields, row, photo, seen.get(&photo.id.key()))
    }

    /// Every measured photo's analysis and groups.
    pub fn looks(&self) -> Vec<Look> {
        self.seen.read().unwrap().values().cloned().collect()
    }

    /// What the analysis found for a photo, if it has been measured.
    /// The duplicate groups (or series) among the photos `filter` selects, biggest first:
    /// each with how many of its photos match and a few of them to show.
    pub fn groups(&self, filter: &Filter, series: bool, limit: usize) -> anyhow::Result<Vec<Group>> {
        let narrowed =
            Filter { duplicates: filter.duplicates || !series, series: filter.series || series, ..filter.clone() };
        let rows = self.query(&narrowed, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows;
        let mut by: HashMap<String, Group> = HashMap::new();
        for row in rows {
            let Some(look) = self.look(row) else { continue };
            let Some(id) = (if series { look.series } else { look.duplicates }) else { continue };
            let group = by.entry(id.clone()).or_insert_with(|| Group { id, size: 0, rows: Vec::new() });
            group.size += 1;
            if group.rows.len() < GROUP_SHOWN {
                group.rows.push(row);
            }
        }
        let mut out: Vec<Group> = by.into_values().collect();
        out.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.id.cmp(&b.id)));
        out.truncate(limit);
        Ok(out)
    }

    pub fn look(&self, row: u64) -> Option<Look> {
        let key = self.keys.read().unwrap().get(row as usize)?.clone()?;
        self.seen.read().unwrap().get(&key).cloned()
    }

    /// Every photo's measured image (size, modification time), by key: a photo whose
    /// image is the same need not be measured again.
    pub fn analysed(&self) -> HashMap<String, (u64, u64)> {
        self.seen.read().unwrap().iter().map(|(k, l)| (k.clone(), l.stamp)).collect()
    }

    /// Stores analyses and shows them at once (the groups follow with
    /// [`Library::regroup`]).
    pub fn put_analyses(&self, batch: Vec<(String, (u64, u64), c1analysis::Analysis)>) -> anyhow::Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let records: Vec<(String, Record)> = batch
            .iter()
            .map(|(k, stamp, a)| {
                (k.clone(), Record { version: c1analysis::VERSION, stamp: *stamp, analysis: a.clone() })
            })
            .collect();
        self.looks.put(&records)?;
        let _writing = self.writing.lock().unwrap();
        {
            let mut seen = self.seen.write().unwrap();
            for (key, stamp, analysis) in batch.iter().cloned() {
                let old = seen.remove(&key);
                let (duplicates, series) = old.map(|o| (o.duplicates, o.series)).unwrap_or_default();
                seen.insert(key, Look { analysis, stamp, duplicates, series });
            }
        }
        let keys: Vec<String> = batch.into_iter().map(|(k, _, _)| k).collect();
        self.reindex_keys(&keys)
    }

    /// Works the duplicate groups and series out again over every measured photo, and
    /// re-indexes the photos whose groups changed. Returns how many did.
    pub fn regroup(&self) -> anyhow::Result<usize> {
        let _writing = self.writing.lock().unwrap();
        let changed = self.group_looks()?;
        self.reindex_keys(&changed)?;
        Ok(changed.len())
    }

    /// Sets each measured photo's groups; returns the keys whose groups changed.
    fn group_looks(&self) -> anyhow::Result<Vec<String>> {
        let mut members: Vec<(u64, String, c1analysis::Taken)> = Vec::new();
        {
            let seen = self.seen.read().unwrap();
            if seen.is_empty() {
                return Ok(Vec::new());
            }
            for (row, key) in self.store.keys()? {
                let Some(look) = seen.get(&key) else { continue };
                let Some(photo) = self.store.photo(row)? else { continue };
                if photo.trashed {
                    continue;
                }
                let taken = c1analysis::Taken {
                    asset: photo.asset.0.clone(),
                    camera: photo.exif.camera_model.clone(),
                    at: photo.captured_at,
                    hash: look.analysis.hash,
                };
                members.push((row, key, taken));
            }
        }
        let taken: Vec<c1analysis::Taken> = members.iter().map(|(_, _, t)| t.clone()).collect();
        let groups = c1analysis::group(&taken);
        let mut wanted: HashMap<&str, (Option<String>, Option<String>)> = HashMap::new();
        for (prefix, list) in [("dup", &groups.duplicates), ("series", &groups.series)] {
            for group in list {
                let name = format!("{prefix}:{}", group.iter().map(|&i| members[i].0).min().unwrap_or(0));
                for &i in group {
                    let slot = wanted.entry(members[i].1.as_str()).or_default();
                    if prefix == "dup" {
                        slot.0 = Some(name.clone());
                    } else {
                        slot.1 = Some(name.clone());
                    }
                }
            }
        }
        let mut changed = Vec::new();
        let mut seen = self.seen.write().unwrap();
        for (key, look) in seen.iter_mut() {
            let (duplicates, series) = wanted.remove(key.as_str()).unwrap_or_default();
            if look.duplicates != duplicates || look.series != series {
                look.duplicates = duplicates;
                look.series = series;
                changed.push(key.clone());
            }
        }
        Ok(changed)
    }

    /// Writes the documents of these photos again. The caller holds `writing`.
    fn reindex_keys(&self, keys: &[String]) -> anyhow::Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let mut writer = self.writer()?;
        for key in keys {
            let Some(row) = self.store.row_of(key)? else { continue };
            let Some(photo) = self.store.photo(row)? else { continue };
            writer.delete_term(Term::from_field_u64(self.fields.row, row));
            writer.add_document(self.doc(row, &photo))?;
        }
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
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
            writer.add_document(self.doc(row, &photo))?;
            Ok(())
        })?;
        writer.commit()?;
        let mut batch = self.store.batch()?;
        batch.set_meta("index_schema", &index_version())?;
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
        self.refresh_distinct()?;
        self.refresh_smart()
    }

    /// The smart albums' rules, after albums changed (here or in a source).
    fn refresh_smart(&self) -> anyhow::Result<()> {
        *self.smart.write().unwrap() =
            self.collections()?.into_iter().filter_map(|c| Some((c.id.0, c.rule?))).collect();
        Ok(())
    }

    /// The values substring filters match against, after keywords (or places) changed.
    fn refresh_distinct(&self) -> anyhow::Result<()> {
        let searcher = self.reader.searcher();
        let counts = searcher.search(&AllQuery, &FacetCollector::new(&SUBSTRING_DIMS))?;
        *self.distinct.write().unwrap() =
            counts.into_iter().map(|(dim, values)| (dim, values.into_iter().map(|(v, _)| v).collect())).collect();
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
        let remaps: Vec<KeywordRemap> = self.edits.remaps()?.into_iter().map(|(_, q)| q.remap).collect();
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
                    for remap in &remaps {
                        remap.apply(&mut photo.keywords);
                    }
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
                    writer.add_document(self.doc(row, &photo)).map_err(|e| SourceError::Other(e.to_string()))?;
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
        batch.set_meta("index_schema", &index_version())?;

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
        self.edit_made(|_| Ok(changes.to_vec()))
    }

    /// Like [`edit`](Self::edit), with each photo's ops made from the photo as it is when
    /// the edit runs. Ops made from an earlier read could undo what was edited since: a
    /// location typed while the city was still being saved would set the city back.
    pub fn edit_photos(
        &self,
        rows: &[u64],
        ops: impl Fn(u64, &Photo) -> anyhow::Result<Vec<Op>>,
    ) -> anyhow::Result<Vec<(u64, Op)>> {
        self.edit_made(|batch| {
            let mut changes = Vec::new();
            for &row in rows {
                if let Some(photo) = batch.photo(row)? {
                    changes.extend(ops(row, &photo)?.into_iter().map(|op| (row, op)));
                }
            }
            Ok(changes)
        })
    }

    fn edit_made(
        &self,
        make: impl FnOnce(&crate::store::Batch) -> anyhow::Result<Vec<(u64, Op)>>,
    ) -> anyhow::Result<Vec<(u64, Op)>> {
        let _writing = self.writing.lock().unwrap();
        let mut writer = self.writer()?;
        let mut batch = self.store.batch()?;
        let changes = &make(&batch)?;
        // Only albums take photos; a smart album's come from its rule.
        if changes.iter().any(|(_, op)| membership(op).is_some()) {
            let albums = self.collections()?;
            for id in changes.iter().filter_map(|(_, op)| membership(op)) {
                if let Some(why) = membership_problem(&albums, id) {
                    bail!("{why}");
                }
            }
        }
        let mut edits = self.edits.batch()?;
        let now = now_ms();
        let mut undo = Vec::new();
        // One op per keyword. Removing a keyword takes everything under it too (removing
        // `animal` takes `animal|dog`), each queued on its own so undo puts back exactly
        // what went.
        let mut each: Vec<(u64, Op)> = Vec::new();
        for (row, op) in changes {
            match op {
                Op::RemoveKeywords { paths } => {
                    let Some(photo) = batch.photo(*row)? else { continue };
                    for k in photo.keywords.iter().filter(|k| paths.iter().any(|p| k.0.starts_with(&p.0))) {
                        each.push((*row, Op::RemoveKeywords { paths: vec![k.clone()] }));
                    }
                }
                other => each.extend(other.split().into_iter().map(|op| (*row, op))),
            }
        }
        for (row, op) in each {
            let Some(what) = op.queue_key() else { bail!("{op:?} cannot be queued yet") };
            let Some(photo) = batch.photo(row)? else { continue };
            let mut after = photo.clone();
            op.apply_to(&mut after);
            if after == photo || op.based_on(&photo).is_identity() {
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
            writer.add_document(self.doc(row, &after))?;
        }
        writer.commit()?;
        batch.commit()?;
        edits.commit()?;
        self.reader.reload()?;
        if undo
            .iter()
            .any(|(_, op)| matches!(op, Op::AddKeywords { .. } | Op::RemoveKeywords { .. } | Op::SetPlace { .. }))
        {
            self.refresh_distinct()?;
        }
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
            writer.add_document(self.doc(row, &photo))?;
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

    /// The photos `filter` selects that have a position, clustered for `view`. A filter's
    /// own `within` is set aside, so the map keeps showing the places around the area it
    /// picked.
    pub fn map(&self, filter: &Filter, view: &MapView) -> anyhow::Result<MapResult> {
        let distinct = self.distinct.read().unwrap();
        let smart = self.smart.read().unwrap();
        let filter = Filter { within: None, gps: Some(true), ..filter.clone() };
        let query = expand(&filter, &smart)?.to_query(&self.index, &self.fields, &distinct)?;
        let searcher = self.reader.searcher();
        let docs = searcher.search(query.as_ref(), &DocSetCollector)?;
        let mut columns = Vec::new();
        for segment in searcher.segment_readers() {
            let fast = segment.fast_fields();
            columns.push((
                fast.u64("row")?,
                fast.f64("lat")?,
                fast.f64("lon")?,
                fast.u64("rating_sort")?,
                fast.f64("sharpness")?,
            ));
        }
        let points: Vec<Located> = docs
            .iter()
            .filter_map(|a| {
                let (row, lat, lon, rating, sharp) = &columns[a.segment_ord as usize];
                Some(Located {
                    row: row.first(a.doc_id)?,
                    lat: lat.first(a.doc_id)?,
                    lon: lon.first(a.doc_id)?,
                    rating: rating.first(a.doc_id).unwrap_or(0),
                    sharpness: sharp.first(a.doc_id).unwrap_or(0.0),
                })
            })
            .collect();
        Ok(geo::cluster(&points, view))
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

    /// The sources' albums, as they will be once queued album changes are written.
    pub fn collections(&self) -> anyhow::Result<Vec<Collection>> {
        let mut all = self.store.collections()?;
        for (_, queued) in self.edits.collections()? {
            queued.change.apply_to(&mut all);
        }
        Ok(all)
    }

    /// Queues an album change and shows it at once. Deleting an album drops the queued
    /// edits that put photos into it (or took them out). Returns its number, for
    /// [`Library::undo_collection`].
    pub fn change_collection(&self, change: CollectionChange) -> anyhow::Result<u64> {
        let current = self.collections()?;
        let label = change.label(&current);
        if let Some(why) = collection_problem(&current, &change) {
            bail!("{label}: {why}");
        }
        let seq = {
            let _writing = self.writing.lock().unwrap();
            let mut edits = self.edits.batch()?;
            let mut dropped = Vec::new();
            if let CollectionChange::Delete { id } = &change {
                let gone = descendants(&current, id);
                for (key, pending) in edits.all()? {
                    if membership(&pending.change.op).is_some_and(|c| gone.contains(c)) {
                        edits.remove(&key)?;
                        dropped.push((key, pending));
                    }
                }
            }
            let seq = edits.add_collection(&QueuedCollection { change, label, at: now_ms(), dropped })?;
            edits.commit()?;
            seq
        };
        self.refresh_smart()?;
        Ok(seq)
    }

    /// Queued album changes, in order.
    pub fn collection_changes(&self) -> anyhow::Result<Vec<(u64, QueuedCollection)>> {
        self.edits.collections()
    }

    /// Drops a queued album change, bringing back the edits it dropped. Dropping the
    /// making of an album takes what was queued for it since: changes to it, albums made
    /// in it, and photos put into it.
    pub fn undo_collection(&self, seq: u64) -> anyhow::Result<()> {
        let discard = {
            let _writing = self.writing.lock().unwrap();
            let mut edits = self.edits.batch()?;
            let queued = edits.collections()?;
            let Some((_, first)) = queued.iter().find(|(s, _)| *s == seq) else { return Ok(()) };
            let mut undo = vec![seq];
            let mut made: Vec<CollectionId> = Vec::new();
            if let CollectionChange::Create { id, .. } = &first.change {
                made.push(id.clone());
                for (later, q) in queued.iter().filter(|(s, _)| *s > seq) {
                    let inside =
                        matches!(&q.change, CollectionChange::Create { parent: Some(p), .. } if made.contains(p));
                    if made.contains(q.change.id()) || inside {
                        undo.push(*later);
                        if let CollectionChange::Create { id, .. } = &q.change {
                            made.push(id.clone());
                        }
                    }
                }
            }
            for (s, q) in queued.iter().filter(|(s, _)| undo.contains(s)) {
                for (key, pending) in &q.dropped {
                    edits.put(key, pending)?;
                }
                edits.remove_collection(*s)?;
            }
            let discard: Vec<String> = edits
                .all()?
                .into_iter()
                .filter(|(_, p)| membership(&p.change.op).is_some_and(|c| made.contains(c)))
                .map(|(key, _)| key)
                .collect();
            edits.commit()?;
            discard
        };
        if !discard.is_empty() {
            self.discard(&discard)?;
        }
        self.refresh_smart()
    }

    /// After a source took these album changes: they are done.
    pub fn settle_collections(&self, seqs: &[u64]) -> anyhow::Result<()> {
        let _writing = self.writing.lock().unwrap();
        let mut edits = self.edits.batch()?;
        for seq in seqs {
            edits.remove_collection(*seq)?;
        }
        edits.commit()?;
        drop(_writing);
        self.refresh_smart()
    }

    /// The sources' keyword trees, as they will be once queued keyword changes are
    /// written: moved keywords at their new paths (joined ones counted together), new
    /// parents added, emptied groups gone.
    pub fn keywords(&self) -> anyhow::Result<Vec<KeywordNode>> {
        let mut nodes = self.store.keywords()?;
        for (_, queued) in self.edits.remaps()? {
            let remap = &queued.remap;
            let mut out: Vec<KeywordNode> = Vec::with_capacity(nodes.len());
            for node in nodes {
                let path = remap.path(&node.path);
                match out.iter_mut().find(|n| n.path == path) {
                    Some(same) => same.uses += node.uses,
                    None => out.push(KeywordNode { path, uses: node.uses }),
                }
            }
            let parents: Vec<KeywordPath> =
                out.iter().flat_map(|n| (1..n.path.0.len()).map(|k| KeywordPath(n.path.0[..k].to_vec()))).collect();
            for parent in parents {
                if !out.iter().any(|n| n.path == parent) {
                    out.push(KeywordNode { path: parent, uses: 0 });
                }
            }
            let paths: Vec<KeywordPath> = out.iter().map(|n| n.path.clone()).collect();
            out.retain(|n| {
                !remap.prune.contains(&n.path)
                    || n.uses > 0
                    || paths.iter().any(|c| c.0.len() > n.path.0.len() && c.0.starts_with(&n.path.0))
            });
            nodes = out;
        }
        Ok(nodes)
    }

    /// Queues a keyword tree change and shows it at once: every photo carrying a moved
    /// keyword is rewritten here, and queued keyword edits follow the keywords to their
    /// new paths. Returns its number, for [`Library::undo_remap`].
    pub fn remap_keywords(&self, remap: KeywordRemap) -> anyhow::Result<u64> {
        if remap.map.is_empty() && remap.prune.is_empty() {
            bail!("{}: nothing would change", remap.label);
        }
        let _writing = self.writing.lock().unwrap();
        let carrying = Filter {
            keywords_under: remap.map.iter().map(|(from, _)| from.to_pipe()).collect(),
            trashed: Trashed::Include,
            ..Filter::default()
        };
        let rows = self.query(&carrying, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows;
        let mut writer = self.writer()?;
        let mut batch = self.store.batch()?;
        let mut edits = self.edits.batch()?;
        for row in rows {
            let Some(mut photo) = batch.photo(row)? else { continue };
            if remap.apply(&mut photo.keywords) {
                let fingerprint = batch.fingerprint(row)?.unwrap_or_default();
                batch.put_photo(row, &photo, fingerprint)?;
                writer.delete_term(Term::from_field_u64(self.fields.row, row));
                writer.add_document(self.doc(row, &photo))?;
            }
        }
        // Queued keyword edits name keywords by path: they move with them.
        for (key, mut pending) in edits.all()? {
            let moved = match &pending.change.op {
                Op::AddKeywords { paths } if remap.moves(&paths[0]) => {
                    Op::AddKeywords { paths: vec![remap.path(&paths[0])] }
                }
                Op::RemoveKeywords { paths } if remap.moves(&paths[0]) => {
                    Op::RemoveKeywords { paths: vec![remap.path(&paths[0])] }
                }
                _ => continue,
            };
            pending.change.op = moved;
            edits.remove(&key)?;
            let what = pending.change.op.queue_key().unwrap_or_default();
            edits.put(&pending_key(&pending.change.photo, &what), &pending)?;
        }
        let seq = edits.add_remap(&QueuedRemap { remap, at: now_ms() })?;
        writer.commit()?;
        batch.commit()?;
        edits.commit()?;
        self.reader.reload()?;
        self.refresh_distinct()?;
        Ok(seq)
    }

    /// Queued keyword tree changes, in order.
    pub fn remaps(&self) -> anyhow::Result<Vec<(u64, QueuedRemap)>> {
        self.edits.remaps()
    }

    /// Drops a queued keyword tree change. The photos it rewrote are read from their
    /// source again by the next sync, which must follow (with the remaining changes laid
    /// over them). Queued keyword edits that moved with it stay where they are.
    pub fn undo_remap(&self, seq: u64) -> anyhow::Result<()> {
        let _writing = self.writing.lock().unwrap();
        let mut edits = self.edits.batch()?;
        let Some(queued) = edits.get_remap(seq)? else { return Ok(()) };
        edits.remove_remap(seq)?;
        let moved: Vec<String> = queued.remap.map.iter().map(|(_, to)| to.to_pipe()).collect();
        let carrying = Filter { keywords_under: moved, trashed: Trashed::Include, ..Filter::default() };
        let rows = self.query(&carrying, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows;
        let mut batch = self.store.batch()?;
        for row in rows {
            if let Some(photo) = batch.photo(row)? {
                // A fingerprint no photo has: the next sync takes the source's version.
                batch.put_photo(row, &photo, 0)?;
            }
        }
        // And that sync reads everything, not only what changed in the source.
        for (key, _) in batch.metas_with_prefix("cursor:")? {
            batch.remove_meta(&key)?;
        }
        batch.commit()?;
        edits.commit()
    }

    /// After a source took these keyword tree changes: they are done.
    pub fn settle_remaps(&self, seqs: &[u64]) -> anyhow::Result<()> {
        let _writing = self.writing.lock().unwrap();
        let mut edits = self.edits.batch()?;
        for seq in seqs {
            edits.remove_remap(*seq)?;
        }
        edits.commit()
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

/// An id for an album made in the app: a fresh uuid, which the source stores with it.
pub fn new_collection_id() -> CollectionId {
    CollectionId::new_local(&uuid::Uuid::new_v4().to_string())
}

/// What the index was built with: its schema, and the rules that turn an analysis into
/// classes (`c1analysis::CLASSES`). Either changing rebuilds it.
fn index_version() -> String {
    format!("{SCHEMA_VERSION}.{}", c1analysis::CLASSES)
}

/// The album an op puts a photo into or takes it out of.
fn membership(op: &Op) -> Option<&CollectionId> {
    match op {
        Op::AddToCollection { collection } | Op::RemoveFromCollection { collection } => Some(collection),
        _ => None,
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
