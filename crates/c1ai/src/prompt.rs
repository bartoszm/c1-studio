//! The instructions and the answer's shape, the same for every photo of a run (so a
//! provider can cache them), ported from the legacy `vision.py` prompts.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Pl,
    Both,
}

/// What a run asks for, besides the photo: the caption languages and the catalog's
/// keyword names the model should choose from (most used first).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Brief {
    pub language: Language,
    pub vocabulary: Vec<String>,
}

/// How many of the catalog's keyword names go into the prompt.
pub const VOCABULARY_SIZE: usize = 400;

const INSTRUCTIONS: &str = "You describe photographs from a photographer's Capture One catalog, one photo at a time. \
Your answer becomes suggested metadata that the photographer reviews before anything is saved, so be accurate rather than complete: \
leave a text field empty, or a list empty, when you cannot judge it from the photo.

Fields:
- caption_en / caption_pl: two or three sentences on what is happening, the light and the mood, as a curator would write for a portfolio. \
Fill only the language(s) asked for below; leave the other empty.
- headline: at most five words, in the first language asked for.
- subject: the main subject in two to four words.
- keywords: up to ten keywords chosen from the photographer's vocabulary below, copied exactly. Prefer specific ones over general ones.
- keywords_new: at most three keywords that the vocabulary lacks and the photo clearly calls for. Single words or short phrases, no commas.
- rating: 1 to 5, how strong the photo is within a photographer's own work (3 is a sound keeper, 5 a portfolio piece, 1 a reject: missed focus, blur, a closed eye).
- quality: 1 to 5 for the technical side (focus where it matters, exposure, noise).
- composition: 1 to 5 for framing and timing.
- confidence: 0 to 1, how sure you are of the whole answer.
- location_guess: city and country only when a landmark, a sign or unmistakable architecture shows where it is; otherwise empty strings and confidence 0.

The line of catalog data with each photo (file name, date, camera, place) is context, not something to repeat.";

pub fn system_prompt(brief: &Brief) -> String {
    let language = match brief.language {
        Language::En => "Captions: English only (caption_en).",
        Language::Pl => "Captions: Polish only (caption_pl); the headline in Polish.",
        Language::Both => "Captions: both English (caption_en) and Polish (caption_pl); the headline in English.",
    };
    let vocabulary = if brief.vocabulary.is_empty() {
        "(none yet)".to_string()
    } else {
        brief.vocabulary.iter().take(VOCABULARY_SIZE).cloned().collect::<Vec<_>>().join(", ")
    };
    format!("{INSTRUCTIONS}\n\n{language}\n\nThe photographer's keyword vocabulary:\n{vocabulary}\n")
}

/// The answer's JSON Schema: every field required and no others, as structured output
/// wants it; "unknown" is an empty string, an empty list or 0.
pub fn schema() -> Value {
    let int = json!({"type": "integer"});
    let words = json!({"type": "array", "items": {"type": "string"}});
    json!({
        "type": "object",
        "properties": {
            "caption_en": {"type": "string"},
            "caption_pl": {"type": "string"},
            "headline": {"type": "string"},
            "subject": {"type": "string"},
            "keywords": words,
            "keywords_new": words,
            "rating": int,
            "quality": int,
            "composition": int,
            "confidence": {"type": "number"},
            "location_guess": {
                "type": "object",
                "properties": {
                    "city": {"type": "string"},
                    "country": {"type": "string"},
                    "confidence": {"type": "number"}
                },
                "required": ["city", "country", "confidence"],
                "additionalProperties": false
            }
        },
        "required": ["caption_en", "caption_pl", "headline", "subject", "keywords", "keywords_new",
                     "rating", "quality", "composition", "confidence", "location_guess"],
        "additionalProperties": false
    })
}

/// Identifies the prompt and schema a suggestion was made with.
pub fn prompt_hash(system: &str) -> u64 {
    let text = format!("{}\n{system}\n{}", crate::PROMPT_VERSION, schema());
    xxhash_rust::xxh3::xxh3_64(text.as_bytes())
}
