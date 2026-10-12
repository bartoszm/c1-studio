//! What to select. Every set field narrows the result (AND), as in the Python selector
//! (`legacy/c1cat-py/c1cat/selector.py`), whose semantics this follows.

use std::collections::HashMap;
use std::ops::Bound;

use anyhow::{Context, bail};
use c1model::ColorLabel;
use serde::{Deserialize, Serialize};
use tantivy::query::{
    AllQuery, BooleanQuery, Occur, Query, QueryParser, RangeQuery, RegexQuery, TermQuery, TermSetQuery,
};
use tantivy::schema::{Field, IndexRecordOption};
use tantivy::{Index, Term};

use crate::schema::{Dim, Fields, flag, normalise_folder};

/// A numeric constraint: `>=4`, `<=2.8`, `2-4`, `3`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Range {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub min_exclusive: bool,
    pub max_exclusive: bool,
}

impl Range {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let t = text.trim();
        let num = |s: &str| s.trim().parse::<f64>().with_context(|| format!("cannot parse number in {text:?}"));
        if let Some((lo, hi)) = t.split_once('-').filter(|(lo, _)| !lo.is_empty()) {
            return Ok(Range { min: Some(num(lo)?), max: Some(num(hi)?), ..Range::default() });
        }
        for (op, build) in [
            (">=", (|v| Range { min: Some(v), ..Range::default() }) as fn(f64) -> Range),
            ("<=", |v| Range { max: Some(v), ..Range::default() }),
            (">", |v| Range { min: Some(v), min_exclusive: true, ..Range::default() }),
            ("<", |v| Range { max: Some(v), max_exclusive: true, ..Range::default() }),
            ("=", |v| Range { min: Some(v), max: Some(v), ..Range::default() }),
        ] {
            if let Some(rest) = t.strip_prefix(op) {
                return Ok(build(num(rest)?));
            }
        }
        let v = num(t)?;
        Ok(Range { min: Some(v), max: Some(v), ..Range::default() })
    }

    fn bounds<T>(&self, convert: impl Fn(f64) -> T) -> (Bound<T>, Bound<T>) {
        let lo = match self.min {
            None => Bound::Unbounded,
            Some(v) if self.min_exclusive => Bound::Excluded(convert(v)),
            Some(v) => Bound::Included(convert(v)),
        };
        let hi = match self.max {
            None => Bound::Unbounded,
            Some(v) if self.max_exclusive => Bound::Excluded(convert(v)),
            Some(v) => Bound::Included(convert(v)),
        };
        (lo, hi)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Presence<T> {
    None,
    Any,
    Values(T),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trashed {
    #[default]
    Exclude,
    Include,
    Only,
}

/// Inclusive millisecond bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lon: f64,
    pub max_lon: f64,
}

impl BBox {
    /// The square the Python `--near LAT,LON,KM` used (not a true radius).
    pub fn around(lat: f64, lon: f64, km: f64) -> Self {
        let dlat = km / 111.0;
        let dlon = km / (111.0 * lat.to_radians().cos().abs()).max(1e-6);
        BBox { min_lat: lat - dlat, max_lat: lat + dlat, min_lon: lon - dlon, max_lon: lon + dlon }
    }
}

/// Fields left out of a JSON filter take their defaults, so callers send only what they set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filter {
    /// Source-side ids (`PhotoId::ext`), e.g. Capture One variant keys.
    pub ids: Vec<String>,
    pub assets: Vec<String>,
    pub sources: Vec<String>,
    /// Shell glob over the lower-cased filename, `dscf*.raf`.
    pub filename: Option<String>,
    pub ext: Vec<String>,
    /// Folder subtree, either slash form.
    pub folder: Option<String>,
    pub rating: Option<Presence<Range>>,
    pub color: Option<Presence<Vec<ColorLabel>>>,
    /// All of these exact keyword paths (`a|b`).
    pub keywords: Vec<String>,
    /// At least one of these.
    pub keywords_any: Vec<String>,
    /// Each of these, or anything beneath it.
    pub keywords_under: Vec<String>,
    pub keywords_not: Vec<String>,
    /// For each of these, some keyword with a name containing it at any level of its
    /// path, ignoring case (Capture One's "keywords contains").
    pub keywords_containing: Vec<String>,
    pub unkeyworded: bool,
    /// Substring of the camera model, case-insensitive.
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<Range>,
    pub aperture: Option<Range>,
    pub focal: Option<Range>,
    pub captured: Option<TimeRange>,
    pub year: Option<i32>,
    /// `YYYY-MM`.
    pub month: Option<String>,
    /// `YYYY-MM-Www`: the days of an ISO week inside a month.
    pub week: Option<String>,
    pub imported: Option<TimeRange>,
    /// Collection ids the photo must be in (each).
    pub collections: Vec<String>,
    pub not_in_collections: Vec<String>,
    pub gps: Option<bool>,
    pub within: Option<BBox>,
    /// A place and everything in it: `Poland|Malopolska`, levels as [`Dim::PlaceTree`]
    /// writes them.
    pub place_under: Option<String>,
    /// Only photos with a country but no ISO country code.
    pub no_country_code: bool,
    /// Substring, case-insensitive.
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    /// Free text: words AND-ed, `"a phrase"`, `a OR b`.
    pub text: Option<String>,
    pub edited: Option<bool>,
    pub style: Option<String>,
    pub has_local_layers: bool,
    pub trashed: Trashed,
    pub variant_index: Option<u32>,
    /// From the analysis, each any of its values: `landscape`, `portrait`, ...
    pub shape: Vec<String>,
    /// Main colours: `blue`, `black & white`, ...
    pub hues: Vec<String>,
    /// `dark`, `medium`, `bright`, `clipped highlights`, `blocked shadows`, `low contrast`.
    pub light: Vec<String>,
    /// `sharp`, `soft`, `blurry`.
    pub focus: Vec<String>,
    /// One duplicate group or series: `dup:<row>`, `series:<row>`.
    pub group: Option<String>,
    /// Only photos that look like another image.
    pub duplicates: bool,
    /// Only photos in a burst.
    pub series: bool,
    /// Measured by the analysis, or not yet.
    pub analysed: Option<bool>,
    /// Detail in the sharpest part of the picture (see `c1analysis::Analysis::sharpness`).
    pub sharpness: Option<Range>,
    /// Each of these too.
    pub all_of: Vec<Filter>,
    /// At least one of these, when any are given.
    pub any_of: Vec<Filter>,
}

