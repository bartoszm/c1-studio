//! Claude through Claude Code (`claude -p`), signed in with a Claude plan instead of an
//! API key: the same prompt, schema and photo as [`crate::claude`], counted against the
//! plan's usage limits rather than billed per token. One process per photo; the photo
//! goes in on stdin as a stream-json user message, the answer comes back as the
//! `structured_output` of the final `result` event.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::claude::{Model, Options, Usage, model};
use crate::{AiError, Brief, Failed, PhotoRequest, RawAnswer, Reply, RunRecord, VisionProvider, Vocabulary, validate};

/// How long one photo may take before Claude Code is stopped: answers take seconds, a
/// minute at high effort; past this it is stuck (a prompt it waits on, a network gone).
const CALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Where Claude Code is: on `PATH`, else where its installer puts it.
pub fn find() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(exe));
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(|home| PathBuf::from(home).join(".local").join("bin").join(exe));
    on_path.chain(home).find(|p| p.is_file())
}

/// A command for Claude Code that uses the plan it is signed in with: an
/// `ANTHROPIC_API_KEY` in the app's environment would otherwise take over.
fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.env_remove("ANTHROPIC_API_KEY").env_remove("ANTHROPIC_AUTH_TOKEN").current_dir(std::env::temp_dir());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Whether Claude Code is signed in, and with what (`claude auth status`).
#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Status {
    pub logged_in: bool,
    /// `claude.ai` for a plan, else an API key or a cloud provider.
    pub auth_method: Option<String>,
    /// `pro`, `max`, `team`…
    pub subscription_type: Option<String>,
}

pub fn status(program: &Path) -> Result<Status, AiError> {
    let out =
        command(program).args(["auth", "status"]).output().map_err(|e| AiError::Cli(format!("Claude Code: {e}")))?;
    // Signed out, it says so with a non-zero exit and the same JSON.
    serde_json::from_slice(&out.stdout)
        .map_err(|_| AiError::Cli(format!("Claude Code: {}", String::from_utf8_lossy(&out.stderr).trim())))
}

pub struct ClaudeCode {
    program: PathBuf,
    model: Model,
    options: Options,
    /// The system prompt, in a file: with the vocabulary it can outgrow a command line.
    /// This provider's own, removed when it is dropped.
    system: PathBuf,
    schema: Value,
    prompt: u64,
    vocabulary: Vocabulary,
}

impl ClaudeCode {
    pub fn new(program: PathBuf, options: Options, brief: &Brief, vocabulary: Vocabulary) -> Result<Self, AiError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let model =
            model(&options.model).ok_or_else(|| AiError::BadReply(format!("unknown model {}", options.model)))?;
        let text = crate::system_prompt(brief);
        let schema = crate::schema(&brief.fields);
        let prompt = crate::prompt_hash(brief);
        let name = format!("c1studio-ai-{}-{}.txt", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let system = std::env::temp_dir().join(name);
        std::fs::write(&system, text).map_err(|e| AiError::Cli(format!("Claude Code: the prompt's file: {e}")))?;
        Ok(ClaudeCode { program, model, options, system, schema, prompt, vocabulary })
    }

    fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            // Neither the user's settings, hooks, plugins, MCP servers nor tools: one answer.
            "--safe-mode",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--tools",
            "",
            "--no-session-persistence",
        ]
        .map(String::from)
        .into();
        args.extend(["--model".into(), self.model.id.into(), "--effort".into(), self.options.effort.clone()]);
        args.extend(["--system-prompt-file".into(), self.system.display().to_string()]);
        args.extend(["--json-schema".into(), self.schema.to_string()]);
        args
    }

    fn message(photo: &PhotoRequest) -> String {
        let data = base64::engine::general_purpose::STANDARD.encode(&photo.jpeg);
        let message = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": data}},
                    {"type": "text", "text": photo.context}
                ]
            }
        });
        format!("{message}\n")
    }

    /// Runs Claude Code on one photo. It is killed on a stop or after [`CALL_TIMEOUT`];
    /// its output is read as it comes, so neither pipe fills and stalls it.
    fn call(&self, photo: &PhotoRequest) -> Result<(Value, u64), AiError> {
        let started = Instant::now();
        let mut child = command(&self.program)
            .args(self.args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AiError::Cli(format!("Claude Code would not start: {e}")))?;
        let mut stdin = child.stdin.take().expect("piped");
        // Written from another thread: a photo is larger than the pipe, and Claude Code
        // starts writing before it has read it all. Dropping it closes Claude Code's input.
        let message = Self::message(photo);
        std::thread::spawn(move || stdin.write_all(message.as_bytes()));
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
                Err(e) => break Err(AiError::BadReply(format!("Claude Code: {e}"))),
            }
            if self.options.stopped() {
                break Err(AiError::Stopped);
            }
            if started.elapsed() > CALL_TIMEOUT {
                break Err(AiError::BadReply(format!(
                    "no answer from Claude Code in {} minutes",
                    CALL_TIMEOUT.as_secs() / 60
                )));
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if let Err(e) = ended {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        let millis = started.elapsed().as_millis() as u64;
        let stdout = stdout.join().unwrap_or_default();
        let result = String::from_utf8_lossy(&stdout)
            .lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|event| event["type"] == "result");
        // No result: this photo failed; Claude Code itself ran, so the run goes on.
        let result = result.ok_or_else(|| {
            let stderr = stderr.join().unwrap_or_default();
            let stderr = String::from_utf8_lossy(&stderr);
            AiError::BadReply(format!("Claude Code gave no answer: {}", stderr.lines().last().unwrap_or("").trim()))
        })?;
        Ok((result, millis))
    }
}

impl Drop for ClaudeCode {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.system);
    }
}

/// Reads the final `result` event: an error first, then the structured answer.
fn read(result: &Value) -> Result<(String, RawAnswer), AiError> {
    if result["is_error"].as_bool() == Some(true) {
        let message = result["result"].as_str().unwrap_or("Claude Code failed").to_string();
        return Err(match result["api_error_status"].as_u64() {
            Some(401 | 403) => AiError::BadKey(message),
            Some(429) => AiError::Limit(message),
            Some(status) => AiError::Http { status: status as u16, message, retry_after: None },
            None if message.contains("/login") || message.contains("Not logged in") => AiError::BadKey(message),
            None => AiError::BadReply(message),
        });
    }
    match result["stop_reason"].as_str() {
        Some("refusal") => return Err(AiError::Refused(None)),
        Some("max_tokens") => return Err(AiError::Truncated),
        _ => {}
    }
    let output = &result["structured_output"];
    if !output.is_object() {
        return Err(AiError::BadReply(result["result"].as_str().unwrap_or("no structured answer").to_string()));
    }
    let answer = serde_json::from_value::<RawAnswer>(output.clone()).map_err(|e| AiError::BadReply(e.to_string()))?;
    Ok((output.to_string(), answer))
}

impl VisionProvider for ClaudeCode {
    fn id(&self) -> &str {
        "claude-code"
    }

    fn model(&self) -> &str {
        self.model.id
    }

    fn prompt_hash(&self) -> u64 {
        self.prompt
    }

