//! Describing photos with a vision model (stage 5): the run in the background, its cost
//! up front, the API key, and accepting or rejecting what was suggested.

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use c1ai::agy::{self, Agy};
use c1ai::claude::{self, Claude};
use c1ai::claude_code::{self, ClaudeCode};
use c1ai::keys::{self, ANTHROPIC};
use c1ai::{
    AiError, Brief, Failed, Fields, Language, LeftOut, PhotoRequest, Progress, SheetRequest, Step, VisionProvider,
    Vocabulary,
};
use c1index::ai::{Entry, Status};
use c1index::{Accept, Accepted};
use c1model::{ImageRef, ImageTier, Photo, PhotoId, Place};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{App, Lowers, Session, is_current, text};

/// How a model is reached: Claude through the API with a key, billed per token; Claude
/// Code signed in with a Claude plan, counted against the plan's limits; or Gemini
/// through agy, counted against the Google account's quota.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Access {
    #[default]
    Api,
    Plan,
    Gemini,
}

/// The image a model is sent of each photo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AiImage {
    /// The source's thumbnail: for Capture One its own render of the edits, about 450 by
    /// 300, so the colours are the ones the user sees there.
    #[default]
    Thumb,
    /// The preview, up to [`claude::LONG_EDGE`]: more detail, but for Capture One its
    /// colours are matched here and can be off.
    Preview,
}

/// What the user chose for describing photos, kept per catalog in `ai.json`.
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
    /// What a description holds.
    fields: Fields,
    /// The user's own instructions in place of [`c1ai::INSTRUCTIONS`].
    instructions: Option<String>,
    /// Lessons drawn from the user's corrections (Refine in the AI tab), or written by hand:
    /// they follow the instructions.
    lessons: Option<String>,
    /// When the lessons last changed, Unix ms: corrections made before are what they came from.
    lessons_at: Option<i64>,
    /// A place level (or position) guessed at least this likely (%) is ticked in the review
    /// where the photo has none.
    place_tick: u8,
    /// The Gemini model agy is asked for.
    gemini_model: String,
    /// Gemini gets contact sheets of up to 36 photos rather than one photo per call.
    sheets: bool,
    /// The image sent of each photo, alone or on a sheet.
    image: AiImage,
    /// Keywords not offered to the model, nor matched in its answers.
    left_out: LeftOut,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            access: Access::Api,
            model: claude::DEFAULT_MODEL.into(),
            language: Language::En,
            effort: "low".into(),
            parallel: 4,
            fields: Fields::default(),
            instructions: None,
            lessons: None,
            lessons_at: None,
            place_tick: 80,
            gemini_model: agy::DEFAULT_MODEL.into(),
            sheets: false,
            image: AiImage::Thumb,
            left_out: LeftOut::default(),
        }
    }
}

fn settings_path(s: &Session) -> PathBuf {
    s.settings_path.with_file_name("ai.json")
}

fn load_settings(s: &Session) -> AiSettings {
    std::fs::read(settings_path(s)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// How far a run has got, for the footer and the AI tab.
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct DescribeState {
    running: bool,
    done: usize,
    total: usize,
    failed: usize,
    /// The photos being described now.
    working: Vec<u64>,
    /// When the run started, Unix ms: with `done`, how long the rest will take.
    started: i64,
    /// US dollars spent by this run.
    cost: f64,
    /// Why the run stopped early (no key, a refused key), or the last photo's failure.
    error: Option<String>,
    /// Photos go as contact sheets, a call each.
    sheets: bool,
    /// Calls made and to make: one per photo, or one per contact sheet.
    calls_done: usize,
    calls_total: usize,
    /// The calls under way, in the order they began.
    units: Vec<UnitView>,
    /// What the run did, oldest first, at most [`LOG_LINES`]; kept after the run ends and
    /// cleared when the next one starts.
    log: Vec<LogLine>,
}

/// A call under way: one photo, or a contact sheet of several.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct UnitView {
    /// From 1, in the run's order: a sheet's number.
    call: usize,
    rows: Vec<u64>,
    sheet: bool,
    /// `preparing` (its image being made), `waiting` (sent, the answer awaited) or
    /// `retrying` (a try failed in a way that may pass; another follows after a pause).
    phase: &'static str,
    /// When it entered the phase, Unix ms.
    since: i64,
    /// When the call began, Unix ms.
    began: i64,
    /// The try under way, from 1; 0 while preparing.
    attempt: u32,
    /// Why it is trying again.
    note: Option<String>,
}

/// A line of what a run did.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct LogLine {
    /// Unix ms.
    at: i64,
    /// The call it is about; 0 for the run as a whole.
    call: usize,
    /// False for a failure, or a try that failed.
    ok: bool,
    text: String,
}

