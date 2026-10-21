//! The change log read as history: entries grouped by batch, and the net effect of a
//! run of entries between two dates.
//!
//! Pure functions over [`Event`]s; the library reads the log and hands entries here.

use std::collections::{BTreeMap, HashMap, HashSet};

use c1model::change::spot;
use c1model::{CollectionId, KeywordPath, Op};
use serde::{Deserialize, Serialize};

use crate::edits::{Before, Event, Origin, What};

/// What sort of entry the log holds: the fields a photo has, and the other kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    Rating,
    Color,
    Keywords,
    /// Title, caption and the rest of IPTC.
    Iptc,
    Place,
    Geo,
    /// A photo put into or taken out of an album.
    Albums,
    Added,
    Removed,
    Deleted,
    /// An album made, renamed, moved, given a rule or deleted.
    Collection,
    /// A keyword tree change.
    Remap,
    Written,
    Undone,
}

impl LogKind {
    /// The kinds that are a field of one photo, with set values (the rest add up).
    pub const FIELDS: [LogKind; 7] = [
        LogKind::Rating,
        LogKind::Color,
        LogKind::Keywords,
        LogKind::Iptc,
        LogKind::Place,
        LogKind::Geo,
        LogKind::Albums,
    ];

    /// Entries of this kind that come in batches: one change to many photos.
    fn groups(self) -> bool {
        !matches!(self, LogKind::Collection | LogKind::Remap | LogKind::Written | LogKind::Undone)
    }
}

pub fn kind(what: &What) -> LogKind {
    match what {
        What::Change(change) => match &change.op {
            Op::SetRating { .. } => LogKind::Rating,
            Op::SetColor { .. } => LogKind::Color,
            Op::AddKeywords { .. } | Op::RemoveKeywords { .. } => LogKind::Keywords,
            Op::SetIptc { .. } => LogKind::Iptc,
            Op::SetPlace { .. } => LogKind::Place,
            Op::SetGeo { .. } => LogKind::Geo,
            Op::AddToCollection { .. } | Op::RemoveFromCollection { .. } => LogKind::Albums,
        },
        What::Added { .. } => LogKind::Added,
        What::Removed { .. } => LogKind::Removed,
        What::Deleted { .. } => LogKind::Deleted,
        What::Collection { .. } => LogKind::Collection,
        What::Remap(_) => LogKind::Remap,
        What::Written { .. } => LogKind::Written,
        What::Undone { .. } => LogKind::Undone,
    }
}

/// An entry as the UI gets it: without what it kept of the past (which can be a whole
/// photo), only whether there is any.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EventView {
    /// Its place in the log, when it came from a read that knows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    pub at: i64,
    pub origin: Origin,
    #[serde(flatten)]
    pub what: What,
    pub has_before: bool,
}

impl EventView {
    pub fn new(seq: u64, event: &Event) -> Self {
        Self { seq: Some(seq), ..Self::of(event) }
    }

    pub fn of(event: &Event) -> Self {
        Self {
            seq: None,
            at: event.at,
            origin: event.origin,
            what: event.what.clone(),
            has_before: event.before.is_some(),
        }
    }
}

/// Which entries of the log to list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogQuery {
    /// Only entries before this sequence number: the page before the one just read.
    pub before: Option<u64>,
    /// Inclusive bounds on when an entry was made, in milliseconds.
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Any of these origins, or all when empty.
    pub origins: Vec<Origin>,
    /// Any of these kinds, or all when empty.
    pub kinds: Vec<LogKind>,
    /// How many groups to return; 50 when 0.
    pub limit: usize,
}

impl LogQuery {
    pub fn wants(&self, event: &Event) -> bool {
        (self.origins.is_empty() || self.origins.contains(&event.origin))
            && (self.kinds.is_empty() || self.kinds.contains(&kind(&event.what)))
    }
}

