//! The library: a store and a search index kept in step, fed by [`PhotoSource`]s.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, RwLock};
use std::time::Instant;

use anyhow::{Context, bail};
use c1analysis::ClassSettings;
use c1model::change::{Check, collection_diff, collection_problem, descendants, diff, membership_problem};
use c1model::{
    Change, Collection, CollectionChange, CollectionId, Condition, Cursor, KeywordNode, KeywordPath, KeywordRemap,
    MODEL_VERSION, Op, Photo, PhotoId, PhotoSource, SmartRule, SourceEvent,
};
use serde::{Deserialize, Serialize};
use tantivy::collector::{Count, DocSetCollector, TopDocs};
use tantivy::columnar::StrColumn;
use tantivy::directory::MmapDirectory;
use tantivy::query::{AllQuery, Query};
use tantivy::{
    DocAddress, DocId, Index, IndexReader, IndexWriter, Order, ReloadPolicy, Score, Searcher, SegmentReader, Term,
};

use crate::edits::{self, Edits, Event, Origin, Pending, QueuedCollection, QueuedRemap, What, now_ms, pending_key};
use crate::facets::{Counts, FacetCollector};
use crate::filter::{Distinct, Filter, Presence, Range, Trashed};
use crate::looks::{Look, Looks, Record};
use crate::matching::{self, MatchSpec, Measures};
use crate::places::{Coarse, Guessed, PlaceBook};
use crate::schema::{self, Dim, Fields, SCHEMA_VERSION};
use crate::store::Store;
use crate::trips::{self, Home, Shot, Trip, TripBook, TripEdit, TripSettings};

mod placing;
mod selection;

pub use placing::{FillGroup, NeighbourPlan, NeighbourSettings, PlaceLookup, Unresolved, place_of_path};
pub use selection::{Merged, Selection, group_places};

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
    /// By the analysis; photos not measured come last either way. `Sharpness` is the
    /// softest first, `SharpnessDesc` the sharpest.
    Sharpness,
    SharpnessDesc,
    Brightness,
    BrightnessDesc,
    /// By the colour label: red to purple, and the other way round; unlabelled last.
    ColorLabel,
    ColorLabelRev,
    /// By how well a photo matches the colours, light and focus the filter picks (see
    /// [`crate::matching`]); the newest first when it picks none. Photos not measured last.
    Match,
    MatchWorst,
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
            "sharpness" if text.starts_with('-') => Sort::SharpnessDesc,
            "sharpness" => Sort::Sharpness,
            "sharpness-desc" => Sort::SharpnessDesc,
            "brightness" if text.starts_with('-') => Sort::BrightnessDesc,
            "brightness" => Sort::Brightness,
            "brightness-desc" => Sort::BrightnessDesc,
            "color" | "colour" | "label" => Sort::ColorLabel,
            "color-rev" | "colour-rev" | "label-rev" => Sort::ColorLabelRev,
            "match" => Sort::Match,
            "match-worst" => Sort::MatchWorst,
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

/// A duplicate group or series: its id (a [`Filter::group`] value), how many photos it has,
/// how many of them the filter selects, and the first few of those.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Group {
    pub id: String,
    pub size: usize,
    pub matched: usize,
    pub rows: Vec<u64>,
    /// Where its newest and its oldest matching photo come among the matches in date
    /// order, and its sharpest photo's sharpness: for ordering groups.
    #[serde(skip)]
    newest: usize,
    #[serde(skip)]
    oldest: usize,
    #[serde(skip)]
    sharpest: f32,
}

/// How groups are ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupSort {
    /// Most matching photos first.
    #[default]
    Matched,
    /// Biggest group first.
    Size,
    Newest,
    Oldest,
    /// Sharpest best photo first.
    Sharpest,
}

/// The duplicate group and the series a photo falls in, and whether it is stacked away
/// (not the sharpest of one of them).
type Groupings = (Option<String>, Option<String>, bool);

/// What a photo's measures come to as classes: shape, colours, light, focus.
type Classes = (&'static str, Vec<&'static str>, Vec<&'static str>, &'static str);

fn classes_of(a: &c1analysis::Analysis, s: &ClassSettings) -> Classes {
    (a.shape(s), a.hues(s), a.light(s), a.focus(s))
}

/// How many measured photos some class settings would put in each class.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ClassPreview {
    /// Photos measured.
    pub measured: usize,
    /// `<dimension>|<class>` (`hue|red`, `focus|soft`) → photos.
    pub counts: BTreeMap<String, usize>,
    /// Photos that would be in a duplicate group, and in a series.
    pub duplicates: usize,
    pub series: usize,
}

/// A page of [`Group`]s, and how many there are in all.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GroupPage {
    pub total: usize,
    pub groups: Vec<Group>,
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

/// Where a sync is: reading the source (no count yet), putting what it sent into the
/// index, or writing the index out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncStep {
    Reading,
    Indexing { done: usize, total: usize },
    Saving,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SyncReport {
    pub full: bool,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub deleted: usize,
    /// The source's albums differ from those stored.
    pub albums: bool,
    /// The source's keyword tree differs from the one stored.
    pub keywords: bool,
    pub millis: u128,
}

impl SyncReport {
    /// Whether anything the library shows changed. A full read can change nothing: the
    /// source's file was touched, its photos were not.
    pub fn changed(&self) -> bool {
        self.added + self.updated + self.deleted > 0 || self.albums || self.keywords
    }
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
    /// Where the lines between classes lie (also in `looks`): read when documents are made.
    classes: RwLock<ClassSettings>,
    /// One writer at a time: syncs and edits both rewrite documents.
    writing: Mutex<()>,
    /// One sync at a time: a sync reads its source before it takes `writing`.
    syncing: Mutex<()>,
    /// Trips the user kept, named or dismissed, and the trip settings.
    trip_book: TripBook,
    /// Trips as last worked out, for the index generation they were worked out on.
    trip_cache: Mutex<Option<TripCache>>,
    /// Places looked up online, and photos given only their place's position.
    place_book: PlaceBook,
    /// Where places are, as worked out from their names.
    place_cache: Mutex<placing::PlaceCache>,
    /// Photos given only their place's position, by photo key (also in `place_book`):
    /// read when their documents are made, so the index knows them.
    coarse: RwLock<HashMap<String, Coarse>>,
    /// Photos given a position from their neighbours in time, by photo key (also in
    /// `place_book`).
    guessed: RwLock<HashMap<String, Guessed>>,
}

/// Trips worked out over the photos of one index generation.
struct TripCache {
    generation: u64,
    settings: TripSettings,
    /// Each trip with its name from its places and how many photos it spans.
    trips: Vec<(Trip, String, usize)>,
    /// The city photographed on most days, while no home is set.
    guess: Option<Home>,
}

