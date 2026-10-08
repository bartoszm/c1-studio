//! What a photo is, independent of where it came from.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Identifies one data source instance, e.g. `c1:fiuji-ef90be38`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceId(pub String);

/// Globally unique photo id: the source plus the id the source itself uses.
///
/// "Photo" here is what Capture One calls a *variant*: one file can have several
/// variants (a colour and a black-and-white treatment), each rated and tagged on its own.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PhotoId {
    pub source: SourceId,
    pub ext: String,
}

impl PhotoId {
    pub fn new(source: &SourceId, ext: impl Into<String>) -> Self {
        Self { source: source.clone(), ext: ext.into() }
    }

    /// A single string usable as a storage or index key. `\u{1f}` never occurs in ids.
    pub fn key(&self) -> String {
        format!("{}\u{1f}{}", self.source.0, self.ext)
    }

    pub fn from_key(key: &str) -> Option<Self> {
        let (source, ext) = key.split_once('\u{1f}')?;
        Some(Self { source: SourceId(source.to_owned()), ext: ext.to_owned() })
    }
}

impl fmt::Display for PhotoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.source.0, self.ext)
    }
}

/// Shared by every variant of one underlying file (Capture One's `ZIMAGE`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CollectionId(pub String);

/// A keyword with its hierarchy: `["places", "poland", "wroclaw"]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct KeywordPath(pub Vec<String>);

impl KeywordPath {
    /// Parses `a|b|c`, the separator Capture One uses.
    pub fn from_pipe(path: &str) -> Self {
        Self(path.split('|').map(str::to_owned).collect())
    }

    pub fn to_pipe(&self) -> String {
        self.0.join("|")
    }

    /// `a`, `a|b`, `a|b|c`: the keyword and everything it sits under.
    pub fn ancestors_and_self(&self) -> impl Iterator<Item = String> + '_ {
        (1..=self.0.len()).map(|n| self.0[..n].join("|"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorLabel {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Pink,
    Purple,
}

impl ColorLabel {
    pub const ALL: [ColorLabel; 7] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Pink,
        Self::Purple,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Pink => "pink",
            Self::Purple => "purple",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name().eq_ignore_ascii_case(name))
    }

    /// 1..=7 in Capture One's order; 0 means no label.
    pub fn index(self) -> u8 {
        Self::ALL.iter().position(|c| *c == self).unwrap() as u8 + 1
    }

    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(usize::from(index).checked_sub(1)?).copied()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    /// Lower-case extension without the dot.
    pub ext: String,
    /// Where the original lives, in this machine's native form (`H:\fotos\x.RAF`).
    pub path: Option<String>,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Exif {
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<u32>,
    pub aperture: Option<f64>,
    /// Seconds, e.g. 0.004 for 1/250.
    pub shutter: Option<f64>,
    /// Millimetres.
    pub focal: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Iptc {
    pub title: Option<String>,
    pub headline: Option<String>,
    pub description: Option<String>,
    pub creator: Option<String>,
    pub copyright: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub location: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeoOrigin {
    Exif,
    Manual,
    Gpx,
    Geocoded,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    pub lat: f64,
    pub lon: f64,
    pub alt: Option<f64>,
    pub origin: GeoOrigin,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EditInfo {
    /// Develop settings differ from the defaults.
    pub edited: bool,
    pub styles: Vec<String>,
    pub local_layers: u32,
    pub retouch_layers: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Photo {
    pub id: PhotoId,
    pub asset: AssetId,
    /// Position among the asset's variants; 0 is the original.
    pub variant_index: u32,
    pub file: FileInfo,
    /// Unix epoch milliseconds.
    pub captured_at: Option<i64>,
    pub imported_at: Option<i64>,
    pub exif: Exif,
    /// 1..=5; `None` is unrated.
    pub rating: Option<u8>,
    pub color: Option<ColorLabel>,
    pub keywords: Vec<KeywordPath>,
    pub iptc: Iptc,
    pub place: Place,
    pub geo: Option<GeoPoint>,
    pub collections: Vec<CollectionId>,
    pub edit: EditInfo,
    pub trashed: bool,
}

impl Photo {
    /// A stable hash of every field: equal fingerprints mean nothing changed.
    pub fn fingerprint(&self) -> u64 {
        let bytes = postcard::to_stdvec(self).expect("Photo always serialises");
        xxhash_rust::xxh3::xxh3_64(&bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CollectionKind {
    Album,
    Smart,
    Group,
    Project,
    Folder,
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub id: CollectionId,
    pub name: String,
    pub kind: CollectionKind,
    pub parent: Option<CollectionId>,
    /// Display path from the root, `Group / Album`.
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeywordNode {
    pub path: KeywordPath,
    /// How many photos carry exactly this keyword.
    pub uses: u32,
}