/// Lines a run's log keeps.
const LOG_LINES: usize = 200;

/// Contact sheets kept to look at: those of the calls under way and the last few before.
const SHEETS_KEPT: usize = 8;

impl DescribeState {
    fn note(&mut self, call: usize, ok: bool, text: String) {
        self.log.push(LogLine { at: now(), call, ok, text });
        if self.log.len() > LOG_LINES {
            self.log.drain(..self.log.len() - LOG_LINES);
        }
    }

    fn unit(&mut self, call: usize) -> Option<&mut UnitView> {
        self.units.iter_mut().find(|u| u.call == call)
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// How the log names a call: "Sheet 2", "Photo 7".
fn call_name(call: usize, sheet: bool) -> String {
    format!("{} {call}", if sheet { "Sheet" } else { "Photo" })
}

/// Shows the run's state where it is looked at: the AI tab and the footer. Sent under
/// the lock, so states from the run's threads arrive in the order they were made.
fn publish(app: &AppHandle, s: &Arc<Session>) {
    let state = lock(&s.describe);
    if is_current(app, s) {
        let _ = app.emit("describe", state.clone());
    }
}

/// Hears how call `call` goes and keeps its phase and the log up to date. A photo's call
/// is logged only when it is tried again: one line per photo would bury the rest.
fn progress_for(app: &AppHandle, s: &Arc<Session>, call: usize, sheet: bool) -> Progress {
    let (app, s) = (app.clone(), Arc::clone(s));
    Arc::new(move |step| {
        {
            let mut st = lock(&s.describe);
            let name = call_name(call, sheet);
            match step {
                Step::Sent { attempt } => {
                    if let Some(unit) = st.unit(call) {
                        unit.phase = "waiting";
                        unit.since = now();
                        unit.attempt = attempt;
                    }
                    if attempt > 1 {
                        st.note(call, true, format!("{name}: sent again, try {attempt}"));
                    } else if sheet {
                        st.note(call, true, format!("{name}: sent"));
                    }
                }
                Step::Retrying { attempt, of, why } => {
                    let note = format!("try {attempt} of {of} failed: {why}");
                    if let Some(unit) = st.unit(call) {
                        unit.phase = "retrying";
                        unit.since = now();
                        unit.note = Some(note.clone());
                    }
                    st.note(call, false, format!("{name}: {note}; trying again"));
                }
            }
        }
        publish(&app, &s);
    })
}

#[derive(Serialize)]
pub(crate) struct ModelView {
    id: String,
    name: String,
    input: f64,
    output: f64,
}

/// agy as a Gemini run would use it: whether it is there, and its Gemini models.
struct GeminiView {
    installed: bool,
    models: Vec<(String, String)>,
    /// Why its models could not be listed (offline, not signed in).
    error: Option<String>,
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
    /// With Gemini chosen: whether agy is installed.
    gemini: Option<bool>,
    /// Why agy's models could not be listed.
    gemini_error: Option<String>,
    /// Whether a run can start: a key, Claude Code signed in, or agy there.
    ready: bool,
    /// Calls made in this library and what they cost in all.
    calls: u64,
    spent: f64,
    /// The instructions a run gives when the user wrote none.
    default_instructions: &'static str,
}

/// Whether Claude Code runs on a Claude plan: signed in through claude.ai. Signed in with
/// an API key or a cloud provider it is billed per token, which plan access would show
/// as free, so it does not count.
fn on_plan(status: &claude_code::Status) -> bool {
    status.logged_in && status.auth_method.as_deref() == Some("claude.ai")
}

fn view(s: &Session) -> Result<AiView, String> {
    let settings = load_settings(s);
    let key = keys::get(&ANTHROPIC).map_err(text)?;
    let plan = (settings.access == Access::Plan).then(plan_view);
    // agy is asked for its models only when Gemini is chosen: it takes a moment.
    let gemini = (settings.access == Access::Gemini).then(|| {
        let program = agy::find();
        let listed = program.as_deref().map(agy::models);
        let error = listed.as_ref().and_then(|l| l.as_ref().err()).map(|e| e.to_string());
        let models = listed.and_then(Result::ok).unwrap_or_default();
        GeminiView { installed: program.is_some(), models, error }
    });
    let ready = match (&plan, &gemini) {
        (Some(plan), _) => on_plan(&plan.status),
        (_, Some(gemini)) => gemini.installed,
        _ => key.is_some(),
    };
    let (calls, spent) = s.library.ai_spent().map_err(text)?;
    let models = match &gemini {
        Some(g) => g
            .models
            .iter()
            .map(|(id, name)| ModelView { id: id.clone(), name: name.clone(), input: 0.0, output: 0.0 })
            .collect(),
        None => claude::MODELS
            .iter()
            .map(|m| ModelView { id: m.id.into(), name: m.name.into(), input: m.input, output: m.output })
            .collect(),
    };
    Ok(AiView {
        settings,
        models,
        gemini_error: gemini.as_ref().and_then(|g| g.error.clone()),
        gemini: gemini.map(|g| g.installed),
        key: key.map(|k| (keys::masked(&k), keys::origin(&ANTHROPIC).unwrap_or("stored"))),
        plan,
        ready,
        calls,
        spent,
        default_instructions: c1ai::INSTRUCTIONS,
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
    if settings.gemini_model.trim().is_empty() {
        return Err("Choose a Gemini model".into());
    }
    if !settings.fields.any() {
        return Err("Pick at least one thing to describe".into());
    }
    if !claude::EFFORTS.contains(&settings.effort.as_str()) {
        return Err(format!("unknown effort {}", settings.effort));
    }
    // The default instructions, or none written, are kept as "the default": a later
    // version of them then reaches this catalog too.
    let instructions =
        settings.instructions.map(|i| i.trim().to_string()).filter(|i| !i.is_empty() && i != c1ai::INSTRUCTIONS.trim());
    let lessons = settings.lessons.map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
    if lessons.as_ref().is_some_and(|l| l.chars().count() > MAX_LESSONS_TEXT) {
        return Err(format!("Lessons are limited to {MAX_LESSONS_TEXT} characters: they go into every call"));
    }
    let before = load_settings(&s);
    let lessons_at = if lessons == before.lessons { before.lessons_at } else { lessons.as_ref().map(|_| now()) };
    let settings = AiSettings {
        parallel: settings.parallel.clamp(1, 8),
        place_tick: settings.place_tick.clamp(1, 100),
        instructions,
        lessons,
        lessons_at,
        ..settings
    };
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
    model: String,
    /// Counted against the Claude plan's (or the Google account's) limits instead.
    plan: bool,
    /// The calls it takes: one per photo, or per contact sheet.
    calls: usize,
    /// What the calls go to: "Claude", "Gemini".
    via: &'static str,
}

/// The instructions for a run with these settings, and the vocabulary answers are
/// checked against.
fn brief(s: &Session, settings: &AiSettings) -> Result<(Brief, Vocabulary), String> {
    let (paths, withheld): (Vec<_>, Vec<_>) =
        s.library.vocabulary().map_err(text)?.into_iter().partition(|(path, _)| !settings.left_out.covers(path));
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
        fields: settings.fields,
        instructions: settings.instructions.clone(),
        lessons: settings.lessons.clone(),
    };
    let vocabulary = Vocabulary::new(paths.iter().map(|(p, n)| (p.as_str(), *n)))
        .withholding(withheld.iter().map(|(p, _)| p.as_str()));
    Ok((brief, vocabulary))
}

/// The prompt a run with the current settings would send, for the user to read.
#[derive(Serialize)]
pub(crate) struct PromptView {
    system: String,
    schema: serde_json::Value,
}

#[tauri::command]
pub(crate) async fn ai_prompt(app: State<'_, App>) -> Result<PromptView, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        let settings = load_settings(&s);
        let (brief, _) = brief(&s, &settings)?;
        Ok(PromptView { system: c1ai::system_prompt(&brief), schema: c1ai::schema(&brief.fields) })
    })
    .await
    .map_err(text)?
}

/// The provider for a run with these settings: the API with the stored key, or Claude
/// Code signed in with a plan.
fn provider(s: &Session, settings: &AiSettings) -> Result<Box<dyn VisionProvider>, String> {
    let (brief, vocabulary) = brief(s, settings)?;
    let options = claude::Options {
        model: settings.model.clone(),
        effort: settings.effort.clone(),
        attempts: 4,
        stop: s.describe_stop.clone(),
    };
    Ok(match settings.access {
        Access::Api => {
            let key = keys::get(&ANTHROPIC).map_err(text)?.ok_or_else(|| AiError::NoKey.to_string())?;
            Box::new(Claude::new(key, options, &brief, vocabulary).map_err(text)?)
        }
        Access::Gemini => {
            let program = agy::find().ok_or_else(|| {
                AiError::Cli("agy is not installed: get the Antigravity CLI and sign in".into()).to_string()
            })?;
            let options = claude::Options { model: settings.gemini_model.clone(), ..options };
            Box::new(Agy::new(program, options, &brief, vocabulary, settings.sheets))
        }
        Access::Plan => {
            let program = claude_code::find().ok_or_else(|| {
                AiError::Cli("Claude Code is not installed: get it from claude.com/claude-code and sign in".into())
                    .to_string()
            })?;
            let status = claude_code::status(&program).map_err(text)?;
            if !on_plan(&status) {
                return Err(match status.auth_method {
                    Some(method) if status.logged_in => format!(
                        "Claude Code is signed in with {method}, billed per token, not a Claude plan: \
                         run `claude auth login` with your plan, or choose API key access"
                    ),
                    _ => "Claude Code is not signed in: run `claude auth login`".into(),
                });
            }
            Box::new(ClaudeCode::new(program, options, &brief, vocabulary).map_err(text)?)
        }
    })
}

/// Of `rows`, the photos a run would describe (`again` takes those described before
/// too), and what that would cost.
fn plan(s: &Session, rows: &[u64], again: bool) -> Result<(Vec<u64>, Estimate), String> {
    let settings = load_settings(s);
    let brief = brief(s, &settings)?.0;
    let (id, name, prompt, price) = match settings.access {
        Access::Gemini => {
            let id = settings.gemini_model.clone();
            (id.clone(), id, agy::prompt_hash(&brief, settings.sheets), None)
        }
        // Both ways send the same prompt, so a photo described one way counts for the other.
        _ => {
            let model = claude::model(&settings.model).ok_or("unknown model")?;
            let price = (settings.access == Access::Api).then_some(model);
            (model.id.to_string(), model.name.to_string(), c1ai::prompt_hash(&brief), price)
        }
    };
    let done = if again { Default::default() } else { s.library.described(rows, &id, prompt).map_err(text)? };
    let todo: Vec<u64> = rows.iter().copied().filter(|r| !done.contains(r)).collect();
    let sheets = settings.access == Access::Gemini && settings.sheets;
    let estimate = Estimate {
        photos: todo.len(),
        skipped: rows.len() - todo.len(),
        cost: price.map_or(0.0, |model| claude::estimate(&model, todo.len(), settings.image == AiImage::Preview)),
        model: name,
        plan: price.is_none(),
        calls: if sheets { todo.len().div_ceil(c1image::PER_SHEET) } else { todo.len() },
        via: if settings.access == Access::Gemini { "Gemini" } else { "Claude" },
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
    // What the photographer wrote: the model builds on it rather than guessing again.
    // Quoted on one line and cut short, so it reads as data and keeps the cost estimated.
    let quoted = |label: &str, value: &Option<String>| {
        value.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(|v| {
            let flat: String = v.split_whitespace().collect::<Vec<_>>().join(" ").replace('"', "'");
            let cut: String = flat.chars().take(CONTEXT_TEXT).collect();
            format!("{label}: \"{cut}{}\"", if cut.len() < flat.len() { "…" } else { "" })
        })
    };
    parts.extend(quoted("title", &photo.iptc.title));
    parts.extend(quoted("headline", &photo.iptc.headline));
    parts.extend(quoted("caption", &photo.iptc.description));
    if let Some(rating) = photo.rating {
        parts.push(format!("rating: {rating}/5"));
    }
    if !photo.keywords.is_empty() {
        let keywords: Vec<String> = photo.keywords.iter().take(CONTEXT_KEYWORDS).map(|k| k.to_pipe()).collect();
        parts.push(format!("keywords already: {}", keywords.join(", ")));
    }
    format!("Catalog data: {}", parts.join(" · "))
}

/// The most of a caption or headline, in characters, and of a photo's keywords, sent
/// with it as context.
const CONTEXT_TEXT: usize = 600;
const CONTEXT_KEYWORDS: usize = 60;

/// The photo as the model sees it, at most [`claude::LONG_EDGE`] on the long side.
fn request(s: &Session, image: AiImage, row: u64, progress: Progress) -> anyhow::Result<Option<PhotoRequest>> {
    let (Some(id), Some(photo)) = (s.library.photo_id(row), s.library.photo(row)?) else { return Ok(None) };
    let Some(pixels) = pixels(s, &id, image, claude::LONG_EDGE)? else { return Ok(None) };
    Ok(Some(PhotoRequest {
        jpeg: c1image::encode_jpeg(&pixels, 85),
        context: context(&photo),
        progress: Some(progress),
    }))
}

/// A photo's pixels for a model, at most `long_edge` on the long side: the image chosen
/// first, then the other one, then a thumbnail made here. The source's own thumbnail is
/// for Capture One its render of the edits (the colours of a preview read from its cache
/// can be off); a missing preview is made if need be. An image that cannot be found, made
/// or loaded gives way to the next; the first such error is returned if none will do.
fn pixels(s: &Session, id: &PhotoId, image: AiImage, long_edge: u32) -> anyhow::Result<Option<c1image::RgbImage>> {
    let mut why: Option<anyhow::Error> = None;
    let mut fitted = |image: anyhow::Result<Option<ImageRef>>| match image
        .and_then(|image| image.map(|r| c1image::load_fitted(&r, long_edge)).transpose().map_err(Into::into))
    {
        Ok(pixels) => pixels,
        Err(e) => {
            why.get_or_insert(e);
            None
        }
    };
    let tiers = match image {
        AiImage::Thumb => [ImageTier::Thumb, ImageTier::Preview],
        AiImage::Preview => [ImageTier::Preview, ImageTier::Thumb],
    };
    for tier in tiers {
        // Only a preview is made: a thumbnail made here comes last, below.
        let found = match tier {
            ImageTier::Preview => s.source.image_or_make(id, tier),
            _ => s.source.image(id, tier),
        };
        if let Some(pixels) = fitted(found.map_err(Into::into)) {
            return Ok(Some(pixels));
        }
    }
    if let Some(pixels) = fitted(s.thumbs.get_or_make(s.source.as_ref(), id).map(|made| made.map(|made| made.image))) {
        return Ok(Some(pixels));
    }
    match why {
        Some(e) => Err(e),
        None => Ok(None),
    }
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
            settings.image,
        ))
    })();
    let (provider, todo, parallel, image) = match started {
        Ok(started) => started,
        Err(e) => {
            s.describing.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    s.describe_stop.store(false, Ordering::SeqCst);
    let sheets = provider.sheets();
    // One photo per call, or a contact sheet of up to 36.
    let units: Vec<Vec<u64>> = if sheets {
        todo.chunks(c1image::PER_SHEET).map(<[u64]>::to_vec).collect()
    } else {
        todo.iter().map(|&row| vec![row]).collect()
    };
    *lock(&s.describe) = DescribeState {
        running: true,
        total: todo.len(),
        started: now(),
        sheets,
        calls_total: units.len(),
        ..Default::default()
    };
    lock(&s.describe_sheets).clear();
    // Shown at once: the first answer can take half a minute, a sheet's a minute or two.
    publish(&app, &s);
    std::thread::spawn(move || {
        let _done = Lowers(&s.describing);
        let next = AtomicUsize::new(0);
        // Photos failed one after another: past FAILING, the rest would fail the same way.
        let failing = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..parallel.min(units.len().max(1)) {
                scope.spawn(|| {
                    while !s.describe_stop.load(Ordering::SeqCst) && is_current(&app, &s) {
                        let index = next.fetch_add(1, Ordering::SeqCst);
                        let Some(unit) = units.get(index) else { break };
                        let call = index + 1;
                        let began = now();
                        {
                            let mut st = lock(&s.describe);
                            st.working.extend(unit);
                            st.units.push(UnitView {
                                call,
                                rows: unit.clone(),
                                sheet: sheets,
                                phase: "preparing",
                                since: began,
                                began,
                                attempt: 0,
                                note: None,
                            });
                        }
                        publish(&app, &s);
                        let progress = progress_for(&app, &s, call, sheets);
                        // A panic is this photo's failure, not the run's end with the
                        // state left saying it runs.
                        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| match unit.as_slice() {
                            [row] if !sheets => describe_one(&s, provider.as_ref(), image, *row, progress)
                                .map(|cost| Described { no_image: 0, unanswered: 0, cost }),
                            rows => describe_sheet(&s, provider.as_ref(), image, rows, call, progress),
                        }))
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("describing it crashed")));
                        let name = call_name(call, sheets);
                        let took = (now() - began) / 1000;
                        let mut st = lock(&s.describe);
                        let tried_again = st.unit(call).is_some_and(|u| u.attempt > 1);
                        st.working.retain(|r| !unit.contains(r));
                        st.units.retain(|u| u.call != call);
                        match outcome {
                            Ok(Described { no_image, unanswered, cost }) => {
                                st.done += unit.len();
                                st.failed += no_image + unanswered;
                                st.cost += cost;
                                st.calls_done += 1;
                                failing.store(0, Ordering::SeqCst);
                                if sheets {
                                    let left_out = if unanswered > 0 {
                                        format!(", {unanswered} missing from the answer")
                                    } else {
                                        String::new()
                                    };
                                    let described = unit.len() - no_image - unanswered;
                                    st.note(
                                        call,
                                        unanswered == 0,
                                        format!("{name}: answered in {took} s, {described} described{left_out}"),
                                    );
                                }
                            }
                            // Stopped mid-call: neither done nor failed.
                            Err(e) if matches!(failed_with(&e), Some(AiError::Stopped)) => {
                                st.note(call, true, format!("{name}: stopped"));
                            }
                            Err(e) => {
                                st.done += unit.len();
                                st.failed += unit.len();
                                st.calls_done += 1;
                                let mut message = format!("{e:#}");
                                // Its tries are in the log; the error says they were made.
                                if tried_again {
                                    message.push_str(" (tried again, to no avail)");
                                }
                                st.note(call, false, format!("{name}: failed after {took} s: {message}"));
                                let what = match unit.as_slice() {
                                    [row] => format!("photo {row}"),
                                    rows => format!("a sheet of {} photos", rows.len()),
                                };
                                s.log.event(&format!("Describing {what} failed: {message}"), &[]);
                                let in_a_row = failing.fetch_add(1, Ordering::SeqCst) + 1;
                                // Without a working key, Claude Code, credit or what is left
                                // of the plan no photo will do: the run stops.
                                if failed_with(&e).is_some_and(AiError::stops_run) {
                                    s.describe_stop.store(true, Ordering::SeqCst);
                                    st.error = Some(message);
                                } else if in_a_row >= FAILING {
                                    s.describe_stop.store(true, Ordering::SeqCst);
                                    st.error =
                                        Some(format!("stopped after {FAILING} photos failed in a row: {message}"));
                                } else {
                                    st.error = Some(message);
                                }
                            }
                        }
                        drop(st);
                        publish(&app, &s);
                    }
                });
            }
        });
        let summary = {
            let mut st = lock(&s.describe);
            st.running = false;
            st.working.clear();
            st.units.clear();
            let summary = format!("Described {} photos, {} failed", st.done - st.failed, st.failed);
            let ok = st.failed == 0;
            st.note(0, ok, summary.clone());
            format!("{summary}, ${:.4}", st.cost)
        };
        s.log.event(&summary, &[]);
        publish(&app, &s);
        if is_current(&app, &s) {
            let _ = app.emit("describe-done", ());
        }
    });
    Ok(())
}

