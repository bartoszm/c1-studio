//! The search document: one per photo.
//!
//! Most dimensions are raw string fields that are both indexed (exact-term filters) and
//! fast (columnar, for counting values per facet). Numeric fields are indexed for range
//! filters and fast for sorting. A value that is missing is simply not added, so a range
//! filter never matches a photo without that value — the same as SQL `NULL`.

use c1model::Photo;
use chrono::{Datelike, Local, TimeZone};
use tantivy::schema::{
    FAST, Field, INDEXED, IndexRecordOption, STRING, Schema, TextFieldIndexing, TextOptions,
};
use tantivy::tokenizer::{AsciiFoldingFilter, LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{Index, TantivyDocument};

/// Bump when fields change; the library rebuilds the index from its store.
pub const SCHEMA_VERSION: u32 = 1;

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
    Month,
    /// `1`..`5`, or `none`.
    Rating,
    /// Colour name, or `none`.
    Color,
    Ext,
    Style,
    Flag,
}

impl Dim {
    pub const ALL: [Dim; 16] = [
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
        Dim::Rating,
        Dim::Color,
        Dim::Ext,
        Dim::Style,
        Dim::Flag,
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
            Dim::Rating => "rating_s",
            Dim::Color => "color",
            Dim::Ext => "ext",
            Dim::Style => "style",
            Dim::Flag => "flag",
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
    pub captured_sort: Field,
    pub imported: Field,
    pub rating: Field,
    pub iso: Field,
    pub aperture: Field,
    pub shutter: Field,
    pub focal: Field,
    pub lat: Field,
    pub lon: Field,
    pub variant_index: Field,
    pub kw_count: Field,
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
        rating: b.add_u64_field("rating", num()),
        iso: b.add_u64_field("iso", num()),
        aperture: b.add_f64_field("aperture", num()),
        shutter: b.add_f64_field("shutter", num()),
        focal: b.add_f64_field("focal", num()),
        lat: b.add_f64_field("lat", num()),
        lon: b.add_f64_field("lon", num()),
        variant_index: b.add_u64_field("variant_index", num()),
        kw_count: b.add_u64_field("kw_count", num()),
        dims,
    };
    (b.build(), fields)
}

pub fn register_tokenizers(index: &Index) {
    let folded = TextAnalyzer::builder(SimpleTokenizer::default()).filter(LowerCaser).filter(AsciiFoldingFilter).build();
    index.tokenizers().register(FOLDED, folded);
}

/// `H:\fotos\2026` → `h:/fotos/2026`: the form folder filters compare in.
pub fn normalise_folder(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

fn local_date(ms: i64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_millis_opt(ms).single()
}

pub fn document(f: &Fields, row: u64, photo: &Photo) -> TantivyDocument {
    let mut doc = TantivyDocument::default();
    let mut dim = |d: Dim, value: &str| doc_add_str(&mut doc, f.dim(d), value);
    dim(Dim::Source, &photo.id.source.0);
    for kw in &photo.keywords {
        dim(Dim::Keyword, &kw.to_pipe());
    }
    let mut tree: Vec<String> = photo.keywords.iter().flat_map(|k| k.ancestors_and_self().collect::<Vec<_>>()).collect();
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
    ];
    for (name, set) in flags {
        if set {
            dim(Dim::Flag, name);
        }
    }

    doc.add_u64(f.row, row);
    doc.add_text(f.ext_id, &photo.id.ext);
    doc.add_text(f.asset, &photo.asset.0);
    doc.add_text(f.filename, photo.file.name.to_lowercase());
    if let Some(path) = &photo.file.path {
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

    doc.add_i64(f.captured_sort, photo.captured_at.unwrap_or(i64::MIN));
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
    for (field, value) in [(f.aperture, photo.exif.aperture), (f.shutter, photo.exif.shutter), (f.focal, photo.exif.focal)] {
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
    doc
}

fn doc_add_str(doc: &mut TantivyDocument, field: Field, value: &str) {
    doc.add_text(field, value);
}