/// Consecutive entries of one batch: the same time, origin and kind, like "rating changed
/// on 1,240 photos". Album, keyword tree, written and undone entries stand alone.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LogGroup {
    /// The oldest and newest sequence numbers in it.
    pub first: u64,
    pub last: u64,
    pub at: i64,
    pub origin: Origin,
    pub kind: LogKind,
    pub entries: usize,
    /// How many different photos they are about.
    pub photos: usize,
    /// The newest entry, for wording a group of one.
    pub sample: EventView,
    /// The photo's row when the group is about one that is still in the library.
    pub row: Option<u64>,
}

/// Builds groups from entries pushed newest first.
#[derive(Default)]
pub struct Grouper {
    open: Option<(LogGroup, HashSet<String>)>,
}

impl Grouper {
    /// Adds an entry; returns the group it closed, if it did not join the open one.
    pub fn push(&mut self, seq: u64, event: &Event) -> Option<LogGroup> {
        let kind = kind(&event.what);
        if let Some((group, photos)) = &mut self.open
            && kind.groups()
            && group.kind == kind
            && group.origin == event.origin
            && group.at == event.at
        {
            group.first = seq;
            group.entries += 1;
            photos.extend(event.what.photo().map(|p| p.key()));
            group.photos = photos.len();
            return None;
        }
        let photos: HashSet<String> = event.what.photo().map(|p| p.key()).into_iter().collect();
        let group = LogGroup {
            first: seq,
            last: seq,
            at: event.at,
            origin: event.origin,
            kind,
            entries: 1,
            photos: photos.len(),
            sample: EventView::new(seq, event),
            row: None,
        };
        self.open.replace((group, photos)).map(|(done, _)| done)
    }

    /// The group still open.
    pub fn finish(&mut self) -> Option<LogGroup> {
        self.open.take().map(|(group, _)| group)
    }
}

/// What a pick of "changed between two dates" is about.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NetWhat {
    /// Anything that ended up different.
    Any,
    /// One field.
    Field(LogKind),
    /// This keyword (levels joined by `|`) was added, or removed.
    KeywordAdded(String),
    KeywordRemoved(String),
    /// Put into this album, or taken out of it.
    AlbumIn(String),
    AlbumOut(String),
    /// Photos that were not there at the start.
    Added,
}

/// A photo's presence over the run: whether its first entry was an add (it was not there
/// before), whether its last was (it is there now), and whether it was deleted for good.
#[derive(Clone, Copy, Debug)]
struct Presence {
    absent_before: bool,
    present_after: bool,
    deleted: bool,
}

/// Keywords with how many photos gained or lost each.
pub type KeywordCounts = Vec<(KeywordPath, usize)>;

/// The net effect of a run of entries: what a photo holds at the end against what it held
/// at the start, with changes that cancelled out left out.
#[derive(Default)]
pub struct Net {
    /// Per photo and field, the first and the last op: set fields keep what the field held
    /// then, and what it holds now.
    fields: HashMap<(String, LogKind), (Op, Op)>,
    /// +1 for each add and −1 for each remove, per photo and keyword.
    keywords: HashMap<(String, KeywordPath), i32>,
    albums: HashMap<(String, CollectionId), i32>,
    presence: HashMap<String, Presence>,
    /// How many entries, by who made them.
    pub origins: BTreeMap<String, usize>,
    pub entries: usize,
    pub written: usize,
    pub album_changes: Vec<EventView>,
    pub keyword_changes: Vec<EventView>,
}

/// Whether a set field ended up with another value than it started with.
fn moved(first: &Op, last: &Op) -> bool {
    match (first, last) {
        (Op::SetRating { base, .. }, Op::SetRating { value, .. }) => base != value,
        (Op::SetColor { base, .. }, Op::SetColor { value, .. }) => base != value,
        (Op::SetIptc { base, .. }, Op::SetIptc { value, .. }) => base != value,
        (Op::SetPlace { base, .. }, Op::SetPlace { value, .. }) => base != value,
        (Op::SetGeo { base, .. }, Op::SetGeo { value, .. }) => spot(base) != spot(value),
        _ => true,
    }
}

fn origin_name(origin: Origin) -> &'static str {
    match origin {
        Origin::App => "App",
        Origin::Source => "Source",
        Origin::Gpx => "Gpx",
        Origin::Restore => "Restore",
    }
}