/// Photos failing one after another before a run stops.
const FAILING: usize = 5;

/// A lock that outlives a panic elsewhere: the run's state is plain counters.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The provider's error behind a photo's failure, if it was one.
fn failed_with(e: &anyhow::Error) -> Option<&AiError> {
    e.downcast_ref::<Failed>().map(|f| &f.error)
}

/// Describes one photo and stores the suggestion. Returns what the call cost. A call
/// that was paid for but gave nothing is recorded all the same.
fn describe_one(
    s: &Session,
    provider: &dyn VisionProvider,
    image: AiImage,
    row: u64,
    progress: Progress,
) -> anyhow::Result<f64> {
    let Some(request) = request(s, image, row, progress)? else { anyhow::bail!("no preview or thumbnail") };
    let reply = match provider.describe(&request) {
        Ok(reply) => reply,
        Err(failed) => {
            if let Some(record) = &failed.record {
                s.library.record_call(row, record)?;
            }
            return Err(failed.into());
        }
    };
    let entry = Entry {
        at: chrono::Utc::now().timestamp_millis(),
        provider: provider.id().to_string(),
        // The model asked for, which is what "described already" compares; the record
        // keeps the one that answered (a fallback, on a declined photo).
        model: provider.model().to_string(),
        prompt: provider.prompt_hash(),
        suggestion: reply.suggestion,
        warnings: reply.warnings,
        cost: reply.record.cost,
        status: Status::Proposed,
    };
    s.library.put_suggestion(row, &entry, &reply.record)?;
    Ok(reply.record.cost)
}

