//! The change log read as history: a page of it, two dates compared, photos imported per
//! day, and the photos a pick from the History tab is about.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use super::{Library, schema};
use crate::edits::Event;
use crate::filter::{Filter, HistoryPick};
use crate::history::{Compare, Grouper, LogGroup, LogQuery, Net, kind};

/// Entries read in one go: a page of the log walks back through the log this many at a time.
const CHUNK: u64 = 2000;
/// Groups in a page when the query does not say.
const PAGE: usize = 50;

/// One page of the log, newest first.
#[derive(Clone, Debug, Serialize)]
pub struct LogPage {
    pub groups: Vec<LogGroup>,
    /// What the albums the entries name were called when last seen, by id.
    pub albums: BTreeMap<String, String>,
    /// Pass as `before` to read the page after this one; `None` at the end.
    pub next: Option<u64>,
}

/// How much was logged on each day, and since when.
#[derive(Clone, Debug, Default, Serialize)]
pub struct LogDays {
    /// When the first entry was made: nothing is known of what came before.
    pub start: Option<i64>,
    /// Entries per day, by the local midnight starting it.
    pub days: Vec<(i64, usize)>,
}

/// The photos a pick came to, and the days of the log, for the log as it was.
#[derive(Default)]
pub(super) struct Cache {
    picked: Mutex<Option<Picked>>,
    days: Mutex<DaysCache>,
}

/// A pick, the newest entry when it was made into rows, and the rows.
type Picked = (HistoryPick, Option<u64>, Arc<Vec<u64>>);

#[derive(Default)]
struct DaysCache {
    /// The next entry to count.
    next: u64,
    start: Option<i64>,
    days: BTreeMap<i64, usize>,
    local: schema::LocalDays,
}

impl Library {
    /// The sequence numbers of the entries made between two times (inclusive).
    fn seq_range(&self, from: Option<i64>, to: Option<i64>) -> anyhow::Result<Range<u64>> {
        let lo = match from {
            Some(from) => self.edits.seq_after(from.saturating_sub(1))?,
            None => 0,
        };
        let hi = match to {
            Some(to) => self.edits.seq_after(to)?,
            None => self.edits.last_seq()?.map_or(0, |last| last + 1),
        };
        Ok(lo..hi.max(lo))
    }

    /// Reads entries oldest first, a chunk at a time, so a long log is not held whole.
    fn each_entry(&self, seqs: Range<u64>, each: &mut dyn FnMut(u64, &Event)) -> anyhow::Result<()> {
        let mut at = seqs.start;
        while at < seqs.end {
            let to = (at + CHUNK).min(seqs.end);
            for (seq, event) in self.edits.range(at..to)? {
                each(seq, &event);
            }
            at = to;
        }
        Ok(())
    }