impl Net {
    /// Takes in the next entry, oldest first.
    pub fn add(&mut self, seq: u64, event: &Event) {
        self.entries += 1;
        *self.origins.entry(origin_name(event.origin).to_owned()).or_default() += 1;
        match &event.what {
            What::Change(change) => {
                let photo = change.photo.key();
                match &change.op {
                    Op::AddKeywords { paths } | Op::RemoveKeywords { paths } => {
                        let step = if matches!(change.op, Op::AddKeywords { .. }) { 1 } else { -1 };
                        for path in paths {
                            *self.keywords.entry((photo.clone(), path.clone())).or_default() += step;
                        }
                    }
                    Op::AddToCollection { collection } => {
                        *self.albums.entry((photo, collection.clone())).or_default() += 1;
                    }
                    Op::RemoveFromCollection { collection } => {
                        *self.albums.entry((photo, collection.clone())).or_default() -= 1;
                    }
                    op => {
                        let field = kind(&event.what);
                        self.fields
                            .entry((photo, field))
                            .and_modify(|(_, last)| *last = op.clone())
                            .or_insert_with(|| (op.clone(), op.clone()));
                    }
                }
            }
            What::Added { photo, .. } => {
                let state = self.presence.entry(photo.key()).or_insert(Presence {
                    absent_before: true,
                    present_after: true,
                    deleted: false,
                });
                state.present_after = true;
            }
            What::Removed { photo, .. } | What::Deleted { photo, .. } => {
                let deleted = matches!(event.what, What::Deleted { .. });
                let state = self.presence.entry(photo.key()).or_insert(Presence {
                    absent_before: false,
                    present_after: false,
                    deleted: false,
                });
                state.present_after = false;
                state.deleted |= deleted;
            }
            What::Collection { .. } => self.album_changes.push(EventView::new(seq, event)),
            What::Remap(remap) => {
                self.keyword_changes.push(EventView::new(seq, event));
                // What it did to each photo it rewrote, when the entry kept them.
                if let Some(Before::Keywords(photos)) = &event.before {
                    for (photo, old) in photos {
                        let mut new = old.clone();
                        remap.apply(&mut new);
                        let key = photo.key();
                        for path in old.iter().filter(|p| !new.contains(p)) {
                            *self.keywords.entry((key.clone(), path.clone())).or_default() -= 1;
                        }
                        for path in new.iter().filter(|p| !old.contains(p)) {
                            *self.keywords.entry((key.clone(), path.clone())).or_default() += 1;
                        }
                    }
                }
            }
            What::Written { .. } => self.written += 1,
            What::Undone { .. } => {}
        }
    }

    /// Whether a photo is still there at the end: one removed since has no row to show,
    /// so what happened to it before does not count.
    fn alive(&self, photo: &str) -> bool {
        self.presence.get(photo).is_none_or(|p| p.present_after)
    }

    fn changed(&self) -> impl Iterator<Item = (&String, LogKind)> {
        self.fields
            .iter()
            .filter(|(_, (first, last))| moved(first, last))
            .map(|((photo, kind), _)| (photo, *kind))
            .chain(self.keywords.iter().filter(|(_, n)| **n != 0).map(|((photo, _), _)| (photo, LogKind::Keywords)))
            .chain(self.albums.iter().filter(|(_, n)| **n != 0).map(|((photo, _), _)| (photo, LogKind::Albums)))
            .filter(|(photo, _)| self.alive(photo))
    }

    /// How many photos ended up with each field changed.
    pub fn fields(&self) -> Vec<(LogKind, usize)> {
        let mut per: HashMap<LogKind, HashSet<&String>> = HashMap::new();
        for (photo, kind) in self.changed() {
            per.entry(kind).or_default().insert(photo);
        }
        LogKind::FIELDS.iter().filter_map(|k| per.get(k).map(|p| (*k, p.len()))).collect()
    }

