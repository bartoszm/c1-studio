//! The search document: one per photo.
//!
//! Most dimensions are raw string fields that are both indexed (exact-term filters) and
//! fast (columnar, for counting values per facet). Numeric fields are indexed for range
//! filters and fast for sorting. A value that is missing is simply not added, so a range
//! filter never matches a photo without that value — the same as SQL `NULL`.

use c1model::Photo;

use crate::looks::Look;
use std::collections::HashMap;

use chrono::{Datelike, Local, TimeZone};
use tantivy::schema::{FAST, Field, INDEXED, IndexRecordOption, STRING, Schema, TextFieldIndexing, TextOptions};
use tantivy::tokenizer::{AsciiFoldingFilter, LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{Index, TantivyDocument};

/// Bump when fields change; the library rebuilds the index from its store.
pub const SCHEMA_VERSION: u32 = 6;

/// Tokenizer for free text: split on non-alphanumerics, lower-case, strip diacritics,
/// so `wroclaw` finds `Wrocław` (as FTS5's `remove_diacritics 2` did in the Python index).
pub const FOLDED: &str = "folded";

/// A dimension that can be filtered by exact value and counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dim {
    Source,
    /// Exact keyword paths, `a|b`.
    Keyword,
    /// Every keyword plus all its ancestors: counting this gives "photos at or under".
    KeywordTree,
    Collection,
    Camera,
    Lens,
    Country,
    State,
    City,
    Year,
    /// `YYYY-MM`, local date.
    Month,
    /// `YYYY-MM-Www`: the days of ISO week `ww` that fall in that month, so a week
    /// across two months is counted under each, with its own days.
    Week,
    /// `1`..`5`, or `none`.
    Rating,
    /// Colour name, or `none`.
    Color,
    Ext,
    Style,
    Flag,
    /// The photo's folder and every folder above it, `H:/fotos/2026`, in the path's own
    /// case: counting this gives "photos at or under".
    Folder,
    /// `Country|State|City|Location` and every level above it, a missing level as
    /// [`NO_PLACE`]: counting this gives "photos at or under".
    PlaceTree,
    /// From the analysis (stage 3): landscape, portrait, square, panorama.
    Shape,
    /// The colours a picture is mostly made of (`blue`), or `black & white`.
    Hue,
    /// dark, medium, bright; clipped highlights, blocked shadows, low contrast.
    Light,
    /// sharp, soft, blurry.
    Focus,
    /// The duplicate group and series a photo falls in: `dup:<row>`, `series:<row>`.
    Group,
}

/// A level of [`Dim::PlaceTree`] the photo has no value for (`Poland|—|Kraków`: no state).
pub const NO_PLACE: &str = "—";

impl Dim {
    pub const ALL: [Dim; 24] = [
        Dim::Source,
        Dim::Keyword,
        Dim::KeywordTree,
        Dim::Collection,
        Dim::Camera,
        Dim::Lens,
        Dim::Country,
        Dim::State,
        Dim::City,
        Dim::Year,
        Dim::Month,
        Dim::Week,
        Dim::Rating,
        Dim::Color,
        Dim::Ext,
        Dim::Style,
        Dim::Flag,
        Dim::Folder,
        Dim::PlaceTree,
        Dim::Shape,
        Dim::Hue,
        Dim::Light,
        Dim::Focus,
        Dim::Group,
    ];

    pub fn field_name(self) -> &'static str {
        match self {
            Dim::Source => "source",
            Dim::Keyword => "kw",
            Dim::KeywordTree => "kw_tree",
            Dim::Collection => "coll",
            Dim::Camera => "camera",
            Dim::Lens => "lens",
            Dim::Country => "country",
            Dim::State => "state",
            Dim::City => "city",
            Dim::Year => "year",
            Dim::Month => "month",
            Dim::Week => "week",
            Dim::Rating => "rating_s",
            Dim::Color => "color",
            Dim::Ext => "ext",
            Dim::Style => "style",
            Dim::Flag => "flag",
            Dim::Folder => "folder_tree",
            Dim::PlaceTree => "place_tree",
            Dim::Shape => "shape",
            Dim::Hue => "hue",
            Dim::Light => "light",
            Dim::Focus => "focus",
            Dim::Group => "group",
        }
    }
}