/// Values known per dimension, for turning substring filters into exact terms.
pub type Distinct = HashMap<Dim, Vec<String>>;

fn term(field: Field, value: &str) -> Box<dyn Query> {
    Box::new(TermQuery::new(Term::from_field_text(field, value), IndexRecordOption::Basic))
}

fn any_of(field: Field, values: impl IntoIterator<Item = String>) -> Box<dyn Query> {
    Box::new(TermSetQuery::new(values.into_iter().map(|v| Term::from_field_text(field, &v))))
}

fn range_u64(field: Field, r: &Range) -> Box<dyn Query> {
    // Integer columns: round the bounds inwards so `>2.5` means `>=3`.
    let (lo, hi) = r.bounds(|v| v);
    let lo = match lo {
        Bound::Included(v) => Bound::Included(Term::from_field_u64(field, v.ceil().max(0.0) as u64)),
        Bound::Excluded(v) => Bound::Included(Term::from_field_u64(field, (v.floor() + 1.0).max(0.0) as u64)),
        Bound::Unbounded => Bound::Unbounded,
    };
    let hi = match hi {
        Bound::Included(v) if v < 0.0 => return Box::new(BooleanQuery::new(vec![])),
        Bound::Included(v) => Bound::Included(Term::from_field_u64(field, v.floor() as u64)),
        Bound::Excluded(v) if v <= 0.0 => return Box::new(BooleanQuery::new(vec![])),
        Bound::Excluded(v) => Bound::Included(Term::from_field_u64(field, (v.ceil() - 1.0) as u64)),
        Bound::Unbounded => Bound::Unbounded,
    };
    Box::new(RangeQuery::new(lo, hi))
}

fn range_f64(field: Field, r: &Range) -> Box<dyn Query> {
    let (lo, hi) = r.bounds(|v| Term::from_field_f64(field, v));
    Box::new(RangeQuery::new(lo, hi))
}