    fn net_presence(&self, wanted: impl Fn(&Presence) -> bool) -> HashSet<&String> {
        self.presence.iter().filter(|(_, p)| wanted(p)).map(|(photo, _)| photo).collect()
    }

    /// Photos that were not there at the start and are at the end.
    pub fn added(&self) -> usize {
        self.net_presence(|p| p.absent_before && p.present_after).len()
    }

    /// Photos that were there at the start and are not at the end.
    pub fn removed(&self) -> usize {
        self.net_presence(|p| !p.absent_before && !p.present_after).len()
    }

    /// Of those, the ones deleted for good from the trash.
    pub fn deleted(&self) -> usize {
        self.net_presence(|p| !p.absent_before && !p.present_after && p.deleted).len()
    }

    /// The keywords most photos gained, and those most lost, `top` of each.
    pub fn keywords(&self, top: usize) -> (KeywordCounts, KeywordCounts) {
        let mut up: HashMap<&KeywordPath, usize> = HashMap::new();
        let mut down: HashMap<&KeywordPath, usize> = HashMap::new();
        for ((_, path), n) in self.keywords.iter().filter(|((photo, _), _)| self.alive(photo)) {
            match n.cmp(&0) {
                std::cmp::Ordering::Greater => *up.entry(path).or_default() += 1,
                std::cmp::Ordering::Less => *down.entry(path).or_default() += 1,
                std::cmp::Ordering::Equal => {}
            }
        }
        let ranked = |counts: HashMap<&KeywordPath, usize>| {
            let mut v: Vec<(KeywordPath, usize)> = counts.into_iter().map(|(p, n)| (p.clone(), n)).collect();
            v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            v.truncate(top);
            v
        };
        (ranked(up), ranked(down))
    }

    /// Per album, how many photos went in and how many came out.
    pub fn albums(&self) -> Vec<(CollectionId, usize, usize)> {
        let mut per: BTreeMap<&CollectionId, (usize, usize)> = BTreeMap::new();
        for ((_, id), n) in self.albums.iter().filter(|((photo, _), _)| self.alive(photo)) {
            let entry = per.entry(id).or_default();
            match n.cmp(&0) {
                std::cmp::Ordering::Greater => entry.0 += 1,
                std::cmp::Ordering::Less => entry.1 += 1,
                std::cmp::Ordering::Equal => {}
            }
        }
        per.into_iter().map(|(id, (i, o))| (id.clone(), i, o)).collect()
    }

    /// The keys of the photos a pick is about, in no particular order.
    pub fn photos(&self, what: &NetWhat) -> Vec<String> {
        let mut out: HashSet<&String> = HashSet::new();
        match what {
            NetWhat::Any => {
                out.extend(self.changed().map(|(photo, _)| photo));
                out.extend(self.net_presence(|p| p.absent_before && p.present_after));
            }
            NetWhat::Field(wanted) => out.extend(self.changed().filter(|(_, k)| k == wanted).map(|(photo, _)| photo)),
            NetWhat::KeywordAdded(path) | NetWhat::KeywordRemoved(path) => {
                let want_up = matches!(what, NetWhat::KeywordAdded(_));
                let path = KeywordPath::from_pipe(path);
                out.extend(
                    self.keywords
                        .iter()
                        .filter(|((photo, p), n)| {
                            *p == path && self.alive(photo) && if want_up { **n > 0 } else { **n < 0 }
                        })
                        .map(|((photo, _), _)| photo),
                );
            }
            NetWhat::AlbumIn(id) | NetWhat::AlbumOut(id) => {
                let want_in = matches!(what, NetWhat::AlbumIn(_));
                out.extend(
                    self.albums
                        .iter()
                        .filter(|((photo, a), n)| {
                            a.0 == *id && self.alive(photo) && if want_in { **n > 0 } else { **n < 0 }
                        })
                        .map(|((photo, _), _)| photo),
                );
            }
            NetWhat::Added => out.extend(self.net_presence(|p| p.absent_before && p.present_after)),
        }
        out.into_iter().cloned().collect()
    }
}