    /// A page of the log, newest first, in groups: one batch of a kind from one origin
    /// is one group.
    pub fn log_page(&self, query: &LogQuery) -> anyhow::Result<LogPage> {
        let limit = if query.limit == 0 { PAGE } else { query.limit };
        let mut seqs = self.seq_range(query.from, query.to)?;
        if let Some(before) = query.before {
            seqs.end = seqs.end.min(before);
        }
        let mut grouper = Grouper::default();
        let mut groups: Vec<LogGroup> = Vec::new();
        'read: while seqs.end > seqs.start {
            let from = seqs.start.max(seqs.end.saturating_sub(CHUNK));
            for (seq, event) in self.edits.range(from..seqs.end)?.into_iter().rev() {
                if query.wants(&event) {
                    groups.extend(grouper.push(seq, &event));
                    // One group past the page: the last one on it is complete.
                    if groups.len() > limit {
                        break 'read;
                    }
                }
            }
            seqs.end = from;
        }
        groups.extend(grouper.finish());
        let more = groups.len() > limit;
        groups.truncate(limit);
        let next = more.then(|| groups.last().map(|g| g.first)).flatten();
        for group in &mut groups {
            if group.photos == 1
                && let Some(photo) = group.sample.what.photo()
            {
                group.row = self.store.row_of(&photo.key())?;
            }
        }
        let ids: HashSet<&c1model::CollectionId> = groups.iter().filter_map(|g| g.sample.what.album()).collect();
        let albums = self.edits.album_names(ids)?.into_iter().map(|(id, name)| (id.0, name)).collect();
        Ok(LogPage { groups, albums, next })
    }

    /// What changed between two times (inclusive; `to` of `None` is now): the net effect
    /// on photos, so a rating set and set back is not a change.
    pub fn compare(&self, from: i64, to: Option<i64>) -> anyhow::Result<Compare> {
        let mut net = Net::default();
        self.each_entry(self.seq_range(Some(from), to)?, &mut |seq, event| net.add(seq, event))?;
        let mut compare = net.compare(Some(from), to, self.log_days()?.start);
        let ids: Vec<c1model::CollectionId> =
            compare.albums.iter().map(|a| c1model::CollectionId(a.id.clone())).collect();
        let names = self.edits.album_names(&ids)?;
        for album in &mut compare.albums {
            album.name = names.iter().find(|(id, _)| id.0 == album.id).map(|(_, name)| name.clone());
        }
        Ok(compare)
    }

    /// How many photos `filter` selects that were imported on each day, by the local
    /// midnight starting it, in date order. Photos with no import time are left out.
    pub fn growth(&self, filter: &Filter) -> anyhow::Result<Vec<(i64, usize)>> {
        let query = self.compile(filter)?;
        let searcher = self.reader.searcher();
        let docs = searcher.search(query.as_ref(), &tantivy::collector::DocSetCollector)?;
        let mut columns = Vec::new();
        for segment in searcher.segment_readers() {
            columns.push(segment.fast_fields().i64("imported")?);
        }
        let mut days: BTreeMap<i64, usize> = BTreeMap::new();
        let mut local = schema::LocalDays::default();
        for a in &docs {
            if let Some(at) = columns[a.segment_ord as usize].first(a.doc_id)
                && let Some(midnight) = local.midnight(at)
            {
                *days.entry(midnight).or_default() += 1;
            }
        }
        Ok(days.into_iter().collect())
    }

    /// Entries per day and when the log starts. Counted once, and from then on only what
    /// was logged since.
    pub fn log_days(&self) -> anyhow::Result<LogDays> {
        let mut cache = self.history.days.lock().unwrap();
        let cache = &mut *cache;
        let from = cache.next;
        let mut next = from;
        self.edits.times_from(from, &mut |seq, at| {
            next = seq + 1;
            cache.start.get_or_insert(at);
            if let Some(midnight) = cache.local.midnight(at) {
                *cache.days.entry(midnight).or_default() += 1;
            }
        })?;
        cache.next = next;
        Ok(LogDays { start: cache.start, days: cache.days.iter().map(|(d, n)| (*d, *n)).collect() })
    }

    /// The rows of the photos a pick from the History tab is about, in order.
    fn history_rows(&self, pick: &HistoryPick) -> anyhow::Result<Arc<Vec<u64>>> {
        let last = self.edits.last_seq()?;
        if let Some((cached, at, rows)) = &*self.history.picked.lock().unwrap()
            && cached == pick
            && *at == last
        {
            return Ok(rows.clone());
        }
        let mut keys: BTreeSet<String> = BTreeSet::new();
        match pick {
            HistoryPick::Changed { from, to, what } => {
                let mut net = Net::default();
                self.each_entry(self.seq_range(Some(*from), *to)?, &mut |seq, event| net.add(seq, event))?;
                keys.extend(net.photos(what));
            }
            HistoryPick::Entries { first, last, kind: only } => {
                self.each_entry(*first..last.saturating_add(1), &mut |_, event| {
                    if only.is_none_or(|k| k == kind(&event.what)) {
                        keys.extend(event.photos().iter().map(|p| p.key()));
                    }
                })?;
            }
        }
        let mut rows: Vec<u64> = Vec::with_capacity(keys.len());
        for key in &keys {
            rows.extend(self.store.row_of(key)?);
        }
        rows.sort_unstable();
        let rows = Arc::new(rows);
        *self.history.picked.lock().unwrap() = Some((pick.clone(), last, rows.clone()));
        Ok(rows)
    }

    /// The filter with every History pick in it (also in nested filters) made into the
    /// rows it picks.
    pub(super) fn resolve_history(&self, filter: &Filter) -> anyhow::Result<Filter> {
        let mut f = filter.clone();
        if let Some(pick) = f.history.take() {
            let picked = self.history_rows(&pick)?;
            f.rows = Some(match f.rows.take() {
                // Another pick or list of rows narrows it.
                Some(rows) => {
                    let keep: HashSet<u64> = picked.iter().copied().collect();
                    rows.into_iter().filter(|r| keep.contains(r)).collect()
                }
                None => picked.as_ref().clone(),
            });
        }
        f.all_of = f.all_of.iter().map(|s| self.resolve_history(s)).collect::<anyhow::Result<_>>()?;
        f.any_of = f.any_of.iter().map(|s| self.resolve_history(s)).collect::<anyhow::Result<_>>()?;
        Ok(f)
    }
}
