//! Describing photos with a vision model (stage 5): the run in the background, its cost
//! up front, the API key, and accepting or rejecting what was suggested.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use c1ai::claude::{self, Claude};
use c1ai::claude_code::{self, ClaudeCode};
use c1ai::keys::{self, ANTHROPIC};
use c1ai::{AiError, Brief, Language, PhotoRequest, VisionProvider, Vocabulary};
use c1index::Accept;
use c1index::ai::{Entry, Status};
use c1model::{ImageTier, Op, Photo};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{App, Lowers, Session, is_current, text};

/// What the user chose for describing photos, kept per catalog in `ai.json`.
/// How Claude is reached: the API with a key, billed per token, or Claude Code signed
/// in with a Claude plan, counted against the plan's limits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Access {
    #[default]
    Api,
    Plan,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AiSettings {
    access: Access,
    model: String,
    language: Language,
    /// `low`, `medium`, `high`.
    effort: String,
    /// Photos described at once.
    parallel: usize,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            access: Access::Api,
            model: claude::DEFAULT_MODEL.into(),
            language: Language::En,
            effort: "low".into(),
            parallel: 4,
        }
    }
}

fn settings_path(s: &Session) -> PathBuf {
    s.settings_path.with_file_name("ai.json")
}

fn load_settings(s: &Session) -> AiSettings {
    std::fs::read(settings_path(s)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// How far a run has got, for the footer.
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct DescribeState {
    running: bool,
    done: usize,
    total: usize,
    failed: usize,
    /// US dollars spent by this run.
    cost: f64,
    /// Why the run stopped early (no key, a refused key), or the last photo's failure.
    error: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ModelView {
    id: &'static str,
    name: &'static str,
    input: f64,
    output: f64,
}

/// Claude Code as a plan run would use it.
#[derive(Serialize)]
pub(crate) struct PlanView {
    installed: bool,
    #[serde(flatten)]
    status: claude_code::Status,
    /// Why the status could not be read.
    error: Option<String>,
}

fn plan_view() -> PlanView {
    let Some(program) = claude_code::find() else {
        return PlanView { installed: false, status: Default::default(), error: None };
    };
    match claude_code::status(&program) {
        Ok(status) => PlanView { installed: true, status, error: None },
        Err(e) => PlanView { installed: true, status: Default::default(), error: Some(e.to_string()) },
    }
}

#[derive(Serialize)]
pub(crate) struct AiView {
    settings: AiSettings,
    models: Vec<ModelView>,
    /// The key in use, masked, and where it comes from (`stored`, `environment`).
    key: Option<(String, &'static str)>,
    /// Claude Code, looked at only when the plan is chosen: it takes a second to ask.
    plan: Option<PlanView>,
    /// Whether a run can start: a key, or Claude Code signed in.
    ready: bool,
    /// Calls made in this library and what they cost in all.
    calls: u64,
    spent: f64,
}

fn view(s: &Session) -> Result<AiView, String> {
    let settings = load_settings(s);
    let key = keys::get(&ANTHROPIC).map_err(text)?;
    let plan = (settings.access == Access::Plan).then(plan_view);
    let ready = match &plan {
        Some(plan) => plan.status.logged_in,
        None => key.is_some(),
    };
    let (calls, spent) = s.library.ai_spent().map_err(text)?;
    Ok(AiView {
        settings,
        models: claude::MODELS
            .iter()
            .map(|m| ModelView { id: m.id, name: m.name, input: m.input, output: m.output })
            .collect(),
        key: key.map(|k| (keys::masked(&k), keys::origin(&ANTHROPIC).unwrap_or("stored"))),
        plan,
        ready,
        calls,
        spent,
    })
}

#[tauri::command]
pub(crate) async fn ai_settings(app: State<'_, App>) -> Result<AiView, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || view(&s)).await.map_err(text)?
}

#[tauri::command]
pub(crate) async fn set_ai_settings(app: State<'_, App>, settings: AiSettings) -> Result<AiView, String> {
    let s = app.session()?;
    if claude::model(&settings.model).is_none() {
        return Err(format!("unknown model {}", settings.model));
    }
    let settings = AiSettings { parallel: settings.parallel.clamp(1, 8), ..settings };
    std::fs::write(settings_path(&s), serde_json::to_vec_pretty(&settings).map_err(text)?).map_err(text)?;
    tauri::async_runtime::spawn_blocking(move || view(&s)).await.map_err(text)?
}

/// Stores the API key in the Windows Credential Manager; an empty one removes it.
#[tauri::command]
pub(crate) async fn set_ai_key(app: State<'_, App>, key: String) -> Result<AiView, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        keys::set(&ANTHROPIC, &key).map_err(text)?;
        view(&s)
    })
    .await
    .map_err(text)?
}

#[derive(Serialize)]
pub(crate) struct Estimate {
    /// Photos a run would describe.
    photos: usize,
    /// Photos left out: described already with this model and prompt.
    skipped: usize,
    /// US dollars, roughly; nothing on a plan.
    cost: f64,
    model: &'static str,
    /// Counted against the Claude plan's limits instead.
    plan: bool,
}

/// The instructions for a run with these settings, and the vocabulary answers are
/// checked against.
fn brief(s: &Session, settings: &AiSettings) -> Result<(Brief, Vocabulary), String> {
    let paths = s.library.vocabulary().map_err(text)?;
    let brief = Brief {
        language: settings.language,
        vocabulary: paths.iter().map(|(p, _)| p.rsplit('|').next().unwrap_or(p).to_string()).fold(
            Vec::new(),
            |mut names, n| {
                if !names.contains(&n) {
                    names.push(n);
                }
                names
            },
        ),
    };
    let vocabulary = Vocabulary::new(paths.iter().map(|(p, n)| (p.as_str(), *n)));
    Ok((brief, vocabulary))
}

/// The provider for a run with these settings: the API with the stored key, or Claude
/// Code signed in with a plan.
fn provider(s: &Session, settings: &AiSettings) -> Result<Box<dyn VisionProvider>, String> {
    let (brief, vocabulary) = brief(s, settings)?;
    let options = claude::Options { model: settings.model.clone(), effort: settings.effort.clone(), attempts: 4 };
    Ok(match settings.access {
        Access::Api => {
            let key = keys::get(&ANTHROPIC).map_err(text)?.ok_or_else(|| AiError::NoKey.to_string())?;
            Box::new(Claude::new(key, options, &brief, vocabulary).map_err(text)?)
        }
        Access::Plan => {
            let program = claude_code::find().ok_or_else(|| {
                AiError::Cli("not installed: get it from claude.com/claude-code and sign in".into()).to_string()
            })?;
            Box::new(ClaudeCode::new(program, options, &brief, vocabulary).map_err(text)?)
        }
    })
}

/// Of `rows`, the photos a run would describe (`again` takes those described before
/// too), and what that would cost.
fn plan(s: &Session, rows: &[u64], again: bool) -> Result<(Vec<u64>, Estimate), String> {
    let settings = load_settings(s);
    let model = claude::model(&settings.model).ok_or("unknown model")?;
    // Both ways send the same prompt, so a photo described one way counts for the other.
    let prompt = c1ai::prompt_hash(&c1ai::system_prompt(&brief(s, &settings)?.0));
    let done = if again { Default::default() } else { s.library.described(rows, model.id, prompt).map_err(text)? };
    let todo: Vec<u64> = rows.iter().copied().filter(|r| !done.contains(r)).collect();
    let on_plan = settings.access == Access::Plan;
    let estimate = Estimate {
        photos: todo.len(),
        skipped: rows.len() - todo.len(),
        cost: if on_plan { 0.0 } else { claude::estimate(&model, todo.len()) },
        model: model.name,
        plan: on_plan,
    };
    Ok((todo, estimate))
}

#[tauri::command]
pub(crate) async fn ai_estimate(app: State<'_, App>, rows: Vec<u64>, again: bool) -> Result<Estimate, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || plan(&s, &rows, again).map(|(_, e)| e)).await.map_err(text)?
}

/// One line of what the catalog knows of a photo: context for the model.
fn context(photo: &Photo) -> String {
    let mut parts = vec![photo.file.name.clone()];
    if let Some(at) = photo.captured_at.and_then(chrono::DateTime::from_timestamp_millis) {
        parts.push(at.format("%Y-%m-%d %H:%M").to_string());
    }
    parts.extend(photo.exif.camera_model.clone());
    parts.extend(photo.exif.lens.clone());
    let place: Vec<&str> = [&photo.place.location, &photo.place.city, &photo.place.country]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
    if !place.is_empty() {
        parts.push(format!("place: {}", place.join(", ")));
    }
    if !photo.keywords.is_empty() {
        let keywords: Vec<String> = photo.keywords.iter().map(|k| k.to_pipe()).collect();
        parts.push(format!("keywords already: {}", keywords.join(", ")));
    }
    format!("Catalog data: {}", parts.join(" · "))
}

/// The photo as the model sees it: its preview (else its thumbnail), at most
/// [`claude::LONG_EDGE`] on the long side.
fn request(s: &Session, row: u64) -> anyhow::Result<Option<PhotoRequest>> {
    let (Some(id), Some(photo)) = (s.library.photo_id(row), s.library.photo(row)?) else { return Ok(None) };
    let image = match s.source.image_or_make(&id, ImageTier::Preview)? {
        Some(image) => Some(image),
        None => s.thumbs.get_or_make(s.source.as_ref(), &id)?.map(|made| made.image),
    };
    let Some(image) = image else { return Ok(None) };
    let pixels = c1image::load_fitted(&image, claude::LONG_EDGE)?;
    Ok(Some(PhotoRequest { jpeg: c1image::encode_jpeg(&pixels, 85), context: context(&photo) }))
}

/// Describes these photos in the background, a few at a time, unless a run is under way.
/// Each answer is stored as it comes, so a run stopped or crashed keeps what it got.
#[tauri::command]
pub(crate) async fn start_describe(app: AppHandle, rows: Vec<u64>, again: bool) -> Result<(), String> {
    let s = app.state::<App>().session()?;
    if s.describing.swap(true, Ordering::SeqCst) {
        return Err("Photos are being described already".into());
    }
    let started = (|| {
        let settings = load_settings(&s);
        let (todo, _) = plan(&s, &rows, again)?;
        Ok::<_, String>((
            Arc::<dyn VisionProvider>::from(provider(&s, &settings)?),
            todo,
            settings.parallel.clamp(1, 8),
        ))
    })();
    let (provider, todo, parallel) = match started {
        Ok(started) => started,
        Err(e) => {
            s.describing.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    s.describe_stop.store(false, Ordering::SeqCst);
    *s.describe.lock().unwrap() = DescribeState { running: true, total: todo.len(), ..DescribeState::default() };
    std::thread::spawn(move || {
        let _done = Lowers(&s.describing);
        let next = AtomicUsize::new(0);
        let state = Mutex::new(DescribeState { running: true, total: todo.len(), ..DescribeState::default() });
        let report = |state: &DescribeState| {
            *s.describe.lock().unwrap() = state.clone();
            if is_current(&app, &s) {
                let _ = app.emit("describe", state.clone());
            }
        };
        std::thread::scope(|scope| {
            for _ in 0..parallel.min(todo.len().max(1)) {
                scope.spawn(|| {
                    while !s.describe_stop.load(Ordering::SeqCst) && is_current(&app, &s) {
                        let Some(&row) = todo.get(next.fetch_add(1, Ordering::SeqCst)) else { break };
                        let outcome = describe_one(&s, provider.as_ref(), row);
                        let mut st = state.lock().unwrap();
                        st.done += 1;
                        match outcome {
                            Ok(cost) => st.cost += cost,
                            Err(e) => {
                                st.failed += 1;
                                let message = format!("{e:#}");
                                s.log.event(&format!("Describing photo {row} failed: {message}"), &[]);
                                // Without a working key, Claude Code or what is left of the
                                // plan no photo will do: the run stops.
                                if let Some(AiError::NoKey | AiError::BadKey(_) | AiError::Cli(_) | AiError::Limit(_)) =
                                    e.downcast_ref::<AiError>()
                                {
                                    s.describe_stop.store(true, Ordering::SeqCst);
                                }
                                st.error = Some(message);
                            }
                        }
                        report(&st);
                    }
                });
            }
        });
        let last = DescribeState { running: false, ..state.into_inner().unwrap() };
        s.log.event(
            &format!("Described {} photos, {} failed, ${:.4}", last.done - last.failed, last.failed, last.cost),
            &[],
        );
        report(&last);
        if is_current(&app, &s) {
            let _ = app.emit("describe-done", ());
        }
    });
    Ok(())
}

/// Describes one photo and stores the suggestion. Returns what the call cost.
fn describe_one(s: &Session, provider: &dyn VisionProvider, row: u64) -> anyhow::Result<f64> {
    let Some(request) = request(s, row)? else { anyhow::bail!("no preview or thumbnail") };
    let reply = provider.describe(&request)?;
    let entry = Entry {
        at: chrono::Utc::now().timestamp_millis(),
        provider: provider.id().to_string(),
        model: reply.record.model.clone(),
        prompt: provider.prompt_hash(),
        suggestion: reply.suggestion,
        warnings: reply.warnings,
        cost: reply.record.cost,
        status: Status::Proposed,
    };
    s.library.put_suggestion(row, &entry, &reply.record)?;
    Ok(reply.record.cost)
}

/// Stops the run after the photos under way.
#[tauri::command]
pub(crate) async fn stop_describe(app: State<'_, App>) -> Result<(), String> {
    app.session()?.describe_stop.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub(crate) async fn describe_state(app: State<'_, App>) -> Result<DescribeState, String> {
    Ok(app.session()?.describe.lock().unwrap().clone())
}

#[derive(Serialize)]
pub(crate) struct SuggestionRow {
    row: u64,
    #[serde(flatten)]
    entry: Entry,
}

/// Stored suggestions, newest first: `proposed`, `accepted`, `rejected`, or all.
#[tauri::command]
pub(crate) async fn suggestions(app: State<'_, App>, status: Option<Status>) -> Result<Vec<SuggestionRow>, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        let list = s.library.suggestions(status).map_err(text)?;
        Ok(list.into_iter().map(|(row, entry)| SuggestionRow { row, entry }).collect())
    })
    .await
    .map_err(text)?
}

#[tauri::command]
pub(crate) async fn suggestion(app: State<'_, App>, row: u64) -> Result<Option<Entry>, String> {
    app.session()?.library.suggestion(row).map_err(text)
}

/// Queues what the user took from suggestions. Returns the ops that undo it.
#[tauri::command]
pub(crate) async fn accept_suggestions(app: State<'_, App>, picks: Vec<Accept>) -> Result<Vec<(u64, Op)>, String> {
    let s = app.session()?;
    s.touched();
    tauri::async_runtime::spawn_blocking(move || s.library.accept_suggestions(&picks).map_err(|e| format!("{e:#}")))
        .await
        .map_err(text)?
}

/// Sets suggestions aside (`rejected`), or back to `proposed`.
#[tauri::command]
pub(crate) async fn set_suggestion_status(app: State<'_, App>, rows: Vec<u64>, status: Status) -> Result<(), String> {
    app.session()?.library.set_suggestion_status(&rows, status).map_err(text)
}