/// How the log compares two dates: what changed between them, in the terms of the
/// sidebar (fields, keywords, albums), as the numbers the History tab shows.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Compare {
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// How many entries lie between, and by whom (`App`, `Source`, `Gpx`, `Restore`).
    pub entries: usize,
    pub by_origin: BTreeMap<String, usize>,
    /// Photos that ended up different in any way, or added.
    pub changed: usize,
    pub added: usize,
    pub removed: usize,
    pub deleted: usize,
    /// Photos whose field ended up different, by field.
    pub fields: Vec<(LogKind, usize)>,
    /// The 20 keywords most photos gained, and those most lost: path and photos.
    pub keywords_added: Vec<(String, usize)>,
    pub keywords_removed: Vec<(String, usize)>,
    /// Per album: id, name when known, photos in, photos out.
    pub albums: Vec<AlbumNet>,
    pub album_changes: Vec<EventView>,
    pub keyword_changes: Vec<EventView>,
    pub written: usize,
    /// When the log starts: nothing before is known.
    pub log_start: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AlbumNet {
    pub id: String,
    pub name: Option<String>,
    pub photos_in: usize,
    pub photos_out: usize,
}

impl Net {
    pub fn compare(&self, from: Option<i64>, to: Option<i64>, log_start: Option<i64>) -> Compare {
        let (up, down) = self.keywords(20);
        let pipe = |v: Vec<(KeywordPath, usize)>| v.into_iter().map(|(p, n)| (p.to_pipe(), n)).collect();
        Compare {
            from,
            to,
            entries: self.entries,
            changed: self.photos(&NetWhat::Any).len(),
            by_origin: self.origins.clone(),
            added: self.added(),
            removed: self.removed(),
            deleted: self.deleted(),
            fields: self.fields(),
            keywords_added: pipe(up),
            keywords_removed: pipe(down),
            albums: self
                .albums()
                .into_iter()
                .map(|(id, photos_in, photos_out)| AlbumNet { id: id.0, name: None, photos_in, photos_out })
                .collect(),
            album_changes: self.album_changes.clone(),
            keyword_changes: self.keyword_changes.clone(),
            written: self.written,
            log_start,
        }
    }
}

#[cfg(test)]
mod tests {
    use c1model::{Change, PhotoId, SourceId};

    use super::*;

    fn photo(n: u32) -> PhotoId {
        PhotoId::new(&SourceId("c1".into()), n.to_string())
    }

    fn event(at: i64, origin: Origin, photo_n: u32, op: Op) -> Event {
        Event::new(at, origin, What::Change(Change { photo: photo(photo_n), op }))
    }

    fn rating(base: Option<u8>, value: Option<u8>) -> Op {
        Op::SetRating { base, value }
    }

    fn net(events: &[Event]) -> Net {
        let mut net = Net::default();
        for (seq, event) in events.iter().enumerate() {
            net.add(seq as u64, event);
        }
        net
    }

    #[test]
    fn a_rating_changed_and_changed_back_has_no_net_change() {
        let n = net(&[
            event(1, Origin::App, 1, rating(None, Some(3))),
            event(2, Origin::App, 1, rating(Some(3), None)),
            event(3, Origin::App, 2, rating(None, Some(5))),
        ]);
        assert_eq!(n.fields(), [(LogKind::Rating, 1)]);
        assert_eq!(n.photos(&NetWhat::Field(LogKind::Rating)), [photo(2).key()]);
    }

    #[test]
    fn a_keyword_added_removed_and_added_again_is_added() {
        let path = KeywordPath::from_pipe("street");
        let add = || Op::AddKeywords { paths: vec![path.clone()] };
        let remove = || Op::RemoveKeywords { paths: vec![path.clone()] };
        let n = net(&[
            event(1, Origin::App, 1, add()),
            event(2, Origin::App, 1, remove()),
            event(3, Origin::App, 1, add()),
            // Added and taken off again: nothing.
            event(4, Origin::App, 2, add()),
            event(5, Origin::App, 2, remove()),
            event(6, Origin::App, 3, remove()),
        ]);
        let (up, down) = n.keywords(20);
        assert_eq!((up, down), (vec![(path.clone(), 1)], vec![(path.clone(), 1)]));
        assert_eq!(n.photos(&NetWhat::KeywordAdded("street".into())), [photo(1).key()]);
        assert_eq!(n.photos(&NetWhat::KeywordRemoved("street".into())), [photo(3).key()]);
    }

