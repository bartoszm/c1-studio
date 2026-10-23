//! Photos described by a vision model: a caption, a headline, keywords taken from the
//! catalog's own vocabulary (and a few new ones), a rating and a guess at the place.
//!
//! A [`VisionProvider`] answers one photo at a time with a [`Reply`]: the checked
//! [`Suggestion`] and a [`RunRecord`] of what it cost. Nothing here writes anywhere; the
//! library keeps suggestions until the user accepts them into the edit queue.
//! [`claude::Claude`] calls the Claude API with a key; [`claude_code::ClaudeCode`] asks
//! the same of Claude Code signed in with a Claude plan. Gemini through `agy` follows
//! behind the same trait.

pub mod agy;
pub mod claude;
pub mod claude_code;
pub mod keys;
mod prompt;

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use prompt::{Brief, Fields, INSTRUCTIONS, Language, prompt_hash, schema, system_prompt};

/// Bumped when the prompt or the schema changes: photos described by another version are
/// described again when asked.
pub const PROMPT_VERSION: u32 = 2;

/// One photo to describe: a JPEG of it and one line of what the catalog already knows
/// (file name, date, camera, place), which helps the model and costs a few tokens.
pub struct PhotoRequest {
    pub jpeg: Vec<u8>,
    pub context: String,
    /// Told when the call goes out and before it is tried again.
    pub progress: Option<Progress>,
}

/// Where a call has got to, for a run to show while it waits.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// The request went out (`attempt` from 1); the answer is awaited.
    Sent { attempt: u32 },
    /// Attempt `attempt` of at most `of` failed in a way that may pass; another follows.
    Retrying { attempt: u32, of: u32, why: String },
}

/// Hears a call's [`Step`]s. Called on the thread making the call.
pub type Progress = Arc<dyn Fn(Step) + Send + Sync>;

pub(crate) fn tell(progress: &Option<Progress>, step: Step) {
    if let Some(progress) = progress {
        progress(step);
    }
}

/// What the model answered, as the schema asks for it ([`schema`]). Unchecked.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawAnswer {
    pub caption_en: String,
    pub caption_pl: String,
    pub headline: String,
    pub subject: String,
    pub keywords: Vec<String>,
    pub keywords_new: Vec<String>,
    pub rating: i64,
    pub quality: i64,
    pub composition: i64,
    pub confidence: f64,
    pub location_guess: RawPlace,
}

/// Where the photo was taken, level by level, as the model gave it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawPlace {
    pub country: RawLevel,
    pub state: RawLevel,
    pub city: RawLevel,
    pub location: RawLevel,
    pub coordinates: RawPoint,
}

/// One level of a place: its name and the probability it is right.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawLevel {
    pub name: String,
    pub p: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawPoint {
    pub lat: f64,
    pub lon: f64,
    pub p: f64,
    /// How far off the point may be.
    pub radius_km: f64,
}

/// A suggestion for one photo, checked: empty texts are `None`, numbers are in range,
/// keywords are split into ones the catalog has (by their full path) and new ones.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub caption_en: Option<String>,
    pub caption_pl: Option<String>,
    pub headline: Option<String>,
    pub subject: Option<String>,
    /// Keywords the catalog already has, as pipe paths (`places|Poland|Kraków`).
    pub keywords: Vec<String>,
    /// Keywords the catalog does not have yet.
    pub keywords_new: Vec<String>,
    /// 1..=5.
    pub rating: Option<u8>,
    pub quality: Option<u8>,
    pub composition: Option<u8>,
    /// 0..1.
    pub confidence: f32,
    pub place: Option<PlaceGuess>,
}

/// Where the model thinks the photo was taken: each level it named, from the country
/// down to a location (a district, a landmark), with the probability that it is right, and
/// a point. A deeper level is never more likely than the one above it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaceGuess {
    pub country: Option<Guess>,
    pub state: Option<Guess>,
    pub city: Option<Guess>,
    pub location: Option<Guess>,
    /// The country's ISO code, once the gazetteer knows the country.
    pub country_code: Option<String>,
    pub point: Option<PointGuess>,
}

