//! What several picked photos have, read together for the details panel: their keywords,
//! albums and places with how many carry each, and each editable field merged.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

use c1model::{ColorLabel, Iptc, Photo, Place};
use serde::Serialize;

use super::Library;

/// One field over the selection: the value all of them share (`same`), how many different
/// values they have, how many have none, and the most used values with their counts.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Merged<T> {
    pub same: Option<T>,
    pub values: usize,
    pub empty: usize,
    pub top: Vec<(T, usize)>,
}

/// Values a [`Merged`] lists, the most used first.
const TOP: usize = 12;

/// The picked photos' keywords, albums and places, and their fields merged.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Selection {
    /// Keyword paths (`a|b`) with how many photos carry each.
    pub keywords: Vec<(String, u64)>,
    /// Album ids with how many photos each holds.
    pub albums: Vec<(String, u64)>,
    /// Places with the photos that have each, the most used first.
    pub places: Vec<(Place, Vec<u64>)>,
    pub rating: Merged<u8>,
    pub color: Merged<ColorLabel>,
    /// `[lat, lon]`.
    pub geo: Merged<[f64; 2]>,
    /// By IPTC field name, as the app's `CaptionField`.
    pub iptc: BTreeMap<&'static str, Merged<String>>,
}

/// Merges one field's values, keyed by `key` (a value its own key, or its bits for floats).
struct Merger<K, T> {
    counts: HashMap<K, (T, usize)>,
    empty: usize,
}

impl<K: Hash + Eq + Ord, T> Merger<K, T> {
    fn new() -> Self {
        Merger { counts: HashMap::new(), empty: 0 }
    }

    fn add(&mut self, value: Option<T>, key: impl Fn(&T) -> K) {
        match value {
            None => self.empty += 1,
            Some(v) => self.counts.entry(key(&v)).or_insert((v, 0)).1 += 1,
        }
    }

    fn done(self) -> Merged<T> {
        let values = self.counts.len();
        let mut top: Vec<(K, (T, usize))> = self.counts.into_iter().collect();
        top.sort_by(|(ka, (_, a)), (kb, (_, b))| b.cmp(a).then_with(|| ka.cmp(kb)));
        let mut top: Vec<(T, usize)> = top.into_iter().take(TOP).map(|(_, v)| v).collect();
        let same = if values == 1 && self.empty == 0 { top.pop().map(|(v, _)| v) } else { None };
        Merged { same, values, empty: self.empty, top }
    }
}

/// The IPTC fields by the names the app uses.
type IptcField = fn(&Iptc) -> &Option<String>;

const IPTC_FIELDS: [(&str, IptcField); 8] = [
    ("title", |i| &i.title),
    ("headline", |i| &i.headline),
    ("description", |i| &i.description),
    ("creator", |i| &i.creator),
    ("copyright", |i| &i.copyright),
    ("job_id", |i| &i.job_id),
    ("instructions", |i| &i.instructions),
    ("source", |i| &i.source),
];

/// A selection summed up one photo at a time, so a big one is never held whole.
struct Tally {
    keywords: BTreeMap<String, u64>,
    albums: BTreeMap<String, u64>,
    places: Vec<(u64, Place)>,
    rating: Merger<u8, u8>,
    color: Merger<u8, ColorLabel>,
    geo: Merger<(u64, u64), [f64; 2]>,
    iptc: Vec<Merger<String, String>>,
}

impl Tally {
    fn new() -> Self {
        Tally {
            keywords: BTreeMap::new(),
            albums: BTreeMap::new(),
            places: Vec::new(),
            rating: Merger::new(),
            color: Merger::new(),
            geo: Merger::new(),
            iptc: IPTC_FIELDS.iter().map(|_| Merger::new()).collect(),
        }
    }

    fn add(&mut self, row: u64, photo: Photo) {
        let text = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
        for path in &photo.keywords {
            *self.keywords.entry(path.to_pipe()).or_default() += 1;
        }
        for id in &photo.collections {
            *self.albums.entry(id.0.clone()).or_default() += 1;
        }
        self.rating.add(photo.rating, |v| *v);
        self.color.add(photo.color, |v| *v as u8);
        self.geo.add(photo.geo.map(|g| [g.lat, g.lon]), |v| (v[0].to_bits(), v[1].to_bits()));
        for (merger, (_, field)) in self.iptc.iter_mut().zip(IPTC_FIELDS) {
            merger.add(text(field(&photo.iptc)), String::clone);
        }
        self.places.push((row, photo.place));
    }

    fn done(self) -> Selection {
        Selection {
            keywords: self.keywords.into_iter().collect(),
            albums: self.albums.into_iter().collect(),
            places: group_places(self.places),
            rating: self.rating.done(),
            color: self.color.done(),
            geo: self.geo.done(),
            iptc: IPTC_FIELDS.iter().map(|(name, _)| *name).zip(self.iptc.into_iter().map(Merger::done)).collect(),
        }
    }
}

/// Groups photos by place, a field of only spaces counting as empty: such a photo has
/// no location, as the index sees it.
pub fn group_places(places: Vec<(u64, Place)>) -> Vec<(Place, Vec<u64>)> {
    let clean = |v: Option<String>| v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let mut groups = BTreeMap::<[Option<String>; 5], Vec<u64>>::new();
    for (row, p) in places {
        let key = [clean(p.country), clean(p.country_code), clean(p.state), clean(p.city), clean(p.location)];
        groups.entry(key).or_default().push(row);
    }
    let groups = groups.into_iter().map(|([country, country_code, state, city, location], rows)| {
        (Place { country, country_code, state, city, location }, rows)
    });
    let mut out: Vec<_> = groups.collect();
    out.sort_by_key(|(_, rows)| std::cmp::Reverse(rows.len()));
    out
}

impl Library {
    /// What the photos at `rows` have, read together in one transaction.
    pub fn selection(&self, rows: &[u64]) -> anyhow::Result<Selection> {
        let mut tally = Tally::new();
        self.photos(rows, |row, photo| tally.add(row, photo))?;
        Ok(tally.done())
    }
}