    fn describe(&self, photo: &PhotoRequest) -> Result<Reply, Failed> {
        // Claude Code retries rate limits and overload itself; this is for the rest.
        let mut attempt = 0;
        let (result, millis) = loop {
            attempt += 1;
            match self.call(photo) {
                Ok(done) => break done,
                Err(e) if e.retryable() && attempt < self.options.attempts.min(2) && !self.options.stopped() => {
                    std::thread::sleep(Duration::from_secs(2u64.pow(attempt)));
                }
                Err(e) => return Err(e.into()),
            }
        };
        let usage: Usage = serde_json::from_value(result["usage"].clone()).unwrap_or_default();
        let record = |raw: String| RunRecord {
            provider: "claude-code".to_string(),
            model: self.model.id.to_string(),
            prompt: self.prompt,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_write_tokens: usage.cache_creation_input_tokens,
            cache_read_tokens: usage.cache_read_input_tokens,
            // Counted against the plan, not billed.
            cost: 0.0,
            millis,
            raw,
        };
        let (raw, answer) =
            read(&result).map_err(|error| Failed { error, record: Some(Box::new(record(result.to_string()))) })?;
        let (suggestion, warnings) = validate(&answer, &self.vocabulary);
        Ok(Reply { suggestion, warnings, record: record(raw) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The last line Claude Code prints, as it printed it for a photo (trimmed).
    const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"tool_use","api_error_status":null,"total_cost_usd":0.0012,
      "usage":{"input_tokens":2,"cache_creation_input_tokens":3671,"cache_read_input_tokens":0,"output_tokens":905},
      "result":"","structured_output":{"caption_en":"Three men play xiangqi at a stone table.","caption_pl":"","headline":"Street chess","subject":"xiangqi","keywords":["street"],"keywords_new":["xiangqi"],"rating":3,"quality":4,"composition":3,"confidence":0.9,"location_guess":{"country":{"name":"China","p":0.5},"state":{"name":"","p":0},"city":{"name":"Hong Kong","p":0.5},"location":{"name":"","p":0},"coordinates":{"lat":0,"lon":0,"p":0,"radius_km":0}}}}"#;

    #[test]
    fn reads_the_structured_answer() {
        let result: Value = serde_json::from_str(RESULT).unwrap();
        let (raw, answer) = read(&result).unwrap();
        assert_eq!(answer.headline, "Street chess");
        assert_eq!(answer.keywords_new, vec!["xiangqi"]);
        assert!(raw.starts_with('{'));
    }

    #[test]
    fn errors_say_what_to_do() {
        let error = |status: Value, text: &str| {
            read(&json!({"type": "result", "is_error": true, "api_error_status": status, "result": text})).unwrap_err()
        };
        assert!(matches!(error(json!(429), "usage limit reached"), AiError::Limit(_)));
        assert!(matches!(error(json!(401), "invalid"), AiError::BadKey(_)));
        assert!(matches!(error(Value::Null, "Not logged in · Please run /login"), AiError::BadKey(_)));
        assert!(error(json!(529), "overloaded").retryable());
        assert!(!error(Value::Null, "model unknown").retryable());
        let refused = json!({"type": "result", "is_error": false, "stop_reason": "refusal"});
        assert!(matches!(read(&refused), Err(AiError::Refused(None))));
    }

    #[test]
    fn asks_for_the_schema_without_tools_and_sends_the_photo() {
        let brief = Brief { vocabulary: vec!["street".into()], ..Default::default() };
        let cli = ClaudeCode::new("claude".into(), Options::default(), &brief, Vocabulary::default()).unwrap();
        let args = cli.args();
        let after = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
        assert_eq!(after("--tools"), "");
        assert_eq!(after("--model"), "claude-opus-5-5");
        assert_eq!(serde_json::from_str::<Value>(&after("--json-schema")).unwrap(), crate::schema(&brief.fields));
        assert!(std::fs::read_to_string(after("--system-prompt-file")).unwrap().contains("street"));
        let message = ClaudeCode::message(&PhotoRequest { jpeg: vec![1, 2, 3], context: "DSCF0001.RAF".into() });
        let message: Value = serde_json::from_str(message.trim_end()).unwrap();
        assert_eq!(message["message"]["content"][0]["source"]["data"], "AQID");
        assert_eq!(message["message"]["content"][1]["text"], "DSCF0001.RAF");
    }

    #[test]
    fn a_missing_program_stops_the_run_and_the_prompt_file_goes_with_the_provider() {
        let missing = std::env::temp_dir().join("c1studio-no-such-dir").join("claude.exe");
        let cli = ClaudeCode::new(missing, Options::default(), &Brief::default(), Vocabulary::default()).unwrap();
        let file = cli.system.clone();
        assert!(file.is_file());
        let failed = cli.describe(&PhotoRequest { jpeg: vec![1], context: String::new() }).unwrap_err();
        assert!(matches!(failed.error, AiError::Cli(_)) && failed.error.stops_run() && failed.record.is_none());
        drop(cli);
        assert!(!file.exists());
    }

    #[test]
    fn a_failed_answer_is_kept_with_its_record() {
        let error = read(&json!({"type": "result", "is_error": false, "stop_reason": "end_turn", "result": "no"}));
        assert!(matches!(error, Err(AiError::BadReply(_))));
        assert!(!AiError::BadReply("x".into()).stops_run());
        assert!(
            AiError::Http { status: 400, message: "Your credit balance is too low".into(), retry_after: None }
                .stops_run()
        );
    }
}