/// A place level guessed: a name and the probability that it is right (0..1].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guess {
    pub name: String,
    pub p: f32,
}

/// A position guessed, within `radius_km`, with the probability that it is right.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointGuess {
    pub lat: f64,
    pub lon: f64,
    pub p: f32,
    pub radius_km: f32,
}

impl PlaceGuess {
    /// The levels, country first.
    pub fn levels(&self) -> [&Option<Guess>; 4] {
        [&self.country, &self.state, &self.city, &self.location]
    }

    pub fn levels_mut(&mut self) -> [&mut Option<Guess>; 4] {
        [&mut self.country, &mut self.state, &mut self.city, &mut self.location]
    }

    pub fn is_empty(&self) -> bool {
        self.levels().iter().all(|l| l.is_none()) && self.point.is_none()
    }

    /// Lowers each level to the probability of the one above it, where it was higher.
    /// Returns the levels lowered, with what they had.
    pub fn coherent(&mut self) -> Vec<(String, f32, f32)> {
        let mut lowered = Vec::new();
        let mut above = 1.0f32;
        for level in self.levels_mut().into_iter().flatten() {
            if level.p > above {
                lowered.push((level.name.clone(), level.p, above));
                level.p = above;
            }
            above = level.p;
        }
        lowered
    }
}

/// Checks a place as the model gave it: names trimmed, probabilities in range, a level
/// no more likely than the one above it, a point on the globe. `None` when nothing is left.
fn place_guess(raw: &RawPlace, warnings: &mut Vec<String>) -> Option<PlaceGuess> {
    let level = |l: &RawLevel| {
        let name = l.name.trim();
        let p = l.p.clamp(0.0, 1.0) as f32;
        (!name.is_empty() && p > 0.0).then(|| Guess { name: name.to_string(), p })
    };
    let c = &raw.coordinates;
    let on_globe = (-90.0..=90.0).contains(&c.lat) && (-180.0..=180.0).contains(&c.lon);
    let point = (c.p > 0.0 && on_globe && (c.lat, c.lon) != (0.0, 0.0)).then(|| PointGuess {
        lat: c.lat,
        lon: c.lon,
        p: c.p.clamp(0.0, 1.0) as f32,
        radius_km: c.radius_km.max(0.0) as f32,
    });
    if c.p > 0.0 && !on_globe {
        warnings.push(format!("dropped the position {}, {}: not on the globe", c.lat, c.lon));
    }
    let mut guess = PlaceGuess {
        country: level(&raw.country),
        state: level(&raw.state),
        city: level(&raw.city),
        location: level(&raw.location),
        country_code: None,
        point,
    };
    for (name, was, now) in guess.coherent() {
        warnings.push(format!(
            "{name}: {:.0}% lowered to {:.0}%, as likely as the level above",
            was * 100.0,
            now * 100.0
        ));
    }
    (!guess.is_empty()).then_some(guess)
}

/// What one call cost and took, kept with the suggestion: the set to judge prompts and
/// models by later.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub provider: String,
    pub model: String,
    /// Hash of the system prompt and schema the call was made with.
    pub prompt: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    /// US dollars, from the model's list price.
    pub cost: f64,
    pub millis: u64,
    /// The answer as the model gave it, before checking.
    pub raw: String,
}

