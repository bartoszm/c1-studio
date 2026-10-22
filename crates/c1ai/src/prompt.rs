//! The instructions and the answer's shape, the same for every photo of a run (so a
//! provider can cache them), ported from the legacy `vision.py` prompts. Which fields are
//! asked for, and the instructions themselves, are the user's to choose: the schema and
//! the list of fields follow the choice, the instructions can be replaced as a whole.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Pl,
    Both,
}

/// What a description holds. Fields left out are neither asked for nor answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fields {
    pub caption: bool,
    pub headline: bool,
    pub subject: bool,
    /// Keywords from the catalog's vocabulary.
    pub keywords: bool,
    /// A few keywords the vocabulary lacks.
    pub keywords_new: bool,
    pub rating: bool,
    /// Quality and composition, 1 to 5 each.
    pub scores: bool,
    /// City and country, when the photo shows them.
    pub place: bool,
}

impl Default for Fields {
    fn default() -> Self {
        Fields {
            caption: true,
            headline: true,
            subject: true,
            keywords: true,
            keywords_new: true,
            rating: true,
            scores: true,
            place: true,
        }
    }
}

impl Fields {
    pub fn any(&self) -> bool {
        let Fields { caption, headline, subject, keywords, keywords_new, rating, scores, place } = *self;
        caption || headline || subject || keywords || keywords_new || rating || scores || place
    }
}

/// What a run asks for, besides the photo: the fields, the caption languages, the
/// catalog's keyword names the model should choose from (most used first), and the
/// user's own instructions in place of [`INSTRUCTIONS`].
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Brief {
    pub language: Language,
    pub vocabulary: Vec<String>,
    pub fields: Fields,
    pub instructions: Option<String>,
}

/// How many of the catalog's keyword names go into the prompt.
pub const VOCABULARY_SIZE: usize = 400;

/// What the model is told before the fields: the part a user may rewrite.
pub const INSTRUCTIONS: &str = "You describe photographs from a photographer's Capture One catalog, one photo at a time. \
Your answer becomes suggested metadata that the photographer reviews before anything is saved, so be accurate rather than complete: \
leave a text field empty, or a list empty, when you cannot judge it from the photo.

Each photo comes with a line of catalog data: file name, date, camera, place, and what the photographer has already written \
(caption, headline, keywords, rating). It is context, not something to repeat. Treat the photographer's own metadata as reliable: \
build on the names, places and events it gives, do not contradict it unless the photo plainly shows otherwise, \
and do not suggest keywords the photo already has. Rate the photo on your own judgement; their rating is shown for context only.";

/// Each field's line in the prompt.
const CAPTION: &str = "- caption_en / caption_pl: two or three sentences on what is happening, the light and the mood, \
as a curator would write for a portfolio. Fill only the language(s) asked for below; leave the other empty.";
const HEADLINE: &str = "- headline: at most five words, in the first language asked for.";
const SUBJECT: &str = "- subject: the main subject in two to four words.";
const KEYWORDS: &str = "- keywords: up to ten keywords chosen from the photographer's vocabulary below, copied exactly. \
Prefer specific ones over general ones.";
const KEYWORDS_NEW: &str = "- keywords_new: at most three keywords that the vocabulary lacks and the photo clearly calls for. \
Single words or short phrases, no commas.";
const RATING: &str = "- rating: 1 to 5, how strong the photo is within a photographer's own work \
(3 is a sound keeper, 5 a portfolio piece, 1 a reject: missed focus, blur, a closed eye).";
const SCORES: &str = "- quality: 1 to 5 for the technical side (focus where it matters, exposure, noise).
- composition: 1 to 5 for framing and timing.";
const PLACE: &str = "- location_guess: city and country only when a landmark, a sign or unmistakable architecture shows where it is; \
otherwise empty strings and confidence 0.";
const CONFIDENCE: &str = "- confidence: 0 to 1, how sure you are of the whole answer.";

