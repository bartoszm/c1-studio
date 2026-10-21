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
/// How many picks' rows are kept.
const PICKS_KEPT: usize = 8;
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
    picked: Mutex<Vec<Picked>>,
    days: Mutex<DaysCache>,
}

/// A pick, the newest entry when it was made into rows (if the rows depend on it), and the rows.
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
    /// Forgets the rows picks came to: photos came or went in a way the log does not say
    /// (the first read of a source logs none).
    pub(super) fn forget_picks(&self) {
        self.history.picked.lock().unwrap().clear();
    }

    /// Turns the History tab's side of the change log on or off (it is on unless turned
    /// off). Off, entries are logged as 0.5 logged them, without what a change replaced;
    /// nothing reads the log as history (a page of it, a compare, the days, the growth
    /// chart), a History pick in a filter is set aside, and what was worked out is let go.
    /// Entries already logged stay as they are, so turning it back on finds them.
    pub fn set_history(&self, on: bool) {
        self.edits.set_keep_before(on);
        if !on {
            self.forget_picks();
            *self.history.days.lock().unwrap() = DaysCache::default();
        }
    }

    /// Whether the History tab's side of the change log is on.
    pub fn history_on(&self) -> bool {
        self.edits.keeps_before()
    }

    fn needs_history(&self) -> anyhow::Result<()> {
        if !self.history_on() {
            anyhow::bail!("History is turned off (Settings › History)");
        }
        Ok(())
    }

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
    /// What an entry kept is read only for those `with_before` says yes to.
    fn each_entry(
        &self,
        seqs: Range<u64>,
        with_before: &dyn Fn(&Event) -> bool,
        each: &mut dyn FnMut(u64, &Event),
    ) -> anyhow::Result<()> {
        let mut at = seqs.start;
        while at < seqs.end {
            let to = (at + CHUNK).min(seqs.end);
            for (seq, event) in self.edits.range(at..to, with_before)? {
                each(seq, &event);
            }
            at = to;
        }
        Ok(())
    }

    /// A page of the log, newest first, in groups: one batch of a kind from one origin
    /// is one group.
    pub fn log_page(&self, query: &LogQuery) -> anyhow::Result<LogPage> {
        self.needs_history()?;
        let limit = if query.limit == 0 { PAGE } else { query.limit };
        let mut seqs = self.seq_range(query.from, query.to)?;
        if let Some(before) = query.before {
            seqs.end = seqs.end.min(before);
        }
        let mut grouper = Grouper::default();
        let mut groups: Vec<LogGroup> = Vec::new();
        'read: while seqs.end > seqs.start {
            let from = seqs.start.max(seqs.end.saturating_sub(CHUNK));
            for (seq, event) in self.edits.range(from..seqs.end, &|_| false)?.into_iter().rev() {
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
        self.needs_history()?;
        let mut net = Net::default();
        // A keyword tree change's photos are what it did to them: its entry is read whole.
        let remaps = |event: &Event| matches!(event.what, crate::edits::What::Remap(_));
        self.each_entry(self.seq_range(Some(from), to)?, &remaps, &mut |seq, event| net.add(seq, event))?;
        let mut compare = net.compare(Some(from), to, self.edits.first_at()?);
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
        self.needs_history()?;
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
        self.needs_history()?;
        let mut cache = self.history.days.lock().unwrap();
        let cache = &mut *cache;
        let from = cache.next;
        self.edits.times_from(from, &mut |seq, at| {
            // Counted and moved past together, so a read that fails halfway counts nothing twice.
            cache.next = seq + 1;
            cache.start.get_or_insert(at);
            if let Some(midnight) = cache.local.midnight(at) {
                *cache.days.entry(midnight).or_default() += 1;
            }
        })?;
        Ok(LogDays { start: cache.start, days: cache.days.iter().map(|(d, n)| (*d, *n)).collect() })
    }

    /// The rows of the photos a pick from the History tab is about, in order.
    fn history_rows(&self, pick: &HistoryPick) -> anyhow::Result<Arc<Vec<u64>>> {
        let last = self.edits.last_seq()?;
        // A pick of entries already in the log, or of days that are over, cannot change with
        // what is logged next; the others are good for as long as the log's end is.
        let settled = match pick {
            HistoryPick::Entries { .. } => true,
            HistoryPick::Changed { to, .. } => match (to, self.edits.last_at()?) {
                (Some(to), Some(newest)) => *to < newest,
                _ => false,
            },
        };
        let end = (!settled).then_some(last).flatten();
        if let Some((_, _, rows)) =
            self.history.picked.lock().unwrap().iter().find(|(p, at, _)| p == pick && (settled || *at == end))
        {
            return Ok(rows.clone());
        }
        let mut keys: BTreeSet<String> = BTreeSet::new();
        match pick {
            HistoryPick::Changed { from, to, what } => {
                let mut net = Net::default();
                let remaps = |event: &Event| matches!(event.what, crate::edits::What::Remap(_));
                self.each_entry(self.seq_range(Some(*from), *to)?, &remaps, &mut |seq, event| net.add(seq, event))?;
                keys.extend(net.photos(what));
            }
            HistoryPick::Entries { first, last, kind: only, origin } => {
                // The photos an entry kept are about it too.
                self.each_entry(*first..last.saturating_add(1), &|_| true, &mut |_, event| {
                    if only.is_none_or(|k| k == kind(&event.what)) && origin.is_none_or(|o| o == event.origin) {
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
        let mut picked = self.history.picked.lock().unwrap();
        picked.retain(|(p, _, _)| p != pick);
        // A few picks at a time: the grid's and the sidebar's own are not one slot.
        if picked.len() >= PICKS_KEPT {
            picked.remove(0);
        }
        picked.push((pick.clone(), end, rows.clone()));
        Ok(rows)
    }

    /// The filter with every History pick in it (also in nested filters) made into the
    /// rows it picks. With History off, a pick is set aside: it picks nothing out.
    pub(super) fn resolve_history(&self, filter: &Filter) -> anyhow::Result<Filter> {
        let mut f = filter.clone();
        if let Some(pick) = f.history.take()
            && self.history_on()
        {
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