#[derive(Debug)]
pub struct Reply {
    pub suggestion: Suggestion,
    pub warnings: Vec<String>,
    pub record: RunRecord,
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("no API key: add one in Settings > AI")]
    NoKey,
    #[error("the API key was refused: {0}")]
    BadKey(String),
    #[error("{status}: {message}")]
    Http { status: u16, message: String, retry_after: Option<u64> },
    #[error("network: {0}")]
    Network(String),
    #[error("the model declined this photo{}", .0.as_deref().map(|c| format!(" ({c})")).unwrap_or_default())]
    Refused(Option<String>),
    #[error("the answer was cut off")]
    Truncated,
    #[error("the answer could not be read: {0}")]
    BadReply(String),
    #[error("key store: {0}")]
    Keys(String),
    /// The command-line tool (Claude Code, agy) is missing, not signed in or would not start.
    #[error("{0}")]
    Cli(String),
    #[error("the plan's usage limit is reached: {0}")]
    Limit(String),
    #[error("stopped")]
    Stopped,
}

impl AiError {
    /// Worth trying again after a pause: rate limits, overload, server errors, the network.
    pub fn retryable(&self) -> bool {
        match self {
            AiError::Http { status, .. } => *status == 408 || *status == 429 || *status >= 500,
            AiError::Network(_) => true,
            _ => false,
        }
    }

    /// No other photo would fare better (no key, no Claude Code, no credit, a model the
    /// account cannot use): a run stops rather than fail every photo the same way.
    pub fn stops_run(&self) -> bool {
        match self {
            AiError::NoKey | AiError::BadKey(_) | AiError::Cli(_) | AiError::Limit(_) | AiError::Stopped => true,
            AiError::Http { status, message, .. } => {
                *status == 404 || message.to_lowercase().contains("credit balance")
            }
            _ => false,
        }
    }
}

/// A photo that got no suggestion, with the call's record when one was made and paid
/// for (a refusal, a cut or unreadable answer): spending counts it all the same.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct Failed {
    pub error: AiError,
    pub record: Option<Box<RunRecord>>,
}

impl From<AiError> for Failed {
    fn from(error: AiError) -> Self {
        Failed { error, record: None }
    }
}

pub trait VisionProvider: Send + Sync {
    /// `claude`, `gemini`.
    fn id(&self) -> &str;
    fn model(&self) -> &str;
    /// Hash of the prompt and schema in use: with [`VisionProvider::model`], says whether
    /// a stored suggestion was made the same way.
    fn prompt_hash(&self) -> u64;
    fn describe(&self, photo: &PhotoRequest) -> Result<Reply, Failed>;

    /// Photos go to it as contact sheets ([`VisionProvider::describe_sheet`]).
    fn sheets(&self) -> bool {
        false
    }

    /// Describes the numbered photos of a contact sheet in one call.
    fn describe_sheet(&self, _sheet: &SheetRequest) -> Result<SheetReply, Failed> {
        Err(AiError::BadReply("this provider takes one photo at a time".into()).into())
    }
}

/// A contact sheet to describe: a JPEG of up to 36 photos numbered from 1, row by row,
/// and what the catalog knows of each, in the same order.
pub struct SheetRequest {
    pub jpeg: Vec<u8>,
    pub contexts: Vec<String>,
    /// Told when the call goes out and before it is tried again.
    pub progress: Option<Progress>,
}

/// What a sheet's call gave: per photo (in the request's order) its checked suggestion
/// and warnings, `None` where the answer left it out; the sheet's own warnings; the call.
#[derive(Debug)]
pub struct SheetReply {
    pub replies: Vec<Option<(Suggestion, Vec<String>)>>,
    pub warnings: Vec<String>,
    pub record: RunRecord,
}

/// The keywords a catalog has, by name: what a model's answer is matched against.
/// A name used on several paths maps to the one most photos carry.
#[derive(Default)]
pub struct Vocabulary {
    by_name: HashMap<String, String>,
}