/// What a call gave: photos that got no suggestion, for want of an image or of an answer,
/// and what it cost.
struct Described {
    no_image: usize,
    unanswered: usize,
    cost: f64,
}

/// Describes a contact sheet of `rows` in one call (number `call`) and stores each photo's
/// suggestion.
fn describe_sheet(
    s: &Session,
    provider: &dyn VisionProvider,
    image: AiImage,
    rows: &[u64],
    call: usize,
    progress: Progress,
) -> anyhow::Result<Described> {
    let mut cells = Vec::new();
    let mut contexts = Vec::new();
    let mut on_sheet = Vec::new();
    for &row in rows {
        let (Some(id), Some(photo)) = (s.library.photo_id(row), s.library.photo(row)?) else { continue };
        let Some(cell) = pixels(s, &id, image, c1image::SHEET_CELL)? else { continue };
        cells.push(cell);
        contexts.push(context(&photo));
        on_sheet.push(row);
    }
    if on_sheet.is_empty() {
        anyhow::bail!("no preview or thumbnail");
    }
    let jpeg = c1image::encode_jpeg(&c1image::contact_sheet(&cells), 85);
    let no_image = rows.len() - on_sheet.len();
    {
        let left_off = match no_image {
            0 => String::new(),
            1 => "; 1 photo has no image and is left off".into(),
            n => format!("; {n} photos have no image and are left off"),
        };
        lock(&s.describe).note(call, no_image == 0, format!("Sheet {call}: made, {} photos{left_off}", on_sheet.len()));
        let mut kept = lock(&s.describe_sheets);
        kept.push_back((call, jpeg.clone()));
        while kept.len() > SHEETS_KEPT {
            kept.pop_front();
        }
    }
    let request = SheetRequest { jpeg, contexts, progress: Some(progress) };
    let reply = match provider.describe_sheet(&request) {
        Ok(reply) => reply,
        Err(failed) => {
            if let Some(record) = &failed.record {
                s.library.record_call(on_sheet[0], record)?;
            }
            return Err(failed.into());
        }
    };
    if !reply.warnings.is_empty() {
        s.log.event(&format!("A sheet of {} photos: {}", on_sheet.len(), reply.warnings.join("; ")), &[]);
    }
    let mut unanswered = 0;
    let mut first = Some(());
    for (&row, answer) in on_sheet.iter().zip(reply.replies) {
        let Some((suggestion, warnings)) = answer else {
            unanswered += 1;
            continue;
        };
        let entry = Entry {
            at: chrono::Utc::now().timestamp_millis(),
            provider: provider.id().to_string(),
            model: provider.model().to_string(),
            prompt: provider.prompt_hash(),
            suggestion,
            warnings,
            cost: 0.0,
            status: Status::Proposed,
        };
        // The call is recorded once, with the first photo it gave a suggestion for.
        match first.take() {
            Some(()) => s.library.put_suggestion(row, &entry, &reply.record)?,
            None => s.library.put_suggestion_only(row, &entry)?,
        }
    }
    if first.is_some() {
        s.library.record_call(on_sheet[0], &reply.record)?;
    }
    Ok(Described { no_image, unanswered, cost: reply.record.cost })
}

