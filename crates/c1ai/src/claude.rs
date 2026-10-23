//! The Claude API (Messages, over plain HTTPS: there is no official Rust SDK). One call
//! per photo: the instructions and vocabulary as a cached system prompt, the photo as a
//! base64 JPEG, the answer constrained to [`crate::schema`] by structured output.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AiError, Brief, Failed, PhotoRequest, RawAnswer, Reply, RunRecord, VisionProvider, Vocabulary, validate};

const URL: &str = "https://api.anthropic.com/v1/messages";
const VERSION: &str = "2023-06-01";
/// Lets the API answer a declined photo with another model instead of stopping
/// (`fallbacks: "default"`); Claude API only, and not for Haiku.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// A model to choose from, with its list price per million tokens (US dollars).
#[derive(Clone, Copy, Debug)]
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub input: f64,
    pub output: f64,
    pub fallbacks: bool,
}

pub const MODELS: [Model; 3] = [
    Model { id: "claude-opus-5-5", name: "Claude Opus 5.5", input: 4.0, output: 20.0, fallbacks: true },
    Model { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", input: 2.0, output: 10.0, fallbacks: true },
    Model { id: "claude-haiku-5-5", name: "Claude Haiku 5.5", input: 0.10, output: 0.50, fallbacks: false },
];

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

pub fn model(id: &str) -> Option<Model> {
    MODELS.iter().copied().find(|m| m.id == id)
}

/// The long edge of the JPEG sent: past it the API scales the image down anyway.
pub const LONG_EDGE: u32 = 1568;

/// Tokens a photo takes, roughly, for estimating a run before it starts: the image (most
/// go as their thumbnail, about 450 by 300; a preview at [`LONG_EDGE`] would take about
/// 2200), the line of context, the cached instructions read back, and the answer with a
/// little thinking.
const PHOTO_INPUT: f64 = 400.0;
const CACHED_INPUT: f64 = 1500.0;
const PHOTO_OUTPUT: f64 = 700.0;

/// What describing `photos` photos would cost with `model`, in US dollars.
pub fn estimate(model: &Model, photos: usize) -> f64 {
    let per_photo = (PHOTO_INPUT + CACHED_INPUT * 0.1) * model.input + PHOTO_OUTPUT * model.output;
    per_photo * photos as f64 / 1e6
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Usage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_creation_input_tokens: u64,
    pub(crate) cache_read_input_tokens: u64,
}

/// What a call cost: cache writes at 1.25 times the input price, reads at a tenth.
fn cost(model: &Model, usage: &Usage) -> f64 {
    let input = usage.input_tokens as f64
        + usage.cache_creation_input_tokens as f64 * 1.25
        + usage.cache_read_input_tokens as f64 * 0.1;
    (input * model.input + usage.output_tokens as f64 * model.output) / 1e6
}

/// How much the model may think before answering.
pub const EFFORTS: [&str; 3] = ["low", "medium", "high"];

#[derive(Clone, Debug)]
pub struct Options {
    pub model: String,
    /// One of [`EFFORTS`].
    pub effort: String,
    /// Attempts per photo on rate limits, overload and network errors.
    pub attempts: u32,
    /// Set to stop: a call under way ends as soon as it can, with [`AiError::Stopped`].
    pub stop: Arc<AtomicBool>,
}

impl Default for Options {
    fn default() -> Self {
        Options { model: DEFAULT_MODEL.to_string(), effort: "low".to_string(), attempts: 4, stop: Arc::default() }
    }
}

impl Options {
    pub(crate) fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

pub struct Claude {
    key: String,
    model: Model,
    options: Options,
    system: String,
    schema: Value,
    prompt: u64,
    vocabulary: Vocabulary,
    agent: ureq::Agent,
}

impl Claude {
    pub fn new(key: String, options: Options, brief: &Brief, vocabulary: Vocabulary) -> Result<Self, AiError> {
        let model =
            model(&options.model).ok_or_else(|| AiError::BadReply(format!("unknown model {}", options.model)))?;
        let system = crate::system_prompt(brief);
        let schema = crate::schema(&brief.fields);
        let prompt = crate::prompt_hash(brief);
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(180)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Claude { key, model, options, system, schema, prompt, vocabulary, agent })
    }

    fn body(&self, photo: &PhotoRequest) -> Value {
        let data = base64::engine::general_purpose::STANDARD.encode(&photo.jpeg);
        let mut body = json!({
            "model": self.model.id,
            "max_tokens": 16000,
            "system": [{"type": "text", "text": self.system, "cache_control": {"type": "ephemeral"}}],
            "output_config": {
                "effort": self.options.effort,
                "format": {"type": "json_schema", "schema": self.schema}
            },
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": data}},
                    {"type": "text", "text": photo.context}
                ]
            }]
        });
        if self.model.fallbacks {
            body["fallbacks"] = json!("default");
        }
        body
    }

    fn call(&self, body: &Value) -> Result<(Value, u64), AiError> {
        let started = Instant::now();
        let mut request = self
            .agent
            .post(URL)
            .header("x-api-key", &self.key)
            .header("anthropic-version", VERSION)
            .header("content-type", "application/json");
        if self.model.fallbacks {
            request = request.header("anthropic-beta", FALLBACK_BETA);
        }
        let mut response = request.send_json(body).map_err(|e| AiError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let text = response.body_mut().read_to_string().map_err(|e| AiError::Network(e.to_string()))?;
        let millis = started.elapsed().as_millis() as u64;
        if status != 200 {
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                .unwrap_or(text);
            return Err(if status == 401 || status == 403 {
                AiError::BadKey(message)
            } else {
                AiError::Http { status, message, retry_after }
            });
        }
        let value = serde_json::from_str(&text).map_err(|e| AiError::BadReply(e.to_string()))?;
        Ok((value, millis))
    }
}