fn range_i64(field: Field, from: Option<i64>, to: Option<i64>) -> Box<dyn Query> {
    let lo = from.map_or(Bound::Unbounded, |v| Bound::Included(Term::from_field_i64(field, v)));
    let hi = to.map_or(Bound::Unbounded, |v| Bound::Included(Term::from_field_i64(field, v)));
    Box::new(RangeQuery::new(lo, hi))
}

/// `dscf*.raf` → an anchored regex over the whole term.
fn glob_to_regex(glob: &str) -> String {
    let mut out = String::new();
    for c in glob.chars() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            c if c.is_alphanumeric() || c == '_' => out.push(c),
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
    out
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        *self == Filter::default()
    }

    /// This filter with the parts that pick values of `dim` left out: what a sidebar
    /// group counts over, so that picking one value leaves its alternatives visible.
    /// Exclusions (`keywords_not`, `not_in_collections`) stay, as they hide rather than pick.
    pub fn without(&self, dim: Dim) -> Filter {
        let mut f = self.clone();
        match dim {
            Dim::Source => f.sources.clear(),
            Dim::Keyword => {
                f.keywords.clear();
                f.keywords_any.clear();
            }
            Dim::KeywordTree => f.keywords_under.clear(),
            Dim::Collection => f.collections.clear(),
            Dim::Camera => f.camera = None,
            Dim::Lens => f.lens = None,
            Dim::Country => f.country = None,
            Dim::State => f.state = None,
            Dim::City => f.city = None,
            // Years, months and weeks are one tree, and a range of days the other way to
            // the same thing: each level counts as if no date were picked, so the whole
            // tree stays visible.
            Dim::Year | Dim::Month | Dim::Week => {
                f.year = None;
                f.month = None;
                f.week = None;
                f.captured = None;
            }
            Dim::Rating => f.rating = None,
            Dim::Color => f.color = None,
            Dim::Ext => f.ext.clear(),
            Dim::Style => f.style = None,
            Dim::Folder => f.folder = None,
            Dim::PlaceTree => f.place_under = None,
            Dim::Shape => f.shape.clear(),
            Dim::Hue => f.hues.clear(),
            Dim::Light => f.light.clear(),
            Dim::Focus => f.focus.clear(),
            Dim::Group => f.group = None,
            Dim::Flag => {}
        }
        f
    }

    pub fn to_query(&self, index: &Index, f: &Fields, distinct: &Distinct) -> anyhow::Result<Box<dyn Query>> {
        let mut must: Vec<Box<dyn Query>> = Vec::new();
        let mut must_not: Vec<Box<dyn Query>> = Vec::new();
        let flag_field = f.dim(Dim::Flag);
        let has = |name: &str| term(flag_field, name);

        if !self.ids.is_empty() {
            must.push(any_of(f.ext_id, self.ids.iter().cloned()));
        }
        if !self.assets.is_empty() {
            must.push(any_of(f.asset, self.assets.iter().cloned()));
        }
        if !self.sources.is_empty() {
            must.push(any_of(f.dim(Dim::Source), self.sources.iter().cloned()));
        }
        if let Some(glob) = &self.filename {
            must.push(Box::new(RegexQuery::from_pattern(&glob_to_regex(&glob.to_lowercase()), f.filename)?));
        }
        if !self.ext.is_empty() {
            must.push(any_of(f.dim(Dim::Ext), self.ext.iter().map(|e| e.trim_start_matches('.').to_lowercase())));
        }
        if let Some(place) = &self.place_under {
            must.push(term(f.dim(Dim::PlaceTree), place));
        }
        if self.no_country_code {
            must.push(has(flag::NO_COUNTRY_CODE));
        }
        if let Some(folder) = &self.folder {
            must.push(term(f.folder, &normalise_folder(folder)));
        }

        match &self.rating {
            None => {}
            Some(Presence::None) => must_not.push(has(flag::RATED)),
            Some(Presence::Any) => must.push(has(flag::RATED)),
            Some(Presence::Values(r)) => must.push(range_u64(f.rating, r)),
        }
        match &self.color {
            None => {}
            Some(Presence::None) => must_not.push(has(flag::COLORED)),
            Some(Presence::Any) => must.push(has(flag::COLORED)),
            Some(Presence::Values(colors)) => {
                must.push(any_of(f.dim(Dim::Color), colors.iter().map(|c| c.name().to_owned())));
            }
        }

        let kw = f.dim(Dim::Keyword);
        let tree = f.dim(Dim::KeywordTree);
        for path in &self.keywords {
            must.push(term(kw, path));
        }
        if !self.keywords_any.is_empty() {
            must.push(any_of(kw, self.keywords_any.iter().cloned()));
        }
        for path in &self.keywords_under {
            must.push(term(tree, path));
        }
        for path in &self.keywords_not {
            must_not.push(term(kw, path));
        }
        for needle in &self.keywords_containing {
            let needle = needle.to_lowercase();
            // Every node of the keyword tree is a value, so testing each one's own name
            // covers every level.
            let values: Vec<String> = distinct
                .get(&Dim::KeywordTree)
                .into_iter()
                .flatten()
                .filter(|v| v.rsplit('|').next().is_some_and(|name| name.to_lowercase().contains(&needle)))
                .cloned()
                .collect();
            must.push(any_of(tree, values));
        }
        if self.unkeyworded {
            must_not.push(has(flag::KEYWORDED));
        }

        for (dim, needle) in [
            (Dim::Camera, &self.camera),
            (Dim::Lens, &self.lens),
            (Dim::City, &self.city),
            (Dim::State, &self.state),
            (Dim::Country, &self.country),
            (Dim::Style, &self.style),
        ] {
            if let Some(needle) = needle {
                let needle = needle.to_lowercase();
                let values: Vec<String> = distinct
                    .get(&dim)
                    .into_iter()
                    .flatten()
                    .filter(|v| v.to_lowercase().contains(&needle))
                    .cloned()
                    .collect();
                // No value contains it: an empty set query matches nothing, as it should.
                must.push(any_of(f.dim(dim), values));
            }
        }

        if let Some(r) = &self.iso {
            must.push(range_u64(f.iso, r));
        }
        if let Some(r) = &self.aperture {
            must.push(range_f64(f.aperture, r));
        }
        if let Some(r) = &self.focal {
            must.push(range_f64(f.focal, r));
        }
        if let Some(t) = &self.captured {
            must.push(range_i64(f.captured, t.from, t.to));
        }
        if let Some(year) = self.year {
            must.push(term(f.dim(Dim::Year), &year.to_string()));
        }
        if let Some(month) = &self.month {
            must.push(term(f.dim(Dim::Month), month));
        }
        if let Some(week) = &self.week {
            must.push(term(f.dim(Dim::Week), week));
        }
        if let Some(t) = &self.imported {
            must.push(range_i64(f.imported, t.from, t.to));
        }

        let coll = f.dim(Dim::Collection);
        for id in &self.collections {
            must.push(term(coll, id));
        }
        for id in &self.not_in_collections {
            must_not.push(term(coll, id));
        }

        match self.gps {
            Some(true) => must.push(has(flag::GPS)),
            Some(false) => must_not.push(has(flag::GPS)),
            None => {}
        }
        if let Some(b) = &self.within {
            let lat = Range { min: Some(b.min_lat), max: Some(b.max_lat), ..Range::default() };
            let lon = Range { min: Some(b.min_lon), max: Some(b.max_lon), ..Range::default() };
            must.push(range_f64(f.lat, &lat));
            must.push(range_f64(f.lon, &lon));
        }

        if let Some(text) = &self.text {
            let mut parser = QueryParser::for_index(index, vec![f.text]);
            parser.set_conjunction_by_default();
            must.push(parser.parse_query(text).with_context(|| format!("cannot parse text query {text:?}"))?);
        }

        match self.edited {
            Some(true) => must.push(has(flag::EDITED)),
            Some(false) => must_not.push(has(flag::EDITED)),
            None => {}
        }
        if self.has_local_layers {
            must.push(has(flag::LOCAL_LAYERS));
        }
        match self.trashed {
            Trashed::Exclude => must_not.push(has(flag::TRASHED)),
            Trashed::Include => {}
            Trashed::Only => must.push(has(flag::TRASHED)),
        }
        for (dim, values) in
            [(Dim::Shape, &self.shape), (Dim::Hue, &self.hues), (Dim::Light, &self.light), (Dim::Focus, &self.focus)]
        {
            if !values.is_empty() {
                must.push(any_of(f.dim(dim), values.iter().cloned()));
            }
        }
        if let Some(g) = &self.group {
            must.push(term(f.dim(Dim::Group), g));
        }
        if self.duplicates {
            must.push(has(flag::DUPLICATE));
        }
        if self.series {
            must.push(has(flag::SERIES));
        }
        if let Some(r) = &self.sharpness {
            must.push(range_f64(f.sharpness, r));
        }
        match self.analysed {
            Some(true) => must.push(has(flag::ANALYSED)),
            Some(false) => must_not.push(has(flag::ANALYSED)),
            None => {}
        }
        if let Some(i) = self.variant_index {
            let i = f64::from(i);
            must.push(range_u64(f.variant_index, &Range { min: Some(i), max: Some(i), ..Range::default() }));
        }

        for sub in &self.all_of {
            must.push(sub.to_query(index, f, distinct)?);
        }
        if !self.any_of.is_empty() {
            let mut either = Vec::new();
            for sub in &self.any_of {
                either.push((Occur::Should, sub.to_query(index, f, distinct)?));
            }
            must.push(Box::new(BooleanQuery::new(either)));
        }

        if must.is_empty() {
            must.push(Box::new(AllQuery));
        }
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = must.into_iter().map(|q| (Occur::Must, q)).collect();
        clauses.extend(must_not.into_iter().map(|q| (Occur::MustNot, q)));
        Ok(Box::new(BooleanQuery::new(clauses)))
    }
}