impl Vocabulary {
    /// `paths` are pipe paths with their photo counts.
    pub fn new<'a>(paths: impl IntoIterator<Item = (&'a str, u64)>) -> Self {
        let mut best: HashMap<String, (String, u64)> = HashMap::new();
        for (path, count) in paths {
            let Some(leaf) = path.rsplit('|').next() else { continue };
            let key = fold(leaf);
            match best.get(&key) {
                Some((_, n)) if *n >= count => {}
                _ => {
                    best.insert(key, (path.to_string(), count));
                }
            }
        }
        Vocabulary { by_name: best.into_iter().map(|(k, (p, _))| (k, p)).collect() }
    }

    pub fn path(&self, name: &str) -> Option<&str> {
        self.by_name.get(&fold(name)).map(String::as_str)
    }
}

/// What a run leaves out of the vocabulary a model chooses from: keyword paths, each with
/// all below it, and, if asked, keywords named only with numbers ("01", "2023-05"), which
/// sort a tree but say nothing of a photo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LeftOut {
    /// Pipe paths.
    pub paths: Vec<String>,
    pub numbers: bool,
}

impl Default for LeftOut {
    fn default() -> Self {
        LeftOut { paths: Vec::new(), numbers: true }
    }
}

impl LeftOut {
    /// The keyword at pipe path `path` is not offered.
    pub fn covers(&self, path: &str) -> bool {
        let name = path.rsplit('|').next().unwrap_or(path);
        (self.numbers && number_name(name))
            || self
                .paths
                .iter()
                .any(|p| path.strip_prefix(p.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with('|')))
    }
}

/// A name made of digits and separators only: "01", "2023", "2023-05", "3.".
pub fn number_name(name: &str) -> bool {
    let name = name.trim();
    name.chars().any(|c| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_digit() || matches!(c, ' ' | '-' | '_' | '.' | '/' | '#'))
}

fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