/// Reads a Messages response: the stop reason first, then the text block holding the
/// structured answer.
fn read(message: &Value) -> Result<(String, RawAnswer), AiError> {
    match message["stop_reason"].as_str() {
        Some("refusal") => {
            return Err(AiError::Refused(message["stop_details"]["category"].as_str().map(str::to_string)));
        }
        Some("max_tokens") => return Err(AiError::Truncated),
        _ => {}
    }
    let raw = message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<String>();
    let answer = serde_json::from_str::<RawAnswer>(&raw).map_err(|e| AiError::BadReply(e.to_string()))?;
    Ok((raw, answer))
}

impl VisionProvider for Claude {
    fn id(&self) -> &str {
        "claude"
    }

    fn model(&self) -> &str {
        self.model.id
    }

    fn prompt_hash(&self) -> u64 {
        self.prompt
    }

    fn describe(&self, photo: &PhotoRequest) -> Result<Reply, Failed> {
        let body = self.body(photo);
        let mut attempt = 0;
        let (message, millis) = loop {
            attempt += 1;
            if self.options.stopped() {
                return Err(AiError::Stopped.into());
            }
            match self.call(&body) {
                Ok(done) => break done,
                Err(e) if e.retryable() && attempt < self.options.attempts => {
                    let wait = match &e {
                        AiError::Http { retry_after: Some(s), .. } => *s,
                        _ => 2u64.pow(attempt),
                    };
                    // In short steps, so a stop is not kept waiting.
                    let until = Instant::now() + Duration::from_secs(wait.min(60));
                    while Instant::now() < until && !self.options.stopped() {
                        std::thread::sleep(Duration::from_millis(200));
                    }
                }
                Err(e) => return Err(e.into()),
            }
        };
        let usage: Usage = serde_json::from_value(message["usage"].clone()).unwrap_or_default();
        // A declined photo answered by the fallback model is billed at that model's price.
        let served = message["model"].as_str().and_then(model).unwrap_or(self.model);
        let record = |raw: String| RunRecord {
            provider: "claude".to_string(),
            model: served.id.to_string(),
            prompt: self.prompt,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_write_tokens: usage.cache_creation_input_tokens,
            cache_read_tokens: usage.cache_read_input_tokens,
            cost: cost(&served, &usage),
            millis,
            raw,
        };
        // A refusal or a cut answer was paid for: it is recorded with the error.
        let (raw, answer) =
            read(&message).map_err(|error| Failed { error, record: Some(Box::new(record(message.to_string()))) })?;
        let (suggestion, warnings) = validate(&answer, &self.vocabulary);
        Ok(Reply { suggestion, warnings, record: record(raw) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A response in the API's shape: a thinking block, then the structured answer.
    const RESPONSE: &str = r#"{
      "id": "msg_01", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
      "content": [
        {"type": "thinking", "thinking": "", "signature": "x"},
        {"type": "text", "text": "{\"caption_en\":\"A tram turns into Basztowa street at dusk.\",\"caption_pl\":\"\",\"headline\":\"Tram at dusk\",\"subject\":\"tram\",\"keywords\":[\"street\",\"Kraków\"],\"keywords_new\":[\"tram\"],\"rating\":4,\"quality\":4,\"composition\":3,\"confidence\":0.8,\"location_guess\":{\"country\":{\"name\":\"Poland\",\"p\":0.9},\"state\":{\"name\":\"\",\"p\":0},\"city\":{\"name\":\"Kraków\",\"p\":0.6},\"location\":{\"name\":\"\",\"p\":0},\"coordinates\":{\"lat\":50.06,\"lon\":19.94,\"p\":0.4,\"radius_km\":5}}}"}
      ],
      "stop_reason": "end_turn", "stop_details": null,
      "usage": {"input_tokens": 2210, "output_tokens": 640, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 1480}
    }"#;

    #[test]
    fn reads_the_structured_answer() {
        let message: Value = serde_json::from_str(RESPONSE).unwrap();
        let (_, answer) = read(&message).unwrap();
        assert_eq!(answer.headline, "Tram at dusk");
        assert_eq!(answer.keywords, vec!["street", "Kraków"]);
        assert_eq!(answer.location_guess.city.name, "Kraków");
        let usage: Usage = serde_json::from_value(message["usage"].clone()).unwrap();
        let opus = model("claude-opus-5-5").unwrap();
        let expected = ((2210.0 + 1480.0 * 0.1) * 4.0 + 640.0 * 20.0) / 1e6;
        assert!((cost(&opus, &usage) - expected).abs() < 1e-12);
    }

    #[test]
    fn a_refusal_or_a_cut_answer_is_an_error() {
        let refused = json!({"stop_reason": "refusal", "stop_details": {"category": "cyber"}, "content": []});
        assert!(matches!(read(&refused), Err(AiError::Refused(Some(c))) if c == "cyber"));
        let cut = json!({"stop_reason": "max_tokens", "content": [{"type": "text", "text": "{\"cap"}]});
        assert!(matches!(read(&cut), Err(AiError::Truncated)));
    }

    #[test]
    fn the_request_caches_the_instructions_and_asks_for_the_schema() {
        let brief = Brief { vocabulary: vec!["street".into()], ..Default::default() };
        let claude = Claude::new("k".into(), Options::default(), &brief, Vocabulary::default()).unwrap();
        let body = claude.body(&PhotoRequest { jpeg: vec![1, 2, 3], context: "DSCF0001.RAF".into() });
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(body["messages"][0]["content"][0]["source"]["data"], "AQID");
        assert_eq!(body["fallbacks"], "default");
        let haiku = Options { model: "claude-haiku-5-5".into(), ..Options::default() };
        let claude = Claude::new("k".into(), haiku, &brief, Vocabulary::default()).unwrap();
        assert!(claude.body(&PhotoRequest { jpeg: vec![], context: String::new() }).get("fallbacks").is_none());
    }
}