/// Boolean properties, stored as values of [`Dim::Flag`].
pub mod flag {
    pub const RATED: &str = "rated";
    pub const COLORED: &str = "colored";
    pub const KEYWORDED: &str = "keyworded";
    pub const GPS: &str = "gps";
    pub const EDITED: &str = "edited";
    pub const LOCAL_LAYERS: &str = "local_layers";
    pub const TRASHED: &str = "trashed";
    /// A country without its ISO code.
    pub const NO_COUNTRY_CODE: &str = "no_country_code";
    /// Measured by the analysis (stage 3).
    pub const ANALYSED: &str = "analysed";
    /// Looks the same as another image.
    pub const DUPLICATE: &str = "duplicate";
    /// One of a burst.
    pub const SERIES: &str = "series";
}

/// The place's path and every level above it, down to its last known level:
/// `Poland`, `Poland|—`, `Poland|—|Kraków`. None for a photo with no place at all.
pub fn place_tree(place: &c1model::Place) -> Vec<String> {
    let levels = [&place.country, &place.state, &place.city, &place.location];
    let name = |l: &Option<String>| l.as_deref().map(|s| s.trim().replace('|', "/")).filter(|s| !s.is_empty());
    let Some(last) = levels.iter().rposition(|l| name(l).is_some()) else { return Vec::new() };
    let mut path = String::new();
    let mut out = Vec::with_capacity(last + 1);
    for (i, level) in levels[..=last].iter().enumerate() {
        if i > 0 {
            path.push('|');
        }
        path.push_str(&name(level).unwrap_or_else(|| NO_PLACE.to_owned()));
        out.push(path.clone());
    }
    out
}

#[derive(Clone)]
pub struct Fields {
    pub row: Field,
    pub ext_id: Field,
    pub asset: Field,
    pub text: Field,
    pub filename: Field,
    pub folder: Field,
    pub captured: Field,
    /// Sort keys: the value, then the row, so photos that tie keep their order when one
    /// of them is edited (an edit re-adds its document at the end of the index).
    pub captured_sort: Field,
    pub imported: Field,
    pub imported_sort: Field,
    pub rating: Field,
    pub rating_sort: Field,
    pub iso: Field,
    pub aperture: Field,
    pub shutter: Field,
    pub focal: Field,
    pub lat: Field,
    pub lon: Field,
    pub variant_index: Field,
    pub kw_count: Field,
    /// From the analysis, for sorting and ranges.
    pub sharpness: Field,
    pub brightness: Field,
    dims: Vec<(Dim, Field)>,
}

impl Fields {
    pub fn dim(&self, dim: Dim) -> Field {
        self.dims.iter().find(|(d, _)| *d == dim).map(|(_, f)| *f).expect("every Dim has a field")
    }
}