/// A trip as shown: stored, named, and counted over a filter.
#[derive(Clone, Debug, Serialize)]
pub struct TripRow {
    #[serde(flatten)]
    pub trip: Trip,
    /// The places it is named after when the user has not named it.
    pub label: String,
    /// Photos in its span.
    pub photos: usize,
    /// Of them, the ones the filter selects (its own trip set aside).
    pub count: usize,
    /// The photo shown for it among those: highest rating, then sharpest.
    pub cover: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TripList {
    pub settings: TripSettings,
    /// A home to offer while none is set.
    pub home_guess: Option<Home>,
    pub trips: Vec<TripRow>,
}

/// A photo on a trip's route: where and when it was taken.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RoutePoint {
    pub at: i64,
    pub lat: f64,
    pub lon: f64,
    pub row: u64,
}

/// A photo the filter selects, as trips and the timeline count it.
struct Taken {
    at: i64,
    row: u64,
    rating: u64,
    sharpness: f64,
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
        let trip_book = TripBook::open(&dir.join("trips.redb"))?;
        let place_book = PlaceBook::open(&dir.join("places.redb"))?;
        // A photo moved since it was given its place's position is placed for real: its
        // mark goes. (One with no position keeps it: an undo can be redone.)
        let mut marks = place_book.coarse()?;
        let mut moved = Vec::new();
        let keys: HashMap<u64, &str> = store.rows_of(marks.keys().map(String::as_str))?.into_iter().collect();
        store.photos(&keys.keys().copied().collect::<Vec<_>>(), |row, p| {
            if p.geo.is_some_and(|g| !marks[keys[&row]].holds(g.lat, g.lon)) {
                moved.push((keys[&row].to_owned(), None));
            }
        })?;
        place_book.set_coarse(&moved)?;
        for (key, _) in &moved {
            marks.remove(key);
        }
        let coarse = RwLock::new(marks);
        let guessed = RwLock::new(place_book.guessed()?);
        let classes = looks.settings()?;
        let seen = looks
            .all()?
            .into_iter()
            .map(|(key, r)| {
                (key, Look { analysis: r.analysis, stamp: r.stamp, duplicates: None, series: None, stacked: false })
            })
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
            classes: RwLock::new(classes),
            writing: Mutex::new(()),
            syncing: Mutex::new(()),
            trip_book,
            trip_cache: Mutex::new(None),
            place_book,
            place_cache: Mutex::default(),
            coarse,
            guessed,
        };
        // Groups as worked out now: the index is told of any that differ from last time.
        let regrouped = library.group_looks()?;
        let flagged = library.store.meta("coarse_flagged")?.is_some();
        if !rebuild {
            library.reindex_keys(&regrouped)?;
            // Photos given their place's position by a build that did not flag them yet:
            // written again, once.
            if !flagged {
                let coarse: Vec<String> = library.coarse.read().unwrap().keys().cloned().collect();
                library.reindex_keys(&coarse)?;
            }
        }
        if rebuild {
            library.reindex()?;
        }
        if !flagged {
            let mut batch = library.store.batch()?;
            batch.set_meta("coarse_flagged", "1")?;
            batch.commit()?;
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
        let key = photo.id.key();
        let coarse =
            photo.geo.is_some_and(|g| self.coarse.read().unwrap().get(&key).is_some_and(|c| c.holds(g.lat, g.lon)));
        schema::document(&self.fields, row, photo, seen.get(&key), coarse, &self.classes.read().unwrap())
    }

    /// Where the lines between classes lie in this library.
    pub fn class_settings(&self) -> ClassSettings {
        self.classes.read().unwrap().clone()
    }

    /// Draws the lines between classes where `settings` say (brought into range first;
    /// returns what was kept): the measured photos are classed and grouped again, and
    /// those whose classes changed are written to the index. Nothing is measured again.
    pub fn set_class_settings(&self, settings: &ClassSettings) -> anyhow::Result<ClassSettings> {
        let settings = settings.sanitized();
        let _writing = self.writing.lock().unwrap();
        if settings == *self.classes.read().unwrap() {
            return Ok(settings);
        }
        let before = std::mem::replace(&mut *self.classes.write().unwrap(), settings.clone());
        // Kept only once the index follows: were it stored first, a failed re-index would leave
        // settings the index does not show, and asking for them again would do nothing.
        let applied = (|| {
            let mut changed: HashSet<String> = self
                .seen
                .read()
                .unwrap()
                .iter()
                .filter(|(_, l)| classes_of(&l.analysis, &before) != classes_of(&l.analysis, &settings))
                .map(|(k, _)| k.clone())
                .collect();
            changed.extend(self.group_looks()?);
            let keys: Vec<String> = changed.into_iter().collect();
            self.reindex_keys(&keys)?;
            self.looks.set_settings(&settings)
        })();
        if let Err(e) = applied {
            *self.classes.write().unwrap() = before;
            return Err(e);
        }
        Ok(settings)
    }

    /// How many measured photos `settings` would put in each class, and in groups, without
    /// changing anything: for a preview while the user moves a slider.
    pub fn preview_classes(&self, settings: &ClassSettings) -> anyhow::Result<ClassPreview> {
        let settings = settings.sanitized();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let measured = {
            let seen = self.seen.read().unwrap();
            for look in seen.values() {
                let (shape, hues, light, focus) = classes_of(&look.analysis, &settings);
                for (dim, names) in [("shape", vec![shape]), ("hue", hues), ("light", light), ("focus", vec![focus])] {
                    for name in names {
                        *counts.entry(format!("{dim}|{name}")).or_default() += 1;
                    }
                }
            }
            seen.len()
        };
        let wanted = self.grouping(&settings)?;
        Ok(ClassPreview {
            measured,
            counts,
            duplicates: wanted.values().filter(|w| w.0.is_some()).count(),
            series: wanted.values().filter(|w| w.1.is_some()).count(),
        })
    }

    /// Every measured photo's analysis and groups.
    pub fn looks(&self) -> Vec<Look> {
        self.seen.read().unwrap().values().cloned().collect()
    }

    /// [`Library::groups_sorted`], most matching first.
    pub fn groups(&self, filter: &Filter, series: bool, offset: usize, limit: usize) -> anyhow::Result<GroupPage> {
        self.groups_sorted(filter, series, GroupSort::default(), false, offset, limit)
    }

    /// The duplicate groups (or series) among the photos `filter` selects, in `sort` order:
    /// each with its size, how many of its photos match and a few of those to show.
    /// A group with fewer matching than a group needs (2, [`ClassSettings::series_min`] for a
    /// series) is left out: one red frame of a burst is no series. `limit` groups from
    /// `offset` on, with the number of groups in all. Stacking (see [`Filter::stacked`]) is
    /// left out of the filter: a group is counted whole. `reverse` turns `sort` round
    /// (smallest first, oldest first); groups that tie keep their order.
    pub fn groups_sorted(
        &self,
        filter: &Filter,
        series: bool,
        sort: GroupSort,
        reverse: bool,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<GroupPage> {
        let narrowed = Filter {
            duplicates: filter.duplicates || !series,
            series: filter.series || series,
            stacked: false,
            ..filter.clone()
        };
        // In date order, a group's place among the matches says how new or old it is.
        let order = if matches!(sort, GroupSort::Newest | GroupSort::Oldest) { Sort::DateDesc } else { Sort::Row };
        let rows = self.query(&narrowed, &QueryOptions { sort: order, ..QueryOptions::default() })?.rows;
        let mut by: HashMap<String, Group> = HashMap::new();
        for (at, row) in rows.into_iter().enumerate() {
            let Some(look) = self.look(row) else { continue };
            let Some(id) = (if series { look.series } else { look.duplicates }) else { continue };
            let group = by.entry(id.clone()).or_insert_with(|| Group {
                id,
                size: 0,
                matched: 0,
                rows: Vec::new(),
                newest: at,
                oldest: at,
                sharpest: f32::MIN,
            });
            group.matched += 1;
            group.oldest = at;
            group.sharpest = group.sharpest.max(look.analysis.sharpness);
            if group.rows.len() < GROUP_SHOWN {
                group.rows.push(row);
            }
        }
        let least = if series { self.classes.read().unwrap().series_min } else { 2 };
        by.retain(|_, g| g.matched >= least);
        if by.is_empty() {
            return Ok(GroupPage { total: 0, groups: Vec::new() });
        }
        // Each whole group's size, as the details panel counts it: every group in one count.
        let all = Filter { duplicates: !series, series, ..Filter::default() };
        let options = QueryOptions { limit: Some(0), facets: vec![Dim::Group], ..QueryOptions::default() };
        for (id, n) in self.query(&all, &options)?.facets.remove(&Dim::Group).unwrap_or_default() {
            if let Some(g) = by.get_mut(&id) {
                g.size = n as usize;
            }
        }
        // A photo trashed since the groups were worked out still matches with trash shown.
        for g in by.values_mut() {
            g.size = g.size.max(g.matched);
        }
        let mut out: Vec<Group> = by.into_values().collect();
        let most = |a: &Group, b: &Group| b.matched.cmp(&a.matched).then_with(|| b.size.cmp(&a.size));
        out.sort_by(|a, b| {
            let primary = match sort {
                GroupSort::Matched => most(a, b),
                GroupSort::Size => b.size.cmp(&a.size).then_with(|| b.matched.cmp(&a.matched)),
                GroupSort::Newest => a.newest.cmp(&b.newest),
                GroupSort::Oldest => b.oldest.cmp(&a.oldest),
                GroupSort::Sharpest => b.sharpest.total_cmp(&a.sharpest),
            };
            (if reverse { primary.reverse() } else { primary }).then_with(|| most(a, b)).then_with(|| a.id.cmp(&b.id))
        });
        let total = out.len();
        Ok(GroupPage { total, groups: out.into_iter().skip(offset).take(limit).collect() })
    }

    /// What the analysis found for a photo, if it has been measured.
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
                let (duplicates, series, stacked) =
                    old.map(|o| (o.duplicates, o.series, o.stacked)).unwrap_or_default();
                seen.insert(key, Look { analysis, stamp, duplicates, series, stacked });
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

    /// The duplicate group and series (`dup:<row>`, `series:<row>`) each measured photo
    /// falls in under `settings`, by photo key; photos in neither are left out.
    fn grouping(&self, settings: &ClassSettings) -> anyhow::Result<HashMap<String, Groupings>> {
        let mut members: Vec<(u64, String, c1analysis::Taken)> = Vec::new();
        let mut sharpness: Vec<f32> = Vec::new();
        // What grouping needs of each analysis, copied out so the lock is not held while the
        // photos are read from the store (seconds on a big library): that would stall
        // everything that looks a photo up, and the analysis writer.
        let looks: HashMap<String, (u64, f32)> = {
            let seen = self.seen.read().unwrap();
            seen.iter().map(|(k, l)| (k.clone(), (l.analysis.hash, l.analysis.sharpness))).collect()
        };
        if looks.is_empty() {
            return Ok(HashMap::new());
        }
        for (row, key) in self.store.keys()? {
            let Some(&(hash, sharp)) = looks.get(&key) else { continue };
            let Some(photo) = self.store.photo(row)? else { continue };
            if photo.trashed {
                continue;
            }
            let taken = c1analysis::Taken {
                asset: photo.asset.0.clone(),
                camera: photo.exif.camera_model.clone(),
                at: photo.captured_at,
                hash,
            };
            sharpness.push(sharp);
            members.push((row, key, taken));
        }
        let taken: Vec<c1analysis::Taken> = members.iter().map(|(_, _, t)| t.clone()).collect();
        let groups = c1analysis::group(&taken, settings);
        let mut wanted: HashMap<String, Groupings> = HashMap::new();
        for (prefix, list) in [("dup", &groups.duplicates), ("series", &groups.series)] {
            for group in list {
                let name = format!("{prefix}:{}", group.iter().map(|&i| members[i].0).min().unwrap_or(0));
                // The sharpest stays on show (the lowest row among equals).
                let keep = group.iter().copied().max_by(|&a, &b| {
                    sharpness[a].total_cmp(&sharpness[b]).then_with(|| members[b].0.cmp(&members[a].0))
                });
                for &i in group {
                    let slot = wanted.entry(members[i].1.clone()).or_default();
                    if prefix == "dup" {
                        slot.0 = Some(name.clone());
                    } else {
                        slot.1 = Some(name.clone());
                    }
                    slot.2 |= Some(i) != keep;
                }
            }
        }
        Ok(wanted)
    }

    /// Sets each measured photo's groups; returns the keys whose groups changed.
    fn group_looks(&self) -> anyhow::Result<Vec<String>> {
        let mut wanted = self.grouping(&self.classes.read().unwrap().clone())?;
        let mut changed = Vec::new();
        let mut seen = self.seen.write().unwrap();
        for (key, look) in seen.iter_mut() {
            let (duplicates, series, stacked) = wanted.remove(key.as_str()).unwrap_or_default();
            if look.duplicates != duplicates || look.series != series || look.stacked != stacked {
                look.duplicates = duplicates;
                look.series = series;
                look.stacked = stacked;
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

    /// Adds values edits brought to those substring filters match against, without a
    /// pass over the whole index. Values no photo has any more stay until the next
    /// refresh; a filter matching one finds nothing by it.
    fn learn_distinct(&self, learned: HashSet<(Dim, String)>) {
        if learned.is_empty() {
            return;
        }
        let mut distinct = self.distinct.write().unwrap();
        let dims: HashSet<Dim> = learned.iter().map(|(dim, _)| *dim).collect();
        for dim in dims {
            let values = distinct.entry(dim).or_default();
            let known: HashSet<&String> = values.iter().collect();
            let new: Vec<String> =
                learned.iter().filter(|(d, v)| *d == dim && !known.contains(v)).map(|(_, v)| v.clone()).collect();
            values.extend(new);
        }
    }

    /// Pulls a source's current state into the library. What changed there is logged as
    /// the source's doing.
    pub fn sync(&self, source: &dyn PhotoSource) -> anyhow::Result<SyncReport> {
        self.sync_as(source, Origin::Source)
    }

    /// [`Library::sync`], with what changed logged as coming from `origin` (a backup put
    /// back from the app, say).
    pub fn sync_as(&self, source: &dyn PhotoSource, origin: Origin) -> anyhow::Result<SyncReport> {
        self.sync_with(source, origin, &mut |_| {})
    }

    /// [`Library::sync_as`], telling `progress` which step it is on: it is called often
    /// while indexing, so it should be cheap.
    pub fn sync_with(
        &self,
        source: &dyn PhotoSource,
        origin: Origin,
        progress: &mut dyn FnMut(SyncStep),
    ) -> anyhow::Result<SyncReport> {
        let _one = self.syncing.lock().unwrap_or_else(|e| e.into_inner());
        progress(SyncStep::Reading);
        let started = Instant::now();
        let source_id = source.info().id.0;
        let cursor_key = format!("cursor:{source_id}");
        let since = self.store.meta(&cursor_key)?.map(Cursor);
        // Read before taking the writer: a changed catalog takes seconds to read, and edits
        // made meanwhile need not wait for it. Only syncs change the cursor and the stored
        // photos, and they go one at a time.
        let mut events = Vec::new();
        source.snapshot(since.as_ref(), &mut |event| {
            events.push(event);
            Ok(())
        })?;

        let _writing = self.writing.lock().unwrap();
        // Queued edits stay on top of whatever the source sends.
        let mut queued: HashMap<String, Vec<(String, Pending)>> = HashMap::new();
        for (key, pending) in self.edits.pending()? {
            queued.entry(edits::photo_key(&key).to_owned()).or_default().push((key, pending));
        }
        let remaps: Vec<KeywordRemap> = self.edits.remaps()?.into_iter().map(|(_, q)| q.remap).collect();
        let mut edits = self.edits.batch()?;
        let now = now_ms();
        let known: Vec<(u64, String)> = self.store.keys()?;
        let prefix = format!("{source_id}\u{1f}");
        // The first fill of a source (or a refill after the store was dropped) is not news:
        // its photos and albums are not logged as added. `imported_at` dates them.
        let first_fill = !known.iter().any(|(_, key)| key.starts_with(&prefix));
        // Album changes this app wrote come back from the source: logged already, when
        // they were queued. Kept until a sync that saw the source's albums commits.
        let mut written = edits.written()?;
        let mut saw_albums = false;
        // Albums the app made come back under the source's own ids: app id → source id.
        let mut renamed: HashMap<CollectionId, CollectionId> = HashMap::new();
        // Albums the app deleted, and those in them: photos the store still has in them
        // are not news when they no longer are.
        let mut deleted: HashSet<CollectionId> = HashSet::new();

        let mut batch = self.store.batch()?;
        let mut writer = self.writer()?;
        let mut report = SyncReport::default();
        let mut seen: HashSet<u64> = HashSet::new();
        let mut end: Option<Cursor> = None;
        let row_field = self.fields.row;

        // Sources send their albums before their photos, so photos are diffed against
        // albums already sorted out.
        let total = events.len();
        for (done, event) in events.into_iter().enumerate() {
            if done % 256 == 0 {
                progress(SyncStep::Indexing { done, total });
            }
            match event {
                SourceEvent::BeginFull => report.full = true,
                SourceEvent::BeginIncremental => {}
                SourceEvent::Upsert(mut photo) => {
                    let key = photo.id.key();
                    // The source's own state; queued edits laid over it below do not count.
                    let fingerprint = photo.fingerprint();
                    let mut by = origin;
                    let (row, old) = match batch.row_of(&key)? {
                        Some(row) => {
                            let stored = batch.fingerprint(row)?;
                            if stored == Some(fingerprint) {
                                report.unchanged += 1;
                                seen.insert(row);
                                continue;
                            }
                            // Fingerprint 0 marks a photo the app asked to read again (a
                            // keyword tree change undone): what changed there is the app's doing.
                            if stored == Some(0) && origin == Origin::Source {
                                by = Origin::App;
                            }
                            writer.delete_term(Term::from_field_u64(row_field, row));
                            report.updated += 1;
                            (row, batch.photo(row)?)
                        }
                        None => {
                            report.added += 1;
                            if !first_fill {
                                let what = What::Added { photo: photo.id.clone(), name: photo.file.name.clone() };
                                edits.log(&Event { at: now, origin: by, what })?;
                            }
                            (batch.next_row()?, None)
                        }
                    };
                    for remap in &remaps {
                        remap.apply(&mut photo.keywords);
                    }
                    for (pending_key, mut pending) in queued.remove(&key).unwrap_or_default() {
                        match pending.change.op.check(&photo) {
                            // The source holds what the user wants: nothing left to write.
                            Check::NoOp => edits.remove(&pending_key)?,
                            Check::Apply => {
                                if pending.theirs.take().is_some() {
                                    edits.put(&pending_key, &pending)?;
                                }
                                pending.change.op.apply_to(&mut photo);
                            }
                            // Changed on both sides: keep the user's value until they choose.
                            Check::Conflict(_) => {
                                let theirs = pending.change.op.valued_from(&photo);
                                if pending.theirs.as_ref() != Some(&theirs) {
                                    pending.theirs = Some(theirs);
                                    edits.put(&pending_key, &pending)?;
                                }
                                pending.change.op.apply_to(&mut photo);
                            }
                        }
                    }
                    if let Some(mut old) = old {
                        old.collections.retain(|id| !deleted.contains(id));
                        for id in &mut old.collections {
                            if let Some(real) = renamed.get(id) {
                                *id = real.clone();
                            }
                        }
                        for op in diff(&old, &photo) {
                            let what = What::Change(Change { photo: photo.id.clone(), op });
                            edits.log(&Event { at: now, origin: by, what })?;
                        }
                    }
                    seen.insert(row);
                    batch.put_photo(row, &photo, fingerprint)?;
                    writer.add_document(self.doc(row, &photo))?;
                }
                SourceEvent::Delete(id) => {
                    let key = id.key();
                    if let Some(row) = batch.row_of(&key)? {
                        let name = batch.photo(row)?.map(|p| p.file.name).unwrap_or_default();
                        edits.log(&Event { at: now, origin, what: What::Removed { photo: id, name } })?;
                        writer.delete_term(Term::from_field_u64(row_field, row));
                        batch.delete_photo(row, &key)?;
                        report.deleted += 1;
                    }
                }
                SourceEvent::Collections(collections) => {
                    saw_albums = true;
                    let old = batch.collections_of(&source_id)?;
                    report.albums = old != collections;
                    if !first_fill {
                        let (news, known) =
                            sort_out_written(&mut written, collection_diff(&old, &collections), &mut renamed);
                        for change in &known {
                            if let CollectionChange::Delete { id } = change {
                                deleted.extend(descendants(&old, id));
                            }
                        }
                        for change in news {
                            let label = match &change {
                                CollectionChange::Create { .. } => change.label(&collections),
                                _ => change.label(&old),
                            };
                            let what = What::Collection { change, label };
                            edits.log(&Event { at: now, origin, what })?;
                        }
                    }
                    edits.name_albums(&collections)?;
                    batch.put_collections(&source_id, &collections)?;
                }
                SourceEvent::Keywords(keywords) => {
                    report.keywords = batch.keywords_of(&source_id)? != keywords;
                    batch.put_keywords(&source_id, &keywords)?;
                }
                SourceEvent::End(cursor) => end = Some(cursor),
            }
        }

        let Some(end) = end else { bail!("source {source_id} ended its snapshot without a cursor") };
        if report.full {
            // Anything of this source not seen in a full snapshot is gone.
            for (row, key) in known.iter().filter(|(row, key)| key.starts_with(&prefix) && !seen.contains(row)) {
                if let Some(photo) = batch.photo(*row)? {
                    let what = What::Removed { photo: photo.id, name: photo.file.name };
                    edits.log(&Event { at: now, origin, what })?;
                }
                writer.delete_term(Term::from_field_u64(row_field, *row));
                batch.delete_photo(*row, key)?;
                report.deleted += 1;
            }
        }
        if saw_albums {
            // The source's albums after the write are in: what did not come back never will.
            edits.clear_written()?;
        }
        batch.set_meta(&cursor_key, &end.0)?;
        batch.set_meta("index_schema", &index_version())?;

        // Index first: if we crash between the two commits the store still holds the old
        // fingerprints, so the next sync redoes the work instead of losing it.
        progress(SyncStep::Saving);
        writer.commit()?;
        // One index writer at a time: the regrouping below makes its own.
        drop(writer);
        batch.commit()?;
        edits.commit()?;
        self.refresh()?;
        if report.added + report.updated + report.deleted > 0 {
            self.places_changed();
            // A photo trashed, retimed or added can change who is in a group, and who is
            // its sharpest (the one that stays when groups are stacked). `writing` is held.
            let regrouped = self.group_looks()?;
            self.reindex_keys(&regrouped)?;
        }
        report.millis = started.elapsed().as_millis();
        Ok(report)
    }

    /// Applies set-field edits (rating, colour, ...) to photos by row and queues them for
    /// their source. The library shows them at once. Returns, per row, the op that puts
    /// back what the photo showed before, for undo.
    pub fn edit(&self, changes: &[(u64, Op)]) -> anyhow::Result<Vec<(u64, Op)>> {
        self.edit_as(changes, Origin::App)
    }

    /// [`Library::edit`], logged as coming from `origin` (a GPX track, say).
    pub fn edit_as(&self, changes: &[(u64, Op)], origin: Origin) -> anyhow::Result<Vec<(u64, Op)>> {
        self.edit_made(|_| Ok(changes.to_vec()), origin)
    }

    /// Like [`edit`](Self::edit), with each photo's ops made from the photo as it is when
    /// the edit runs. Ops made from an earlier read could undo what was edited since: a
    /// location typed while the city was still being saved would set the city back.
    pub fn edit_photos(
        &self,
        rows: &[u64],
        ops: impl Fn(u64, &Photo) -> anyhow::Result<Vec<Op>>,
    ) -> anyhow::Result<Vec<(u64, Op)>> {
        let made = |batch: &crate::store::Batch| {
            let mut changes = Vec::new();
            for &row in rows {
                if let Some(photo) = batch.photo(row)? {
                    changes.extend(ops(row, &photo)?.into_iter().map(|op| (row, op)));
                }
            }
            Ok(changes)
        };
        self.edit_made(made, Origin::App)
    }

    fn edit_made(
        &self,
        make: impl FnOnce(&crate::store::Batch) -> anyhow::Result<Vec<(u64, Op)>>,
        origin: Origin,
    ) -> anyhow::Result<Vec<(u64, Op)>> {
        let writing = self.writing.lock().unwrap();
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
        let mut learned = HashSet::new();
        let mut placed = false;
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
            match &op {
                Op::AddKeywords { paths } => learned.extend(
                    paths
                        .iter()
                        .flat_map(|k| k.ancestors_and_self().collect::<Vec<_>>())
                        .map(|p| (Dim::KeywordTree, p)),
                ),
                Op::SetPlace { value, .. } => {
                    placed = true;
                    learned.extend(
                        [(Dim::Country, &value.country), (Dim::State, &value.state), (Dim::City, &value.city)]
                            .into_iter()
                            .filter_map(|(dim, v)| Some((dim, v.clone()?))),
                    )
                }
                _ => {}
            }
            undo.push((row, shown.reverted()));
            edits.log(&Event { at: now, origin, what: What::Change(Change { photo: photo.id.clone(), op: shown }) })?;
            let fingerprint = batch.fingerprint(row)?.unwrap_or_default();
            batch.put_photo(row, &after, fingerprint)?;
            writer.delete_term(Term::from_field_u64(self.fields.row, row));
            writer.add_document(self.doc(row, &after))?;
        }
        writer.commit()?;
        batch.commit()?;
        edits.commit()?;
        self.reader.reload()?;
        // The index's own lock goes with the writer; the next edit waits for both.
        drop(writer);
        drop(writing);
        self.learn_distinct(learned);
        if placed {
            self.places_changed();
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

    /// Everything the change log holds about one photo, newest first.
    pub fn photo_events(&self, id: &PhotoId) -> anyhow::Result<Vec<Event>> {
        self.edits.photo_events(&id.key())
    }

    /// What the albums these events name were called when last seen: also those deleted
    /// since, and those made here under an id the source has replaced.
    pub fn album_names(&self, events: &[Event]) -> anyhow::Result<Vec<(CollectionId, String)>> {
        let ids: HashSet<&CollectionId> = events.iter().filter_map(|e| e.what.album()).collect();
        self.edits.album_names(ids)
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

    /// Logs that a write reached its source: `applied`, the photo edits the source did not
    /// hold already, and so many album and keyword tree changes. Nothing written, nothing logged.
    pub fn log_written(&self, applied: &[Change], albums: usize, keywords: usize) -> anyhow::Result<()> {
        if applied.is_empty() && albums == 0 && keywords == 0 {
            return Ok(());
        }
        let photos = applied.iter().map(|c| &c.photo).collect::<HashSet<_>>().len();
        let what = What::Written { photos, changes: applied.len(), albums, keywords };
        let mut edits = self.edits.batch()?;
        edits.log(&Event { at: now_ms(), origin: Origin::App, what })?;
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
            let what = What::Change(Change { photo: photo.id.clone(), op: shown });
            edits.log(&Event { at: now, origin: Origin::App, what })?;
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
        // Stacking leaves out the frames behind the sharpest of their group. Where other
        // filters narrow the photos, only those whose sharpest frame is among the matches go:
        // a frame that matches stays when its sharpest does not, so no burst disappears.
        if filter.stacked && filter.group.is_none() && filter.stacked_in.is_none() {
            let base = Filter { stacked: false, ..filter.clone() };
            if base != Filter::default() {
                let keepers = Filter { stacked: true, ..base.clone() };
                let counting = QueryOptions { limit: Some(0), facets: vec![Dim::Group], ..QueryOptions::default() };
                let mut counted = self.query_plain(&keepers, &counting)?;
                let kept: Vec<String> =
                    counted.facets.remove(&Dim::Group).unwrap_or_default().into_iter().map(|(id, _)| id).collect();
                return self.query_plain(&Filter { stacked_in: Some(kept), ..base }, options);
            }
        }
        self.query_plain(filter, options)
    }

    fn query_plain(&self, filter: &Filter, options: &QueryOptions) -> anyhow::Result<QueryResult> {
        let started = Instant::now();
        let distinct = self.distinct.read().unwrap();
        let smart = self.smart.read().unwrap();
        let compile = |f: &Filter| expand(f, &smart).and_then(|f| f.to_query(&self.index, &self.fields, &distinct));
        let query = compile(filter)?;
        let searcher = self.reader.searcher();
        // Best (or worst) match ranks by the colours, light and focus the filter picks; with
        // none to rank by, the newest come first.
        let mut options = options.clone();
        let spec = if matches!(options.sort, Sort::Match | Sort::MatchWorst) {
            let lines = self.classes.read().unwrap();
            // Picks a card asks for all of ("red and blue") come as `all_of` alternatives.
            let gather = |pick: fn(&Filter) -> &Vec<String>| -> Vec<String> {
                std::iter::once(filter).chain(&filter.all_of).flat_map(|f| pick(f).iter().cloned()).collect()
            };
            let (hues, light, focus) = (gather(|f| &f.hues), gather(|f| &f.light), gather(|f| &f.focus));
            let every = filter.all_of.iter().filter(|f| !f.hues.is_empty()).count() > 1;
            let spec = MatchSpec::new(&hues, &light, &focus, &lines).map(|s| s.every_colour(every));
            if spec.is_none() {
                options.sort = Sort::DateDesc;
            }
            spec
        } else {
            None
        };
        let mut result = self.run(&searcher, query.as_ref(), &options, spec)?;
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

    /// The trips, each counted over the photos `filter` selects (its trip set aside, so the
    /// other trips stay visible), with the trip settings.
    pub fn trips(&self, filter: &Filter) -> anyhow::Result<TripList> {
        // Copied out, so counting under the filter does not hold up other calls.
        let (settings, guess, trips) = {
            let cache = self.trip_cache()?;
            let cache = cache.as_ref().expect("trip_cache fills it");
            (cache.settings.clone(), cache.guess.clone(), cache.trips.clone())
        };
        let mut taken = self.taken(&Filter { trip: None, ..filter.clone() })?;
        taken.sort_by_key(|t| t.at);
        let mut rows = Vec::with_capacity(trips.len());
        for (trip, label, photos) in trips {
            let from = taken.partition_point(|t| t.at < trip.start);
            let to = taken.partition_point(|t| t.at <= trip.end);
            let cover = taken[from..to]
                .iter()
                .max_by(|a, b| {
                    (a.rating, a.sharpness, std::cmp::Reverse(a.row))
                        .partial_cmp(&(b.rating, b.sharpness, std::cmp::Reverse(b.row)))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|t| t.row);
            rows.push(TripRow { trip, label, photos, count: to - from, cover });
        }
        Ok(TripList { settings, home_guess: guess, trips: rows })
    }

    /// How many photos `filter` selects on each day, by the local midnight starting it,
    /// in date order. Days with none are left out.
    pub fn timeline(&self, filter: &Filter) -> anyhow::Result<Vec<(i64, usize)>> {
        let mut days: HashMap<i64, usize> = HashMap::new();
        let mut local = schema::LocalDays::default();
        for t in self.taken(filter)? {
            if let Some(midnight) = local.midnight(t.at) {
                *days.entry(midnight).or_default() += 1;
            }
        }
        let mut out: Vec<(i64, usize)> = days.into_iter().collect();
        out.sort_unstable();
        Ok(out)
    }

    /// The photos `filter` selects that have a position and a capture time, in the order
    /// they were taken: a route to draw. A filter's own `within` is set aside, as for the map.
    pub fn route(&self, filter: &Filter) -> anyhow::Result<Vec<RoutePoint>> {
        let query = self.compile(&Filter { within: None, gps: Some(true), ..filter.clone() })?;
        let searcher = self.reader.searcher();
        let docs = searcher.search(query.as_ref(), &DocSetCollector)?;
        let mut columns = Vec::new();
        for segment in searcher.segment_readers() {
            let fast = segment.fast_fields();
            columns.push((fast.i64("captured_sort")?, fast.u64("row")?, fast.f64("lat")?, fast.f64("lon")?));
        }
        let mut points: Vec<RoutePoint> = docs
            .iter()
            .filter_map(|a| {
                let (captured, row, lat, lon) = &columns[a.segment_ord as usize];
                Some(RoutePoint {
                    at: captured.first(a.doc_id).and_then(schema::captured_of_sort_key)?,
                    lat: lat.first(a.doc_id)?,
                    lon: lon.first(a.doc_id)?,
                    row: row.first(a.doc_id)?,
                })
            })
            .collect();
        points.sort_by_key(|p| (p.at, p.row));
        Ok(points)
    }

    pub fn trip_settings(&self) -> anyhow::Result<TripSettings> {
        self.trip_book.settings()
    }

    /// Stores the trip settings; trips are worked out again with them.
    pub fn set_trip_settings(&self, settings: &TripSettings) -> anyhow::Result<()> {
        self.trip_book.set_settings(settings)?;
        *self.trip_cache.lock().unwrap() = None;
        Ok(())
    }

    /// Does what the user asked to trips; returns the trip to leave selected.
    pub fn edit_trips(&self, edit: TripEdit) -> anyhow::Result<Option<u64>> {
        // Brought up to date first, so the edit applies to the trips as shown.
        let mut cache = self.trip_cache()?;
        let (mut stored, mut next) = self.trip_book.load()?;
        let selected = trips::apply(&mut stored, edit, &mut next)?;
        self.trip_book.save(&stored, next)?;
        *cache = None;
        Ok(selected)
    }

    /// The trips for the current photos: worked out again when the index changed since.
    fn trip_cache(&self) -> anyhow::Result<MutexGuard<'_, Option<TripCache>>> {
        let searcher = self.reader.searcher();
        let generation = searcher.generation().generation_id();
        let mut cache = self.trip_cache.lock().unwrap();
        if cache.as_ref().is_some_and(|c| c.generation == generation) {
            return Ok(cache);
        }
        let shots = self.shots(&searcher)?;
        let settings = self.trip_book.settings()?;
        let (stored, mut next) = self.trip_book.load()?;
        let mut dismissed: Vec<(i64, i64)> =
            stored.iter().filter(|t| t.kind == trips::Kind::Dismissed).map(|t| (t.start, t.end)).collect();
        dismissed.sort_unstable();
        let found = trips::detect(&shots, &settings, &dismissed);
        let current = trips::reconcile(&stored, &found, &mut next);
        if current != stored {
            self.trip_book.save(&current, next)?;
        }
        let home = settings.home.as_ref().and_then(Home::normalised);
        let home = home.as_ref();
        let trips = current
            .into_iter()
            .map(|t| {
                let label = trips::label(&shots, t.start, t.end, home);
                let photos = shots.partition_point(|s| s.at <= t.end) - shots.partition_point(|s| s.at < t.start);
                (t, label, photos)
            })
            .collect();
        let guess = if home.is_none() { trips::guess_home(&shots) } else { None };
        *cache = Some(TripCache { generation, settings, trips, guess });
        Ok(cache)
    }

    /// Every photo with a capture time (trashed ones aside), sorted by it, as trip
    /// detection reads them: straight from the index's columns.
    fn shots(&self, searcher: &Searcher) -> anyhow::Result<Vec<Shot>> {
        let query = self.compile(&Filter::default())?;
        let docs = searcher.search(query.as_ref(), &DocSetCollector)?;
        let mut by_segment: Vec<Vec<u32>> = vec![Vec::new(); searcher.segment_readers().len()];
        for a in docs {
            by_segment[a.segment_ord as usize].push(a.doc_id);
        }
        let mut shots = Vec::new();
        let mut local = schema::LocalDays::default();
        for (segment, docs) in searcher.segment_readers().iter().zip(by_segment) {
            let fast = segment.fast_fields();
            let captured = fast.i64("captured_sort")?;
            let (lat, lon) = (fast.f64("lat")?, fast.f64("lon")?);
            let country = fast.str(Dim::Country.field_name())?;
            let city = fast.str(Dim::City.field_name())?;
            // Each value once per segment: most photos share a handful of places.
            let mut names: HashMap<(bool, u64), String> = HashMap::new();
            let mut name = |column: &Option<StrColumn>, is_city: bool, doc: u32| {
                let column = column.as_ref()?;
                let ord = column.term_ords(doc).next()?;
                if let Some(n) = names.get(&(is_city, ord)) {
                    return Some(n.clone());
                }
                let mut text = String::new();
                column.ord_to_str(ord, &mut text).ok()?;
                names.insert((is_city, ord), text.clone());
                Some(text)
            };
            for doc in docs {
                let Some(at) = captured.first(doc).and_then(schema::captured_of_sort_key) else { continue };
                let Some(day) = local.day(at) else { continue };
                let geo = lat.first(doc).zip(lon.first(doc));
                shots.push(Shot { at, day, country: name(&country, false, doc), city: name(&city, true, doc), geo });
            }
        }
        shots.sort_by_key(|s| s.at);
        Ok(shots)
    }

    /// The photos `filter` selects that have a capture time.
    fn taken(&self, filter: &Filter) -> anyhow::Result<Vec<Taken>> {
        let query = self.compile(filter)?;
        let searcher = self.reader.searcher();
        let docs = searcher.search(query.as_ref(), &DocSetCollector)?;
        let mut columns = Vec::new();
        for segment in searcher.segment_readers() {
            let fast = segment.fast_fields();
            columns.push((
                fast.i64("captured_sort")?,
                fast.u64("row")?,
                fast.u64("rating_sort")?,
                fast.f64("sharpness")?,
            ));
        }
        Ok(docs
            .iter()
            .filter_map(|a| {
                let (captured, row, rating, sharp) = &columns[a.segment_ord as usize];
                Some(Taken {
                    at: captured.first(a.doc_id).and_then(schema::captured_of_sort_key)?,
                    row: row.first(a.doc_id)?,
                    rating: rating.first(a.doc_id).unwrap_or(0) >> 32,
                    sharpness: sharp.first(a.doc_id).unwrap_or(0.0),
                })
            })
            .collect())
    }

    fn compile(&self, filter: &Filter) -> anyhow::Result<Box<dyn Query>> {
        let distinct = self.distinct.read().unwrap();
        let smart = self.smart.read().unwrap();
        expand(filter, &smart)?.to_query(&self.index, &self.fields, &distinct)
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

    fn run(
        &self,
        searcher: &Searcher,
        query: &dyn Query,
        options: &QueryOptions,
        spec: Option<MatchSpec>,
    ) -> anyhow::Result<QueryResult> {
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
            // Descending on the key, which is negated for the ascending sorts, so photos not
            // measured (the lowest key) come last either way.
            Sort::Sharpness | Sort::SharpnessDesc | Sort::Brightness | Sort::BrightnessDesc => {
                let field = match options.sort {
                    Sort::SharpnessDesc => "sharpness_sort",
                    Sort::Sharpness => "sharpness_sort_rev",
                    Sort::BrightnessDesc => "brightness_sort",
                    _ => "brightness_sort_rev",
                };
                let (n, hits, f) =
                    searcher.search(query, &(Count, top.order_by_fast_field::<i64>(field, Order::Desc), facets))?;
                (n, addrs(hits), f)
            }
            Sort::ColorLabel | Sort::ColorLabelRev => {
                let field = if options.sort == Sort::ColorLabel { "color_sort" } else { "color_sort_rev" };
                let (n, hits, f) =
                    searcher.search(query, &(Count, top.order_by_fast_field::<i64>(field, Order::Desc), facets))?;
                (n, addrs(hits), f)
            }
            Sort::Match | Sort::MatchWorst => {
                // `query` only lets these through with something to match.
                let spec = spec.context("nothing to match")?;
                let worst = options.sort == Sort::MatchWorst;
                let scored = top.tweak_score(move |segment: &SegmentReader| {
                    let fast = segment.fast_fields();
                    let measure = |name: &str| fast.f64(name).expect(name).first_or_default_col(0.0);
                    let hues: Vec<_> = c1analysis::HUES
                        .iter()
                        .map(|h| fast.u64(&format!("hue_share_{h}")).expect("hue share").first_or_default_col(0))
                        .collect();
                    let patches: Vec<_> = c1analysis::HUES
                        .iter()
                        .map(|h| fast.u64(&format!("hue_patch_{h}")).expect("hue patch").first_or_default_col(0))
                        .collect();
                    let (chroma, brightness, contrast) =
                        (measure("chroma"), measure("brightness"), measure("contrast"));
                    let (highlights, shadows, sharpness) =
                        (measure("highlights"), measure("shadows"), measure("sharpness"));
                    let measured = fast.i64("sharpness_sort").expect("sharpness_sort").first_or_default_col(i64::MIN);
                    let row = fast.u64("row").expect("row").first_or_default_col(0);
                    let spec = spec.clone();
                    move |doc: DocId, _: Score| {
                        // Photos not measured have the lowest sort key (see `measure_key`).
                        let score = (measured.get_val(doc) >= i64::MIN + (1 << 20)).then(|| {
                            let mut m = Measures {
                                chroma: chroma.get_val(doc) as f32,
                                brightness: brightness.get_val(doc) as f32,
                                contrast: contrast.get_val(doc) as f32,
                                highlights: highlights.get_val(doc) as f32,
                                shadows: shadows.get_val(doc) as f32,
                                sharpness: sharpness.get_val(doc) as f32,
                                ..Measures::default()
                            };
                            for (share, col) in m.hue_shares.iter_mut().zip(&hues) {
                                *share = col.get_val(doc) as f32 / 1000.0;
                            }
                            for (patch, col) in m.hue_patches.iter_mut().zip(&patches) {
                                *patch = (col.get_val(doc) as f64 / schema::PATCH_SCALE) as f32;
                            }
                            spec.score(&m)
                        });
                        matching::key(score, row.get_val(doc), worst)
                    }
                });
                let (n, hits, f) = searcher.search(query, &(Count, scored, facets))?;
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

    /// The photos at `rows` that exist, read together: for a selection's summaries.
    pub fn photos(&self, rows: &[u64], f: impl FnMut(u64, Photo)) -> anyhow::Result<()> {
        self.store.photos(rows, f)
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
            let at = now_ms();
            let what = What::Collection { change: change.clone(), label: label.clone() };
            edits.log(&Event { at, origin: Origin::App, what })?;
            // The log names albums by id; one made here is known by the app's until written.
            let mut after = current.clone();
            change.apply_to(&mut after);
            edits.name_albums(&after)?;
            let seq = edits.add_collection(&QueuedCollection { change, label, at, dropped })?;
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
            let at = now_ms();
            for (s, q) in queued.iter().filter(|(s, _)| undo.contains(s)) {
                for (key, pending) in &q.dropped {
                    edits.put(key, pending)?;
                }
                edits.remove_collection(*s)?;
                edits.log(&Event { at, origin: Origin::App, what: What::Undone { label: q.label.clone() } })?;
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
        let queued = edits.collections()?;
        for seq in seqs {
            // Logged when queued: the sync that sees it come back knows it by this.
            for (_, q) in queued.iter().filter(|(s, _)| s == seq) {
                edits.add_written(&q.change)?;
            }
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
        let at = now_ms();
        edits.log(&Event { at, origin: Origin::App, what: What::Remap(remap.clone()) })?;
        let seq = edits.add_remap(&QueuedRemap { remap, at })?;
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
        let what = What::Undone { label: queued.remap.label.clone() };
        edits.log(&Event { at: now_ms(), origin: Origin::App, what })?;
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

/// Splits album changes `seen` in a source into news and those this app wrote, which are
/// taken out of `written` (each matches once). Albums the app made get the source's own
/// ids, recorded in `renamed`. Returns (news, known), each in the order seen.
fn sort_out_written(
    written: &mut Vec<CollectionChange>,
    seen: Vec<CollectionChange>,
    renamed: &mut HashMap<CollectionId, CollectionId>,
) -> (Vec<CollectionChange>, Vec<CollectionChange>) {
    let mut seen: Vec<Option<CollectionChange>> = seen.into_iter().map(Some).collect();
    let mut known = Vec::new();
    // An album made inside one made in the same write is known once its parent is: go
    // round until no more are.
    loop {
        let mut more = false;
        for slot in &mut seen {
            let Some(change) = slot.as_ref() else { continue };
            let Some(i) = written.iter().position(|w| same_collection_change(w, change, renamed)) else { continue };
            let mine = written.remove(i);
            if matches!(mine, CollectionChange::Create { .. }) {
                renamed.insert(mine.id().clone(), change.id().clone());
            }
            known.extend(slot.take());
            more = true;
        }
        if !more {
            break;
        }
    }
    (seen.into_iter().flatten().collect(), known)
}

/// Whether `seen`, found by comparing a source's albums, is `written` coming back. A new
/// album is matched by name, kind and parent, ids the app made up read as the source's.
fn same_collection_change(
    written: &CollectionChange,
    seen: &CollectionChange,
    renamed: &HashMap<CollectionId, CollectionId>,
) -> bool {
    let real = |id: &CollectionId| renamed.get(id).unwrap_or(id).clone();
    match (written, seen) {
        (
            CollectionChange::Create { name: a, kind: k, parent: p, .. },
            CollectionChange::Create { name: b, kind: l, parent: q, .. },
        ) => a == b && k == l && p.as_ref().map(real) == *q,
        (CollectionChange::Create { .. }, _) | (_, CollectionChange::Create { .. }) => false,
        (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b) && real(a.id()) == *b.id(),
    }
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