/// Stops the run: calls under way end as soon as they can.
#[tauri::command]
pub(crate) async fn stop_describe(app: State<'_, App>) -> Result<(), String> {
    app.session()?.describe_stop.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub(crate) async fn describe_state(app: State<'_, App>) -> Result<DescribeState, String> {
    Ok(lock(&app.session()?.describe).clone())
}

/// What a photo has now of what a suggestion would set: the review shows the two side
/// by side, so nothing the photographer wrote is replaced by mistake.
#[derive(Serialize)]
pub(crate) struct Current {
    caption: Option<String>,
    headline: Option<String>,
    rating: Option<u8>,
    /// Pipe paths.
    keywords: Vec<String>,
    place: Place,
    /// It has a position: a guessed one is never given.
    placed: bool,
}

impl Current {
    fn of(photo: &Photo) -> Current {
        let text = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        Current {
            caption: text(&photo.iptc.description),
            headline: text(&photo.iptc.headline),
            rating: photo.rating,
            keywords: photo.keywords.iter().map(|k| k.to_pipe()).collect(),
            place: photo.place.clone(),
            placed: photo.geo.is_some(),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct SuggestionRow {
    row: u64,
    #[serde(flatten)]
    entry: Entry,
    current: Option<Current>,
}

/// Stored suggestions, newest first: `proposed`, `accepted`, `rejected`, or all; each
/// with what its photo has now (edits queued included).
#[tauri::command]
pub(crate) async fn suggestions(app: State<'_, App>, status: Option<Status>) -> Result<Vec<SuggestionRow>, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        let list = s.library.suggestions(status).map_err(text)?;
        list.into_iter()
            .map(|(row, entry)| {
                let current = s.library.photo(row).map_err(text)?.as_ref().map(Current::of);
                Ok(SuggestionRow { row, entry, current })
            })
            .collect()
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
pub(crate) async fn accept_suggestions(app: State<'_, App>, picks: Vec<Accept>) -> Result<Accepted, String> {
    let s = app.session()?;
    s.touched();
    tauri::async_runtime::spawn_blocking(move || s.library.accept_suggestions(&picks).map_err(|e| format!("{e:#}")))
        .await
        .map_err(text)?
}

/// The suggestions reviewed, newest first, and how many could not be read.
#[derive(Serialize)]
pub(crate) struct Reviews {
    reviews: Vec<c1index::ReviewRow>,
    unreadable: usize,
}

/// The suggestions reviewed, newest first: what lessons are drawn from.
#[tauri::command]
pub(crate) async fn ai_reviews(app: State<'_, App>) -> Result<Reviews, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        let (reviews, unreadable) = s.library.reviews().map_err(text)?;
        Ok(Reviews { reviews, unreadable })
    })
    .await
    .map_err(text)?
}

#[tauri::command]
pub(crate) async fn delete_ai_reviews(app: State<'_, App>, seqs: Vec<u64>) -> Result<(), String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || s.library.delete_reviews(&seqs).map_err(text)).await.map_err(text)?
}

/// How long the lessons may be, in characters: they go into every call.
const MAX_LESSONS_TEXT: usize = 4000;

/// Lessons proposed by Claude from the corrections given (one line each: those made since
/// the lessons in use were saved, and those before) and the count of suggestions accepted
/// without one. Through the API key with API access, or with Gemini access when a key is
/// stored, else Claude Code on a plan: the AI tab says which before asking. The call is
/// recorded, so what it cost counts with the rest. Not kept: the user reads the lessons
/// and saves them in the settings.
#[tauri::command]
pub(crate) async fn refine_lessons(
    app: State<'_, App>,
    reviews: Vec<String>,
    earlier: Vec<String>,
    unchanged: usize,
) -> Result<c1ai::refine::Refined, String> {
    let s = app.session()?;
    tauri::async_runtime::spawn_blocking(move || {
        let settings = load_settings(&s);
        let ask = c1ai::refine::RefineAsk {
            instructions: settings.instructions.clone().unwrap_or_else(|| c1ai::INSTRUCTIONS.to_string()),
            lessons: settings.lessons.clone(),
            reviews,
            earlier,
            unchanged,
        };
        let (system, message, schema) = (c1ai::refine::system(), c1ai::refine::message(&ask), c1ai::refine::schema());
        let options = claude::Options {
            model: settings.model.clone(),
            effort: settings.effort.clone(),
            attempts: 3,
            stop: Default::default(),
        };
        // Gemini describes, but refining is Claude's: the key if there is one, else the plan.
        let key = keys::get(&ANTHROPIC).map_err(text)?;
        let (answer, record) = match (settings.access, key) {
            (Access::Api, None) => return Err(AiError::NoKey.to_string()),
            (Access::Api | Access::Gemini, Some(key)) => {
                let claude = Claude::new(key, options, &Brief::default(), Vocabulary::default()).map_err(text)?;
                claude.ask(&system, &message, &schema).map_err(text)?
            }
            (Access::Plan | Access::Gemini, _) => {
                let program = claude_code::find().ok_or_else(|| {
                    "Refining the prompt asks Claude: add an API key in Settings > AI, or install Claude Code and sign in"
                        .to_string()
                })?;
                if !on_plan(&claude_code::status(&program).map_err(text)?) {
                    return Err("Refining the prompt asks Claude: Claude Code is not signed in with a Claude plan".into());
                }
                ClaudeCode::asking(program, options, &system, schema).map_err(text)?.ask(&message).map_err(text)?
            }
        };
        s.library.record_refine(&record).map_err(text)?;
        c1ai::refine::read(&answer).map_err(text)
    })
    .await
    .map_err(text)?
}

/// Sets suggestions aside (`rejected`), or back to `proposed`.
#[tauri::command]
pub(crate) async fn set_suggestion_status(app: State<'_, App>, rows: Vec<u64>, status: Status) -> Result<(), String> {
    app.session()?.library.set_suggestion_status(&rows, status).map_err(text)
}