pub fn build_schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let num = || INDEXED | FAST;
    let text = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default().set_tokenizer(FOLDED).set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let mut dims = Vec::new();
    for dim in Dim::ALL {
        dims.push((dim, b.add_text_field(dim.field_name(), STRING | FAST)));
    }
    let fields = Fields {
        row: b.add_u64_field("row", num()),
        ext_id: b.add_text_field("ext_id", STRING),
        asset: b.add_text_field("asset", STRING),
        text: b.add_text_field("text", text),
        filename: b.add_text_field("filename", STRING | FAST),
        folder: b.add_text_field("folder", STRING),
        captured: b.add_i64_field("captured", INDEXED),
        captured_sort: b.add_i64_field("captured_sort", FAST),
        imported: b.add_i64_field("imported", num()),
        imported_sort: b.add_i64_field("imported_sort", FAST),
        rating: b.add_u64_field("rating", num()),
        rating_sort: b.add_u64_field("rating_sort", FAST),
        iso: b.add_u64_field("iso", num()),
        aperture: b.add_f64_field("aperture", num()),
        shutter: b.add_f64_field("shutter", num()),
        focal: b.add_f64_field("focal", num()),
        lat: b.add_f64_field("lat", num()),
        lon: b.add_f64_field("lon", num()),
        variant_index: b.add_u64_field("variant_index", num()),
        kw_count: b.add_u64_field("kw_count", num()),
        sharpness: b.add_f64_field("sharpness", num()),
        brightness: b.add_f64_field("brightness", num()),
        dims,
    };
    (b.build(), fields)
}

pub fn register_tokenizers(index: &Index) {
    let folded =
        TextAnalyzer::builder(SimpleTokenizer::default()).filter(LowerCaser).filter(AsciiFoldingFilter).build();
    index.tokenizers().register(FOLDED, folded);
}