pub fn system_prompt(brief: &Brief) -> String {
    let f = brief.fields;
    let instructions = brief.instructions.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(INSTRUCTIONS);
    let lines: Vec<&str> = [
        (f.caption, CAPTION),
        (f.headline, HEADLINE),
        (f.subject, SUBJECT),
        (f.keywords, KEYWORDS),
        (f.keywords_new, KEYWORDS_NEW),
        (f.rating, RATING),
        (f.scores, SCORES),
        (f.place, PLACE),
        (true, CONFIDENCE),
    ]
    .into_iter()
    .filter_map(|(on, line)| on.then_some(line))
    .collect();
    let mut prompt = format!("{instructions}\n\nFields:\n{}\n", lines.join("\n"));
    let language = match (f.caption, brief.language) {
        (true, Language::En) => Some("Captions: English only (caption_en)."),
        (true, Language::Pl) => Some("Captions: Polish only (caption_pl); the headline in Polish."),
        (true, Language::Both) => {
            Some("Captions: both English (caption_en) and Polish (caption_pl); the headline in English.")
        }
        (false, Language::Pl) if f.headline => Some("The headline in Polish."),
        (false, _) if f.headline => Some("The headline in English."),
        (false, _) => None,
    };
    if let Some(language) = language {
        prompt.push_str(&format!("\n{language}\n"));
    }
    if f.keywords || f.keywords_new {
        let vocabulary = if brief.vocabulary.is_empty() {
            "(none yet)".to_string()
        } else {
            brief.vocabulary.iter().take(VOCABULARY_SIZE).cloned().collect::<Vec<_>>().join(", ")
        };
        prompt.push_str(&format!("\nThe photographer's keyword vocabulary:\n{vocabulary}\n"));
    }
    prompt
}

/// The answer's JSON Schema for these fields: every field required and no others, as
/// structured output wants it; "unknown" is an empty string, an empty list or 0.
pub fn schema(fields: &Fields) -> Value {
    let int = json!({"type": "integer"});
    let text = json!({"type": "string"});
    let words = json!({"type": "array", "items": {"type": "string"}});
    let place = json!({
        "type": "object",
        "properties": {
            "city": {"type": "string"},
            "country": {"type": "string"},
            "confidence": {"type": "number"}
        },
        "required": ["city", "country", "confidence"],
        "additionalProperties": false
    });
    let f = fields;
    let mut properties = Map::new();
    for (on, name, kind) in [
        (f.caption, "caption_en", &text),
        (f.caption, "caption_pl", &text),
        (f.headline, "headline", &text),
        (f.subject, "subject", &text),
        (f.keywords, "keywords", &words),
        (f.keywords_new, "keywords_new", &words),
        (f.rating, "rating", &int),
        (f.scores, "quality", &int),
        (f.scores, "composition", &int),
        (true, "confidence", &json!({"type": "number"})),
        (f.place, "location_guess", &place),
    ] {
        if on {
            properties.insert(name.to_string(), kind.clone());
        }
    }
    let required: Vec<&String> = properties.keys().collect();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

/// Identifies the prompt and schema a suggestion was made with. The vocabulary is left
/// out: it grows with every keyword accepted, and would make every photo look as if it
/// had never been described.
pub fn prompt_hash(brief: &Brief) -> u64 {
    let general = Brief { vocabulary: Vec::new(), ..brief.clone() };
    let text = format!("{}\n{}\n{}", crate::PROMPT_VERSION, system_prompt(&general), schema(&brief.fields));
    xxhash_rust::xxh3::xxh3_64(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_left_out_are_neither_asked_for_nor_in_the_schema() {
        let fields = Fields { caption: false, keywords: false, keywords_new: false, place: false, ..Fields::default() };
        let brief = Brief { fields, vocabulary: vec!["street".into()], ..Default::default() };
        let prompt = system_prompt(&brief);
        assert!(
            !prompt.contains("caption_en") && !prompt.contains("vocabulary:") && !prompt.contains("location_guess")
        );
        assert!(prompt.contains("- headline") && prompt.contains("- confidence"));
        let schema = schema(&fields);
        let names: Vec<&str> = schema["properties"].as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(names.len(), schema["required"].as_array().unwrap().len());
        assert!(names.contains(&"rating") && !names.contains(&"caption_en") && !names.contains(&"location_guess"));
    }

    #[test]
    fn own_instructions_replace_the_default_and_change_the_hash() {
        let default = Brief::default();
        let own = Brief { instructions: Some("Describe tersely.".into()), ..Default::default() };
        let blank = Brief { instructions: Some("  ".into()), ..Default::default() };
        assert!(system_prompt(&own).starts_with("Describe tersely.\n\nFields:"));
        assert_eq!(system_prompt(&blank), system_prompt(&default));
        assert_ne!(prompt_hash(&own), prompt_hash(&default));
    }

    #[test]
    fn a_grown_vocabulary_keeps_the_hash() {
        let before = Brief { vocabulary: vec!["street".into()], ..Default::default() };
        let after = Brief { vocabulary: vec!["street".into(), "tram".into()], ..Default::default() };
        assert_eq!(prompt_hash(&before), prompt_hash(&after));
        let fewer = Brief { fields: Fields { rating: false, ..Fields::default() }, ..before.clone() };
        assert_ne!(prompt_hash(&before), prompt_hash(&fewer));
    }
}
