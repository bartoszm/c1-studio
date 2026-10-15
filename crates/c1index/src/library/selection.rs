//! What several picked photos have, read together for the details panel: their keywords,
//! albums and places with how many carry each, and each editable field merged.

use std::collections::{BTreeMap, HashSet};
use std::hash::Hash;

use c1model::{ColorLabel, Iptc, Photo, Place};
use serde::Serialize;

use super::Library;

/// One field over the selection: the value all of them share (`same`), how many different
/// values they have, and how many have none.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Merged<T> {
    pub same: Option<T>,
    pub values: usize,
    pub empty: usize,
}

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
    seen: HashSet<K>,
    first: Option<T>,
    empty: usize,
}

impl<K: Hash + Eq, T> Merger<K, T> {
    fn new() -> Self {
        Merger { seen: HashSet::new(), first: None, empty: 0 }
    }

    fn add(&mut self, value: Option<T>, key: impl Fn(&T) -> K) {
        match value {
            None => self.empty += 1,
            Some(v) => {
                if self.seen.insert(key(&v)) && self.first.is_none() {
                    self.first = Some(v);
                }
            }
        }
    }

    fn done(self) -> Merged<T> {
        let values = self.seen.len();
        let same = if values == 1 && self.empty == 0 { self.first } else { None };
        Merged { same, values, empty: self.empty }
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

/// The summary of `photos` (row and photo).
pub fn summarize(photos: impl IntoIterator<Item = (u64, Photo)>) -> Selection {
    let mut keywords = BTreeMap::<String, u64>::new();
    let mut albums = BTreeMap::<String, u64>::new();
    let mut places = Vec::new();
    let mut rating = Merger::new();
    let mut color = Merger::new();
    let mut geo = Merger::new();
    let mut iptc: Vec<Merger<String, String>> = IPTC_FIELDS.iter().map(|_| Merger::new()).collect();
    let text = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
    for (row, photo) in photos {
        for path in &photo.keywords {
            *keywords.entry(path.to_pipe()).or_default() += 1;
        }
        for id in &photo.collections {
            *albums.entry(id.0.clone()).or_default() += 1;
        }
        rating.add(photo.rating, |v| *v);
        color.add(photo.color, |v| *v);
        geo.add(photo.geo.map(|g| [g.lat, g.lon]), |v| (v[0].to_bits(), v[1].to_bits()));
        for (merger, (_, field)) in iptc.iter_mut().zip(IPTC_FIELDS) {
            merger.add(text(field(&photo.iptc)), String::clone);
        }
        places.push((row, photo.place));
    }
    Selection {
        keywords: keywords.into_iter().collect(),
        albums: albums.into_iter().collect(),
        places: group_places(places),
        rating: rating.done(),
        color: color.done(),
        geo: geo.done(),
        iptc: IPTC_FIELDS.iter().map(|(name, _)| *name).zip(iptc.into_iter().map(Merger::done)).collect(),
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
        let mut photos = Vec::with_capacity(rows.len());
        self.photos(rows, |row, photo| photos.push((row, photo)))?;
        Ok(summarize(photos))
    }
}