/// `H:\fotos\2026` → `h:/fotos/2026`: the form folder filters compare in.
pub fn normalise_folder(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// `H:\fotos\2026\a.raf` → `H:`, `H:/fotos`, `H:/fotos/2026`: the folders holding a
/// file, outermost first, with forward slashes and the path's own case.
pub fn folder_tree(path: &str) -> Vec<String> {
    let mut parts: Vec<&str> = path.split(['\\', '/']).filter(|p| !p.is_empty()).collect();
    parts.pop();
    let mut prefix = if path.starts_with(['\\', '/']) { "/".to_owned() } else { String::new() };
    let mut out = Vec::new();
    for part in parts {
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        prefix.push_str(part);
        out.push(prefix.clone());
    }
    out
}

fn local_date(ms: i64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_millis_opt(ms).single()
}

const DAY_MS: i64 = 86_400_000;

/// Local days of many times. The time zone is asked once or twice per day rather than
/// once per time: on Windows each asking costs microseconds, a second over the library.
#[derive(Default)]
pub struct LocalDays {
    /// UTC day → the local offset (ms) at its start and at its end.
    offsets: HashMap<i64, (i64, i64)>,
}

impl LocalDays {
    fn ask(ms: i64) -> Option<i64> {
        let utc = chrono::DateTime::from_timestamp_millis(ms)?.naive_utc();
        Some(i64::from(Local.offset_from_utc_datetime(&utc).local_minus_utc()) * 1000)
    }

    fn offset(&mut self, ms: i64) -> Option<i64> {
        let day = ms.div_euclid(DAY_MS);
        let (start, end) = match self.offsets.get(&day) {
            Some(known) => *known,
            None => {
                let known = (Self::ask(day * DAY_MS)?, Self::ask(day * DAY_MS + DAY_MS - 1)?);
                self.offsets.insert(day, known);
                known
            }
        };
        // A day the clocks change on is asked per time.
        if start == end { Some(start) } else { Self::ask(ms) }
    }

    /// The local day of a time, as days since 1970-01-01: the same number all day.
    pub fn day(&mut self, ms: i64) -> Option<i64> {
        Some((ms + self.offset(ms)?).div_euclid(DAY_MS))
    }

    /// The local midnight that starts the day of a time.
    pub fn midnight(&mut self, ms: i64) -> Option<i64> {
        let day = self.day(ms)?;
        // The offset at midnight itself, which differs from `ms`'s on a day the clocks change.
        let guess = day * DAY_MS - self.offset(ms)?;
        Some(day * DAY_MS - self.offset(guess)?)
    }
}

/// The capture time back from a `captured_sort` value; None for a photo without one.
pub fn captured_of_sort_key(key: i64) -> Option<i64> {
    (key >= i64::MIN + (1 << 20)).then_some(key >> 20)
}

/// A time (ms) with the row in its low 20 bits, so equal times sort by row. Photos
/// without a time sort first, also by row.
fn sort_key(ms: Option<i64>, row: u64) -> i64 {
    let tie = (row & 0xf_ffff) as i64;
    match ms {
        Some(ms) => ms.saturating_mul(1 << 20).saturating_add(tie),
        None => i64::MIN + tie,
    }
}

pub fn document(f: &Fields, row: u64, photo: &Photo, look: Option<&Look>) -> TantivyDocument {
    let mut doc = TantivyDocument::default();
    let mut dim = |d: Dim, value: &str| doc_add_str(&mut doc, f.dim(d), value);
    dim(Dim::Source, &photo.id.source.0);
    for kw in &photo.keywords {
        dim(Dim::Keyword, &kw.to_pipe());
    }
    let mut tree: Vec<String> =
        photo.keywords.iter().flat_map(|k| k.ancestors_and_self().collect::<Vec<_>>()).collect();
    tree.sort();
    tree.dedup();
    for path in &tree {
        dim(Dim::KeywordTree, path);
    }
    for coll in &photo.collections {
        dim(Dim::Collection, &coll.0);
    }
    for (d, value) in [
        (Dim::Camera, &photo.exif.camera_model),
        (Dim::Lens, &photo.exif.lens),
        (Dim::Country, &photo.place.country),
        (Dim::State, &photo.place.state),
        (Dim::City, &photo.place.city),
    ] {
        if let Some(v) = value {
            dim(d, v);
        }
    }
    if let Some(date) = photo.captured_at.and_then(local_date) {
        dim(Dim::Year, &date.year().to_string());
        dim(Dim::Month, &format!("{:04}-{:02}", date.year(), date.month()));
        dim(Dim::Week, &format!("{:04}-{:02}-W{:02}", date.year(), date.month(), date.iso_week().week()));
    }
    dim(Dim::Rating, &photo.rating.map_or("none".to_owned(), |r| r.to_string()));
    dim(Dim::Color, photo.color.map_or("none", |c| c.name()));
    if !photo.file.ext.is_empty() {
        dim(Dim::Ext, &photo.file.ext);
    }
    for style in &photo.edit.styles {
        dim(Dim::Style, style);
    }
    let flags = [
        (flag::RATED, photo.rating.is_some()),
        (flag::COLORED, photo.color.is_some()),
        (flag::KEYWORDED, !photo.keywords.is_empty()),
        (flag::GPS, photo.geo.is_some()),
        (flag::EDITED, photo.edit.edited),
        (flag::LOCAL_LAYERS, photo.edit.local_layers > 0),
        (flag::TRASHED, photo.trashed),
        (flag::NO_COUNTRY_CODE, photo.place.country.is_some() && photo.place.country_code.is_none()),
        (flag::ANALYSED, look.is_some()),
        (flag::DUPLICATE, look.is_some_and(|l| l.duplicates.is_some())),
        (flag::SERIES, look.is_some_and(|l| l.series.is_some())),
    ];
    for (name, set) in flags {
        if set {
            dim(Dim::Flag, name);
        }
    }
    if let Some(look) = look {
        let a = &look.analysis;
        dim(Dim::Shape, a.shape());
        for hue in a.hues() {
            dim(Dim::Hue, hue);
        }
        for light in a.light() {
            dim(Dim::Light, light);
        }
        dim(Dim::Focus, a.focus());
        for group in [&look.duplicates, &look.series].into_iter().flatten() {
            dim(Dim::Group, group);
        }
    }

    for place in place_tree(&photo.place) {
        doc.add_text(f.dim(Dim::PlaceTree), &place);
    }
    doc.add_u64(f.row, row);
    doc.add_text(f.ext_id, &photo.id.ext);
    doc.add_text(f.asset, &photo.asset.0);
    doc.add_text(f.filename, photo.file.name.to_lowercase());
    if let Some(path) = &photo.file.path {
        for folder in folder_tree(path) {
            doc.add_text(f.dim(Dim::Folder), &folder);
        }
        let folder = normalise_folder(path.rsplit_once(['\\', '/']).map_or("", |(dir, _)| dir));
        let mut prefix = String::new();
        for (i, part) in folder.split('/').enumerate() {
            if i > 0 {
                prefix.push('/');
            }
            prefix.push_str(part);
            doc.add_text(f.folder, &prefix);
        }
    }

    // One text field holds everything free text searches, as the FTS table did.
    let mut text = vec![photo.file.name.clone()];
    text.extend(photo.keywords.iter().map(|k| k.0.join(" ")));
    let i = &photo.iptc;
    let p = &photo.place;
    for value in [&i.description, &i.headline, &i.title, &p.city, &p.state, &p.country, &p.location, &i.creator]
        .into_iter()
        .flatten()
    {
        text.push(value.clone());
    }
    doc.add_text(f.text, text.join("\n"));

    doc.add_i64(f.captured_sort, sort_key(photo.captured_at, row));
    doc.add_i64(f.imported_sort, sort_key(photo.imported_at, row));
    doc.add_u64(f.rating_sort, (u64::from(photo.rating.unwrap_or(0)) << 32) | (row & 0xffff_ffff));
    if let Some(ms) = photo.captured_at {
        doc.add_i64(f.captured, ms);
    }
    if let Some(ms) = photo.imported_at {
        doc.add_i64(f.imported, ms);
    }
    if let Some(r) = photo.rating {
        doc.add_u64(f.rating, u64::from(r));
    }
    if let Some(v) = photo.exif.iso {
        doc.add_u64(f.iso, u64::from(v));
    }
    for (field, value) in
        [(f.aperture, photo.exif.aperture), (f.shutter, photo.exif.shutter), (f.focal, photo.exif.focal)]
    {
        if let Some(v) = value {
            doc.add_f64(field, v);
        }
    }
    if let Some(geo) = &photo.geo {
        doc.add_f64(f.lat, geo.lat);
        doc.add_f64(f.lon, geo.lon);
    }
    doc.add_u64(f.variant_index, u64::from(photo.variant_index));
    doc.add_u64(f.kw_count, photo.keywords.len() as u64);
    if let Some(look) = look {
        doc.add_f64(f.sharpness, f64::from(look.analysis.sharpness));
        doc.add_f64(f.brightness, f64::from(look.analysis.brightness));
    }
    doc
}

fn doc_add_str(doc: &mut TantivyDocument, field: Field, value: &str) {
    doc.add_text(field, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_days_agree_with_the_time_zone() {
        let mut days = LocalDays::default();
        // Every 7 hours over two years, through clock changes wherever the tests run.
        for i in 0..2_500 {
            let ms = 1_700_000_000_000 + i * 7 * 3_600_000;
            let date = local_date(ms).unwrap();
            let midnight = Local
                .from_local_datetime(&date.date_naive().and_hms_opt(0, 0, 0).unwrap())
                .earliest()
                .unwrap()
                .timestamp_millis();
            assert_eq!(days.midnight(ms), Some(midnight), "at {ms}");
            assert_eq!(days.day(ms), days.day(midnight), "at {ms}");
            assert_ne!(days.day(ms), days.day(midnight - 1), "at {ms}");
        }
    }

    #[test]
    fn folders_above_a_file() {
        assert_eq!(folder_tree(r"H:\fotos\2026\a.raf"), ["H:", "H:/fotos", "H:/fotos/2026"]);
        assert_eq!(folder_tree("/home/me/a.jpg"), ["/home", "/home/me"]);
        assert!(folder_tree("a.jpg").is_empty());
        // Each one, normalised, is what a folder filter compares against.
        assert_eq!(normalise_folder("H:/fotos/2026"), "h:/fotos/2026");
    }
}
