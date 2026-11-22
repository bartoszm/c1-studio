//! Which frame of a group of alike photos to keep, asked of a model that sees them
//! together: the frames of a burst differ in what a measure cannot see (an expression, a
//! gesture, where a passer-by stands), and only side by side. The model orders the frames
//! and says in a few words what sets each apart; the photographer still chooses.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::AiError;

/// Bumped when the question changes: an answer to another version is asked for again.
pub const VERSION: u32 = 1;

/// The most frames one question holds: more, and each is too small to judge.
pub const MAX_FRAMES: usize = 16;

/// What is asked about: how many frames, of what kind of group, and how they are shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickAsk {
    pub frames: usize,
    /// A burst (frames moments apart), not the same moment saved twice.
    pub series: bool,
    /// The frames are cells of one contact sheet, numbered in their corners; else each is
    /// an image of its own, in order.
    pub sheet: bool,
}

/// What the model comparing frames is told.
pub fn system() -> String {
    "You help a photographer choose which frame to keep from a group of photos that look alike: a burst, or the same \
moment saved twice. You see every frame of the group, numbered from 1 in the order taken.

Compare the frames with each other, not against an ideal photo: what differs between them is all that matters. \
Look, most important first, at the moment (expressions, open eyes, a gesture or a moving subject at its peak, where \
passers-by stand), the composition (where the subject sits, limbs or heads cut by the frame, the horizon, clutter at \
the edges), the mood and the light, and last the technical side (focus on the subject, motion blur, exposure).

Order all the frames from the one to keep to the one to drop first. For each frame give one reason of at most twelve \
words that names what sets it apart from the others (\"eyes closed\", \"hand hides the face\", \"best gesture, \
slightly soft\"); when two frames cannot be told apart, say so rather than invent a difference. In note, say in one \
sentence how clear the choice is. The reasons are shown to the photographer beside each frame: plain words, no \
frame numbers in them."
        .to_string()
}

/// The user message: what the frames are and how they are shown.
pub fn message(ask: &PickAsk) -> String {
    let kind = if ask.series { "a burst, in the order taken" } else { "one moment saved more than once" };
    let shown = if ask.sheet {
        "They are the cells of the contact sheet, each numbered in its top left corner."
    } else {
        "Each image is one frame, in order: the first image is frame 1."
    };
    format!("{} frames of {kind}. {shown} Which should be kept?", ask.frames)
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "order": {"type": "array", "items": {"type": "integer"}},
            "frames": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"frame": {"type": "integer"}, "reason": {"type": "string"}},
                    "required": ["frame", "reason"],
                    "additionalProperties": false
                }
            },
            "note": {"type": "string"}
        },
        "required": ["order", "frames", "note"],
        "additionalProperties": false
    })
}

/// A model's order of a group's frames.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Verdict {
    /// The frames from 0, the one to keep first: every frame once.
    pub order: Vec<usize>,
    /// Why, per frame in the frames' own order; empty where the model gave none.
    pub reasons: Vec<String>,
    pub note: String,
}

#[derive(Deserialize)]
struct RawFrame {
    frame: i64,
    #[serde(default)]
    reason: String,
}

#[derive(Deserialize)]
struct RawVerdict {
    #[serde(default)]
    order: Vec<i64>,
    #[serde(default)]
    frames: Vec<RawFrame>,
    #[serde(default)]
    note: String,
}

/// Reads the answer about `frames` frames. Numbers that name no frame and repeats are
/// dropped; frames the order leaves out follow in their own order. An order that names
/// no frame at all is no answer.
pub fn read(answer: &Value, frames: usize) -> Result<Verdict, AiError> {
    let raw: RawVerdict = serde_json::from_value(answer.clone()).map_err(|e| AiError::BadReply(e.to_string()))?;
    let index = |n: i64| usize::try_from(n - 1).ok().filter(|&i| i < frames);
    let mut order: Vec<usize> = Vec::with_capacity(frames);
    for i in raw.order.into_iter().filter_map(index) {
        if !order.contains(&i) {
            order.push(i);
        }
    }
    if order.is_empty() {
        return Err(AiError::BadReply("the answer orders none of the frames".into()));
    }
    order.extend((0..frames).filter(|i| !order.contains(i)).collect::<Vec<_>>());
    let mut reasons = vec![String::new(); frames];
    for frame in raw.frames {
        if let Some(i) = index(frame.frame)
            && reasons[i].is_empty()
        {
            reasons[i] = frame.reason.trim().to_string();
        }
    }
    Ok(Verdict { order, reasons, note: raw.note.trim().to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_says_what_the_frames_are_and_how_they_are_shown() {
        let burst = message(&PickAsk { frames: 5, series: true, sheet: true });
        assert!(burst.starts_with("5 frames of a burst"));
        assert!(burst.contains("contact sheet"));
        let twice = message(&PickAsk { frames: 2, series: false, sheet: false });
        assert!(twice.contains("saved more than once") && twice.contains("first image is frame 1"));
    }

    #[test]
    fn an_answer_becomes_an_order_of_every_frame_with_its_reasons() {
        let answer = json!({
            "order": [3, 1, 2],
            "frames": [{"frame": 1, "reason": " eyes closed "}, {"frame": 3, "reason": "best gesture"}],
            "note": " Clear. "
        });
        let v = read(&answer, 3).unwrap();
        assert_eq!(v.order, [2, 0, 1]);
        assert_eq!(v.reasons, ["eyes closed", "", "best gesture"]);
        assert_eq!(v.note, "Clear.");
    }

    #[test]
    fn frames_that_do_not_exist_and_repeats_go_and_those_left_out_follow() {
        let answer = json!({"order": [2, 9, 2, 0, -1], "frames": [{"frame": 7, "reason": "x"}], "note": ""});
        let v = read(&answer, 4).unwrap();
        assert_eq!(v.order, [1, 0, 2, 3]);
        assert!(v.reasons.iter().all(String::is_empty));
    }

    #[test]
    fn an_answer_that_orders_nothing_is_no_answer() {
        assert!(matches!(read(&json!({"order": [], "frames": [], "note": ""}), 3), Err(AiError::BadReply(_))));
        assert!(matches!(read(&json!({"order": [8]}), 3), Err(AiError::BadReply(_))));
        assert!(matches!(read(&json!("no"), 3), Err(AiError::BadReply(_))));
    }

    #[test]
    fn the_schema_asks_for_the_order_and_a_reason_per_frame() {
        let schema = schema();
        assert_eq!(schema["required"], json!(["order", "frames", "note"]));
        assert_eq!(schema["properties"]["frames"]["items"]["required"], json!(["frame", "reason"]));
        assert!(system().contains("order taken"));
    }
}