fn text(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn score(n: i64) -> Option<u8> {
    (1..=5).contains(&n).then_some(n as u8)
}

/// At most this many new keywords per photo: more is noise to review.
pub const MAX_NEW_KEYWORDS: usize = 3;

/// Checks an answer (as the legacy `vision.validate` did): drops keywords with a comma
/// or a pipe (either would split into several keywords in the catalog), maps names onto
/// the catalog's own paths, keeps scores in range. Returns the warnings with it.
pub fn validate(raw: &RawAnswer, vocabulary: &Vocabulary) -> (Suggestion, Vec<String>) {
    let mut warnings = Vec::new();
    let mut keywords: Vec<String> = Vec::new();
    let mut new: Vec<String> = Vec::new();
    for (name, proposed) in raw.keywords.iter().map(|k| (k, false)).chain(raw.keywords_new.iter().map(|k| (k, true))) {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if name.contains(',') || name.contains('|') {
            warnings.push(format!("dropped keyword {name:?}: a comma or a pipe would split it"));
            continue;
        }
        if let Some(path) = vocabulary.path(name) {
            if !keywords.iter().any(|k| k == path) {
                keywords.push(path.to_string());
            }
        } else if !new.iter().any(|k| fold(k) == fold(name)) {
            if !proposed {
                warnings.push(format!("{name:?} is not in the catalog: suggested as new"));
            }
            new.push(name.to_string());
        }
    }
    if new.len() > MAX_NEW_KEYWORDS {
        warnings.push(format!("{} new keywords, kept {MAX_NEW_KEYWORDS}", new.len()));
        new.truncate(MAX_NEW_KEYWORDS);
    }
    let place = place_guess(&raw.location_guess, &mut warnings);
    let suggestion = Suggestion {
        caption_en: text(&raw.caption_en),
        caption_pl: text(&raw.caption_pl),
        headline: text(&raw.headline),
        subject: text(&raw.subject),
        keywords,
        keywords_new: new,
        rating: score(raw.rating),
        quality: score(raw.quality),
        composition: score(raw.composition),
        confidence: raw.confidence.clamp(0.0, 1.0) as f32,
        place,
    };
    (suggestion, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_left_out_with_all_below_them_and_numbers() {
        let left = LeftOut { paths: vec!["trips|asia".into(), "misc".into()], numbers: true };
        assert!(left.covers("trips|asia"));
        assert!(left.covers("trips|asia|beach"));
        assert!(!left.covers("trips|asian food"), "a sibling that starts the same is kept");
        assert!(left.covers("misc|x"));
        assert!(!left.covers("miscellany"));
        assert!(left.covers("places|01"), "a number name, wherever it is");
        assert!(left.covers("2023-05"));
        assert!(!left.covers("places|01 beach"));
        assert!(!left.covers("trips|b&w"));
        assert!(!LeftOut { paths: Vec::new(), numbers: false }.covers("places|01"));
        assert!(!number_name("") && !number_name("--"));
    }

    fn vocabulary() -> Vocabulary {
        Vocabulary::new([("street", 40), ("places|Poland|Kraków", 12), ("old|Kraków", 2), ("people|child", 7)])
    }

    #[test]
    fn keywords_map_onto_the_catalog_paths() {
        let raw = RawAnswer {
            keywords: vec!["Street".into(), "kraków".into(), "tram".into()],
            keywords_new: vec!["Tram".into(), "dusk".into()],
            ..Default::default()
        };
        let (s, warnings) = validate(&raw, &vocabulary());
        assert_eq!(s.keywords, vec!["street", "places|Poland|Kraków"]);
        assert_eq!(s.keywords_new, vec!["tram", "dusk"]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn commas_pipes_and_too_many_new_keywords_are_dropped() {
        let raw = RawAnswer {
            keywords_new: vec!["a, b".into(), "x|y".into(), "one".into(), "two".into(), "three".into(), "four".into()],
            ..Default::default()
        };
        let (s, warnings) = validate(&raw, &vocabulary());
        assert_eq!(s.keywords_new, vec!["one", "two", "three"]);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn scores_and_texts_are_checked() {
        let raw = RawAnswer {
            caption_en: "  A tram at dusk. ".into(),
            headline: " ".into(),
            rating: 9,
            quality: 4,
            composition: 0,
            confidence: 1.7,
            ..Default::default()
        };
        let (s, _) = validate(&raw, &vocabulary());
        assert_eq!(s.caption_en.as_deref(), Some("A tram at dusk."));
        assert_eq!(s.headline, None);
        assert_eq!((s.rating, s.quality, s.composition), (None, Some(4), None));
        assert_eq!(s.confidence, 1.0);
        assert_eq!(s.place, None);
    }

    #[test]
    fn a_place_keeps_named_levels_no_likelier_than_the_one_above() {
        let level = |name: &str, p: f64| RawLevel { name: name.into(), p };
        let raw = RawAnswer {
            location_guess: RawPlace {
                country: level(" Poland ", 0.9),
                state: level("", 0.8),
                city: level("Kraków", 1.4),
                location: level("Kazimierz", 0.0),
                coordinates: RawPoint { lat: 50.05, lon: 19.94, p: 0.5, radius_km: 3.0 },
            },
            ..Default::default()
        };
        let (s, warnings) = validate(&raw, &vocabulary());
        let place = s.place.unwrap();
        assert_eq!(place.country, Some(Guess { name: "Poland".into(), p: 0.9 }));
        assert_eq!(place.state, None, "no name");
        assert_eq!(place.city, Some(Guess { name: "Kraków".into(), p: 0.9 }), "capped at the country's");
        assert_eq!(place.location, None, "p 0");
        assert_eq!(place.point.as_ref().map(|p| (p.lat, p.p)), Some((50.05, 0.5)));
        assert_eq!(warnings.len(), 1, "{warnings:?}");

        let off = RawAnswer {
            location_guess: RawPlace {
                coordinates: RawPoint { lat: 95.0, lon: 0.0, p: 0.5, radius_km: 1.0 },
                ..Default::default()
            },
            ..Default::default()
        };
        let (s, warnings) = validate(&off, &vocabulary());
        assert_eq!(s.place, None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }
}
