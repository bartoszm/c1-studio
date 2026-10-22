//! Gemini through the Antigravity CLI (`agy --print`), signed in with a Google account:
//! the same prompt and fields as [`crate::claude`], counted against the account's quota
//! rather than billed per token. The photo goes to agy as a file it reads; the answer
//! comes back in the JSON envelope's `response`, as a fenced JSON block (agy's own
//! `--json-schema` was seen to answer `{}`, so the schema is part of the prompt).
//!
//! It can also describe a contact sheet ([`c1image::contact_sheet`]): up to 36 numbered
//! photos in one call, each answer carrying its number.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use crate::claude::Options;
use crate::{
    AiError, Brief, Failed, PhotoRequest, RawAnswer, Reply, RunRecord, SheetReply, SheetRequest, VisionProvider,
    Vocabulary, validate,
};

/// How long one call may take: a sheet of 36 photos takes a minute or two.
const CALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub const DEFAULT_MODEL: &str = "gemini-3.8-flash-low";

/// Where agy is: on `PATH`, else where its installer puts it.
pub fn find() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "agy.exe" } else { "agy" };
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(exe));
    let local = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("agy").join("bin").join(exe));
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(|home| PathBuf::from(home).join(".local").join("bin").join(exe));
    on_path.chain(local).chain(home).find(|p| p.is_file())
}

fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.current_dir(std::env::temp_dir());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// The Gemini models agy offers (`agy models`), id and name.
pub fn models(program: &Path) -> Result<Vec<(String, String)>, AiError> {
    let out = command(program).arg("models").output().map_err(|e| AiError::Cli(format!("agy: {e}")))?;
    Ok(parse_models(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_models(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(id, name)| (id.trim().to_string(), name.trim().to_string()))
        .filter(|(id, _)| id.starts_with("gemini"))
        .collect()
}

/// Identifies the prompt and schema agy is sent: Claude's, asked for in the prompt, one
/// photo at a time or as a sheet. A photo described one way is not taken as described
/// the other.
pub fn prompt_hash(brief: &Brief, sheets: bool) -> u64 {
    let text = format!("agy {sheets} {}", crate::prompt_hash(brief));
    xxhash_rust::xxh3::xxh3_64(text.as_bytes())
}

/// The reply agy prints with `--output-format json`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Envelope {
    status: String,
    response: String,
    error: Option<Value>,
    usage: Usage,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
}

pub struct Agy {
    program: PathBuf,
    options: Options,
    system: String,
    schema: String,
    prompt: u64,
    sheets: bool,
    vocabulary: Vocabulary,
}

impl Agy {
    /// `sheets`: photos go as contact sheets ([`VisionProvider::describe_sheet`]).
    pub fn new(program: PathBuf, options: Options, brief: &Brief, vocabulary: Vocabulary, sheets: bool) -> Self {
        Agy {
            program,
            options,
            system: crate::system_prompt(brief),
            schema: crate::schema(&brief.fields).to_string(),
            prompt: prompt_hash(brief, sheets),
            sheets,
            vocabulary,
        }
    }

    fn args(&self, prompt: &str, dir: &Path) -> Vec<String> {
        let mut args: Vec<String> =
            vec!["--print".into(), prompt.into(), "--add-dir".into(), dir.display().to_string()];
        args.extend(["--model".into(), self.options.model.clone(), "--output-format".into(), "json".into()]);
        args.extend([
            "--print-timeout".into(),
            format!("{}s", CALL_TIMEOUT.as_secs()),
            "--disable-slash-commands".into(),
        ]);
        // A model id with its effort in it (`-low`) takes no other.
        if !["-low", "-medium", "-high"].iter().any(|e| self.options.model.ends_with(e)) {
            args.extend(["--effort".into(), self.options.effort.clone()]);
        }
        args
    }

    /// Runs agy on `prompt` with `jpeg` as `image.jpg` in a folder of its own.
    fn call(&self, prompt: impl FnOnce(&Path) -> String, jpeg: &[u8]) -> Result<(Envelope, String, u64), AiError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "c1studio-agy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).map_err(|e| AiError::Cli(format!("agy: a folder for the photo: {e}")))?;
        let _gone = RemoveOnDrop(dir.clone());
        let image = dir.join("image.jpg");
        std::fs::write(&image, jpeg).map_err(|e| AiError::Cli(format!("agy: the photo's file: {e}")))?;
        let started = Instant::now();
        let mut child = command(&self.program)
            .args(self.args(&prompt(&image), &dir))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AiError::Cli(format!("agy would not start: {e}")))?;
        let drain = |mut pipe: Box<dyn Read + Send>| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = pipe.read_to_end(&mut bytes);
                bytes
            })
        };
        let stdout = drain(Box::new(child.stdout.take().expect("piped")));
        let stderr = drain(Box::new(child.stderr.take().expect("piped")));
        let ended = loop {
            match child.try_wait() {
                Ok(Some(_)) => break Ok(()),
                Ok(None) => {}
                Err(e) => break Err(AiError::BadReply(format!("agy: {e}"))),
            }
            if self.options.stopped() {
                break Err(AiError::Stopped);
            }
            if started.elapsed() > CALL_TIMEOUT + Duration::from_secs(60) {
                break Err(AiError::BadReply(format!("no answer from agy in {} minutes", CALL_TIMEOUT.as_secs() / 60)));
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if let Err(e) = ended {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        let millis = started.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&stdout.join().unwrap_or_default()).into_owned();
        let envelope = serde_json::from_str::<Envelope>(stdout.trim()).map_err(|_| {
            let stderr = stderr.join().unwrap_or_default();
            let said = String::from_utf8_lossy(&stderr);
            let last = said.lines().chain(stdout.lines()).rfind(|l| !l.trim().is_empty()).unwrap_or("").trim();
            AiError::BadReply(format!("agy gave no answer: {last}"))
        })?;
        Ok((envelope, stdout, millis))
    }

    fn record(&self, usage: &Usage, millis: u64, raw: String) -> RunRecord {
        RunRecord {
            provider: "gemini".into(),
            model: self.options.model.clone(),
            prompt: self.prompt,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_write_tokens: 0,
            cache_read_tokens: usage.cache_read_tokens,
            cost: 0.0,
            millis,
            raw,
        }
    }

    /// Calls, trying again after a pause on what may pass (the network, a busy server).
    fn call_again(&self, prompt: &dyn Fn(&Path) -> String, jpeg: &[u8]) -> Result<(Envelope, String, u64), AiError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self
                .call(prompt, jpeg)
                .and_then(|(envelope, stdout, millis)| failure(&envelope).map_or(Ok((envelope, stdout, millis)), Err))
            {
                Ok(done) => return Ok(done),
                Err(e) if e.retryable() && attempt < self.options.attempts.min(2) && !self.options.stopped() => {
                    std::thread::sleep(Duration::from_secs(2u64.pow(attempt)));
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn shape(&self) -> String {
        format!(
            "Reply with a single fenced json block and nothing else. The object follows this JSON Schema:\n{}",
            self.schema
        )
    }
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a failed envelope means: no sign-in and an exhausted quota stop a run.
fn failure(envelope: &Envelope) -> Option<AiError> {
    if envelope.status.eq_ignore_ascii_case("SUCCESS") {
        return None;
    }
    let message = match &envelope.error {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => format!("agy answered {}", envelope.status),
    };
    let lower = message.to_lowercase();
    Some(if lower.contains("quota") || lower.contains("rate limit") || lower.contains("resource_exhausted") {
        AiError::Limit(format!("Gemini: {message}"))
    } else if lower.contains("sign in") || lower.contains("login") || lower.contains("unauthenticated") {
        AiError::Cli(format!("agy is not signed in: run agy once to sign in ({message})"))
    } else if lower.contains("unavailable") || lower.contains("overloaded") || lower.contains("timeout") {
        AiError::Network(message)
    } else {
        AiError::BadReply(message)
    })
}

/// The JSON object in a reply: the whole reply, the last fenced `json` block, or the
/// first balanced `{…}` in it, whichever reads.
pub(crate) fn extract_json(text: &str) -> Option<Value> {
    let text = text.trim();
    if let Ok(value @ Value::Object(_)) = serde_json::from_str(text) {
        return Some(value);
    }
    let fenced = text.rsplit("```").enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, block)| block).find_map(|block| {
        let body = block.trim_start().strip_prefix("json").unwrap_or(block);
        serde_json::from_str::<Value>(body.trim()).ok().filter(Value::is_object)
    });
    fenced.or_else(|| {
        let start = text.find('{')?;
        let mut depth = 0i32;
        let mut quoted = false;
        let mut escaped = false;
        for (i, c) in text[start..].char_indices() {
            match c {
                _ if escaped => escaped = false,
                '\\' if quoted => escaped = true,
                '"' => quoted = !quoted,
                '{' if !quoted => depth += 1,
                '}' if !quoted => {
                    depth -= 1;
                    if depth == 0 {
                        return serde_json::from_str(&text[start..=start + i]).ok();
                    }
                }
                _ => {}
            }
        }
        None
    })
}

/// The answer for each numbered photo of a sheet: `photos[i]` for photo `i + 1`, `None`
/// where the reply has none (or the label is not one of them).
fn sheet_answers(value: &Value, count: usize) -> (Vec<Option<RawAnswer>>, Vec<String>) {
    let mut answers: Vec<Option<RawAnswer>> = vec![None; count];
    let mut warnings = Vec::new();
    for item in value["photos"].as_array().into_iter().flatten() {
        let label = match &item["label"] {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().trim_start_matches('#').parse().ok(),
            _ => None,
        };
        let Some(i) = label.and_then(|n| (n as usize).checked_sub(1)).filter(|&i| i < count) else {
            warnings.push(format!("an answer for a photo not on the sheet: {}", item["label"]));
            continue;
        };
        match serde_json::from_value::<RawAnswer>(item.clone()) {
            Ok(answer) => answers[i] = Some(answer),
            Err(e) => warnings.push(format!("photo {}: {e}", i + 1)),
        }
    }
    (answers, warnings)
}

impl VisionProvider for Agy {
    fn id(&self) -> &str {
        "gemini"
    }

    fn model(&self) -> &str {
        &self.options.model
    }

    fn prompt_hash(&self) -> u64 {
        self.prompt
    }

    fn sheets(&self) -> bool {
        self.sheets
    }

    fn describe(&self, photo: &PhotoRequest) -> Result<Reply, Failed> {
        let prompt = |image: &Path| {
            format!("{}\n\nRead the image at {}.\n{}\n\n{}", self.system, image.display(), photo.context, self.shape())
        };
        let (envelope, stdout, millis) = self.call_again(&prompt, &photo.jpeg)?;
        let record = self.record(&envelope.usage, millis, envelope.response.clone());
        let answer = extract_json(&envelope.response)
            .and_then(|v| serde_json::from_value::<RawAnswer>(v).ok())
            .ok_or_else(|| Failed {
                error: AiError::BadReply("no JSON answer".into()),
                record: Some(Box::new(self.record(&envelope.usage, millis, stdout))),
            })?;
        let (suggestion, warnings) = validate(&answer, &self.vocabulary);
        Ok(Reply { suggestion, warnings, record })
    }

    fn describe_sheet(&self, sheet: &SheetRequest) -> Result<SheetReply, Failed> {
        let count = sheet.contexts.len();
        let lines: Vec<String> = sheet.contexts.iter().enumerate().map(|(i, c)| format!("{}: {c}", i + 1)).collect();
        let prompt = |image: &Path| {
            format!(
                "{}\n\nRead the contact sheet at {}: {count} photos, each numbered in a black box at its top left \
                 corner, row by row from the top left. Describe each photo on its own.\nWhat the catalog knows of each:\n{}\n\n\
                 Reply with a single fenced json block and nothing else: {{\"photos\": [...]}}, one object per photo, \
                 each with \"label\" (its number) and the fields of this JSON Schema:\n{}",
                self.system,
                image.display(),
                lines.join("\n"),
                self.schema
            )
        };
        let (envelope, stdout, millis) = self.call_again(&prompt, &sheet.jpeg)?;
        let record = self.record(&envelope.usage, millis, envelope.response.clone());
        let Some(value) = extract_json(&envelope.response) else {
            return Err(Failed {
                error: AiError::BadReply("no JSON answer".into()),
                record: Some(Box::new(self.record(&envelope.usage, millis, stdout))),
            });
        };
        let (answers, warnings) = sheet_answers(&value, count);
        let replies = answers.into_iter().map(|a| a.map(|a| validate(&a, &self.vocabulary))).collect();
        Ok(SheetReply { replies, warnings, record })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_found_however_it_is_wrapped() {
        assert_eq!(extract_json(r#"{"a": 1}"#).unwrap()["a"], 1);
        let fenced = "Here it is:\n```json\n{\"a\": 2, \"b\": \"}\"}\n```\nDone.";
        assert_eq!(extract_json(fenced).unwrap()["a"], 2);
        let bare = r#"Sure. {"a": {"b": "x\"}"}, "c": 3} trailing"#;
        assert_eq!(extract_json(bare).unwrap()["c"], 3);
        assert!(extract_json("no json here").is_none());
    }

    #[test]
    fn the_envelope_says_what_failed() {
        let envelope = |status: &str, error: &str| Envelope {
            status: status.into(),
            error: Some(Value::String(error.into())),
            ..Envelope::default()
        };
        assert!(failure(&envelope("SUCCESS", "")).is_none());
        assert!(matches!(failure(&envelope("ERROR", "RESOURCE_EXHAUSTED: quota")), Some(AiError::Limit(_))));
        let signed_out = failure(&envelope("ERROR", "please sign in")).unwrap();
        assert!(matches!(signed_out, AiError::Cli(_)) && signed_out.stops_run());
        assert!(failure(&envelope("ERROR", "503 UNAVAILABLE")).unwrap().retryable());
        let reply: Envelope = serde_json::from_str(
            r#"{"conversation_id":"x","status":"SUCCESS","response":"```json\n{\"headline\":\"Tram\"}\n```\n","duration_seconds":7.6,"num_turns":1,"usage":{"input_tokens":15029,"output_tokens":111,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":15140}}"#,
        )
        .unwrap();
        assert_eq!(reply.usage.input_tokens, 15029);
        assert_eq!(extract_json(&reply.response).unwrap()["headline"], "Tram");
    }

    #[test]
    fn a_sheet_s_answers_go_back_to_their_photos_by_label() {
        let value = serde_json::json!({"photos": [
            {"label": 2, "headline": "Second"},
            {"label": "#1", "headline": "First"},
            {"label": 9, "headline": "Not there"},
            {"headline": "No label"}
        ]});
        let (answers, warnings) = sheet_answers(&value, 3);
        assert_eq!(answers[0].as_ref().unwrap().headline, "First");
        assert_eq!(answers[1].as_ref().unwrap().headline, "Second");
        assert!(answers[2].is_none(), "left out");
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn only_gemini_models_are_offered() {
        let listed = "Fetching available models...\ngemini-4-argon-low\tGemini 4 Argon (Low)\nclaude-opus-5-5-low\tClaude Opus 5.5 (Low)\n";
        assert_eq!(parse_models(listed), vec![("gemini-4-argon-low".to_string(), "Gemini 4 Argon (Low)".to_string())]);
    }
}