pub fn parse_color(text: &str) -> anyhow::Result<Presence<Vec<ColorLabel>>> {
    match text.trim().to_lowercase().as_str() {
        "none" => Ok(Presence::None),
        "any" => Ok(Presence::Any),
        list => {
            let mut colors = Vec::new();
            for name in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                match ColorLabel::from_name(name) {
                    Some(c) => colors.push(c),
                    None => {
                        bail!("unknown colour {name:?}; choose from red, orange, yellow, green, blue, pink, purple")
                    }
                }
            }
            Ok(Presence::Values(colors))
        }
    }
}

pub fn parse_rating(text: &str) -> anyhow::Result<Presence<Range>> {
    match text.trim().to_lowercase().as_str() {
        "none" => Ok(Presence::None),
        "any" => Ok(Presence::Any),
        other => Ok(Presence::Values(Range::parse(other)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_parse_like_the_python_selector() {
        assert_eq!(Range::parse(">=4").unwrap(), Range { min: Some(4.0), ..Range::default() });
        assert_eq!(Range::parse("2-4").unwrap(), Range { min: Some(2.0), max: Some(4.0), ..Range::default() });
        assert_eq!(Range::parse("1.4-2.8").unwrap().max, Some(2.8));
        assert_eq!(Range::parse("3").unwrap(), Range { min: Some(3.0), max: Some(3.0), ..Range::default() });
        assert!(Range::parse("<2.8").unwrap().max_exclusive);
        assert!(Range::parse("abc").is_err());
    }

    #[test]
    fn globs_become_anchored_regexes() {
        assert_eq!(glob_to_regex("dscf*.raf"), "dscf.*\\.raf");
        assert_eq!(glob_to_regex("a?b"), "a.b");
    }

    #[test]
    fn the_date_tree_counts_as_if_no_date_were_picked() {
        let f = Filter {
            year: Some(2024),
            month: Some("2024-05".into()),
            week: Some("2024-05-W20".into()),
            camera: Some("X".into()),
            ..Filter::default()
        };
        for dim in [Dim::Year, Dim::Month, Dim::Week] {
            assert_eq!(f.without(dim), Filter { camera: Some("X".into()), ..Filter::default() });
        }
        let range = Filter { captured: Some(TimeRange { from: Some(0), to: None }), ..Filter::default() };
        assert_eq!(range.without(Dim::Year), Filter::default());
    }

    #[test]
    fn time_ranges_take_either_bound() {
        let f: Filter = serde_json::from_str(r#"{"captured":{"from":1000}}"#).unwrap();
        assert_eq!(f.captured, Some(TimeRange { from: Some(1000), to: None }));
    }

    #[test]
    fn colours_parse() {
        assert_eq!(parse_color("red, blue").unwrap(), Presence::Values(vec![ColorLabel::Red, ColorLabel::Blue]));
        assert!(parse_color("teal").is_err());
    }
}
