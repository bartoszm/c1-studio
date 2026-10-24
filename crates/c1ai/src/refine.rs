//! Lessons from the review: how the user corrected a model's suggestions (place names
//! changed, keywords added or taken off, captions rewritten, suggestions rejected), turned
//! by a model into a few short lessons that go into the prompt after the instructions.
//! Asked for on demand, as text only: no photo is sent.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::AiError;

/// What refining is asked: the instructions and lessons in use, and the corrections.
#[derive(Clone, Debug, Default)]
pub struct RefineAsk {
    /// The instructions a run gives now (the user's own, or [`crate::INSTRUCTIONS`]).
    pub instructions: String,
    pub lessons: Option<String>,
    /// One line per suggestion corrected or rejected.
    pub reviews: Vec<String>,
    /// Suggestions accepted as they were: they say the model got those right.
    pub unchanged: usize,
}

/// The lessons proposed, to replace the ones in use, and why.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Refined {
    pub lessons: String,
    pub rationale: String,
}

/// How many lessons the prompt is given at most.
pub const MAX_LESSONS: usize = 15;

pub const SYSTEM: &str = "You help a photographer tune the instructions given to a vision model that suggests metadata \
for their photos: captions, a headline, keywords chosen from their own vocabulary, a rating, and the place (country, \
state or region, city, location). The photographer reviews every suggestion and corrects it before accepting it.

You get the instructions in use, the lessons already added to them, and a list of the photographer's corrections. \
Write the lessons the model should follow from now on: short imperative lines, one per line, each starting with \"- \". \
Generalise from the corrections when they share a pattern (place names always written in Polish, a keyword always added \
for a kind of scene, ratings always lowered), and give concrete examples from them (\"Małopolskie, not Lesser Poland\"). \
Keep the lessons in use that the corrections do not contradict; drop or change those they do. Do not repeat what the \
instructions already say, do not invent lessons the corrections do not support, and leave out one-off corrections that \
only fix a single photo. Write at most 15 lessons, in English, whatever the language of the examples. \
In rationale, say in two or three sentences what changed and why.";

/// The user message: what is in use, then the corrections.
pub fn message(ask: &RefineAsk) -> String {
    let lessons = ask.lessons.as_deref().map(str::trim).filter(|l| !l.is_empty()).unwrap_or("(none yet)");
    let reviews = if ask.reviews.is_empty() { "(none)".to_string() } else { ask.reviews.join("\n") };
    format!(
        "Instructions in use:\n{}\n\nLessons in use:\n{lessons}\n\nCorrections ({} suggestions; {} more accepted as they were):\n{reviews}\n",
        ask.instructions.trim(),
        ask.reviews.len(),
        ask.unchanged,
    )
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {"lessons": {"type": "string"}, "rationale": {"type": "string"}},
        "required": ["lessons", "rationale"],
        "additionalProperties": false
    })
}

/// Reads the answer, keeping at most [`MAX_LESSONS`] lines that are lessons.
pub fn read(answer: &Value) -> Result<Refined, AiError> {
    let refined: Refined = serde_json::from_value(answer.clone()).map_err(|e| AiError::BadReply(e.to_string()))?;
    let lessons: Vec<&str> =
        refined.lessons.lines().map(str::trim).filter(|l| !l.is_empty()).take(MAX_LESSONS).collect();
    Ok(Refined { lessons: lessons.join("\n"), rationale: refined.rationale.trim().to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_holds_what_is_in_use_and_the_corrections() {
        let ask = RefineAsk {
            instructions: "Describe the photo.".into(),
            lessons: None,
            reviews: vec!["- state: Lesser Poland → Małopolskie".into()],
            unchanged: 4,
        };
        let text = message(&ask);
        assert!(text.contains("Describe the photo."));
        assert!(text.contains("Lessons in use:\n(none yet)"));
        assert!(text.contains(
            "Corrections (1 suggestions; 4 more accepted as they were):\n- state: Lesser Poland → Małopolskie"
        ));
    }

    #[test]
    fn the_answer_is_cut_to_its_lessons() {
        let many: Vec<String> = (0..20).map(|i| format!("- lesson {i}")).collect();
        let answer = json!({"lessons": format!("{}\n\n", many.join("\n\n")), "rationale": " Because. "});
        let refined = read(&answer).unwrap();
        assert_eq!(refined.lessons.lines().count(), MAX_LESSONS);
        assert_eq!(refined.rationale, "Because.");
        assert!(read(&json!({"nope": 1})).is_ok_and(|r| r.lessons.is_empty()));
        assert!(read(&json!("text")).is_err());
    }
}