    #[test]
    fn photos_added_then_removed_are_not_counted() {
        let at = |n, what: What| Event::new(n, Origin::Source, what);
        let n = net(&[
            at(1, What::Added { photo: photo(1), name: "a".into() }),
            at(2, What::Added { photo: photo(2), name: "b".into() }),
            at(3, What::Removed { photo: photo(2), name: "b".into() }),
            at(4, What::Removed { photo: photo(3), name: "c".into() }),
            at(5, What::Deleted { photo: photo(4), name: "d".into() }),
            at(6, What::Removed { photo: photo(4), name: "d".into() }),
        ]);
        assert_eq!((n.added(), n.removed(), n.deleted()), (1, 2, 1));
        assert_eq!(n.photos(&NetWhat::Added), [photo(1).key()]);
    }

    #[test]
    fn albums_count_the_photos_in_and_out() {
        let album = CollectionId("7".into());
        let n = net(&[
            event(1, Origin::App, 1, Op::AddToCollection { collection: album.clone() }),
            event(2, Origin::App, 2, Op::AddToCollection { collection: album.clone() }),
            event(3, Origin::App, 2, Op::RemoveFromCollection { collection: album.clone() }),
            event(4, Origin::App, 3, Op::RemoveFromCollection { collection: album.clone() }),
        ]);
        assert_eq!(n.albums(), [(album, 1, 1)]);
    }

    #[test]
    fn grouping_splits_by_origin_time_and_kind() {
        let mut grouper = Grouper::default();
        // Newest first.
        let entries = [
            event(5, Origin::Source, 3, rating(None, Some(1))),
            event(5, Origin::Source, 2, rating(None, Some(1))),
            event(5, Origin::App, 1, rating(None, Some(1))),
            event(4, Origin::App, 1, rating(None, Some(1))),
            event(4, Origin::App, 1, Op::SetColor { base: None, value: None }),
        ];
        let mut groups: Vec<LogGroup> = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            groups.extend(grouper.push(10 - i as u64, e));
        }
        groups.extend(grouper.finish());
        let shape: Vec<(i64, Origin, LogKind, usize, usize)> =
            groups.iter().map(|g| (g.at, g.origin, g.kind, g.entries, g.photos)).collect();
        assert_eq!(
            shape,
            [
                (5, Origin::Source, LogKind::Rating, 2, 2),
                (5, Origin::App, LogKind::Rating, 1, 1),
                (4, Origin::App, LogKind::Rating, 1, 1),
                (4, Origin::App, LogKind::Color, 1, 1),
            ]
        );
        assert_eq!((groups[0].first, groups[0].last), (9, 10));
    }

    #[test]
    fn album_and_keyword_tree_entries_are_never_grouped() {
        let mut grouper = Grouper::default();
        let undone = Event::new(1, Origin::App, What::Undone { label: "x".into() });
        let mut done = grouper.push(2, &undone).into_iter().collect::<Vec<_>>();
        done.extend(grouper.push(1, &undone));
        done.extend(grouper.finish());
        assert_eq!(done.len(), 2);
    }

    #[test]
    fn the_kind_and_origin_filters_pick_entries() {
        let query = LogQuery { origins: vec![Origin::Source], kinds: vec![LogKind::Rating], ..LogQuery::default() };
        assert!(query.wants(&event(1, Origin::Source, 1, rating(None, Some(1)))));
        assert!(!query.wants(&event(1, Origin::App, 1, rating(None, Some(1)))));
        assert!(!query.wants(&event(1, Origin::Source, 1, Op::SetColor { base: None, value: None })));
        assert!(LogQuery::default().wants(&event(1, Origin::Gpx, 1, rating(None, None))));
    }
}
