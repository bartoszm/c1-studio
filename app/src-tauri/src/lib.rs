//! The desktop app: the library behind Tauri commands, and pixels behind the `c1img:`
//! protocol (`c1img://localhost/thumb/<row>`, `c1img://localhost/preview/<row>`).

use std::sync::{Arc, Mutex};

use c1index::{Filter, Library, QueryOptions, QueryResult, SyncReport};
use c1model::{Collection, ImageTier, Photo, PhotoSource};
use serde::Serialize;
use source_c1::{CaptureOne, data_dir, find_catalog};
use tauri::http::{Response, StatusCode, header};
use tauri::{AppHandle, Emitter, Manager, State};

struct Session {
    source: CaptureOne,
    library: Library,
    /// One sync at a time: the search index has a single writer.
    syncing: Mutex<()>,
}

/// The open session, or why there is none (no catalog found, library unreadable).
struct App(Result<Arc<Session>, String>);

impl App {
    fn session(&self) -> Result<Arc<Session>, String> {
        self.0.clone()
    }
}

fn open() -> anyhow::Result<Session> {
    let catalog = find_catalog(None)?;
    let dir = data_dir(&catalog);
    let source = CaptureOne::open(&catalog, dir.join("work"))?;
    let library = Library::open(dir.join("library"))?;
    Ok(Session { source, library, syncing: Mutex::new(()) })
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
struct Status {
    catalog: String,
    source: String,
    photos: u64,
    writes: String,
}

#[tauri::command]
async fn status(app: State<'_, App>) -> Result<Status, String> {
    let s = app.session()?;
    Ok(Status {
        catalog: s.source.catalog_path().display().to_string(),
        source: s.source.info().name,
        photos: s.library.len(),
        writes: format!("{:?}", s.source.write_status()),
    })
}

#[tauri::command]
async fn query(app: State<'_, App>, filter: Filter, options: QueryOptions) -> Result<QueryResult, String> {
    app.session()?.library.query(&filter, &options).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn photo(app: State<'_, App>, row: u64) -> Result<Option<Photo>, String> {
    app.session()?.library.photo(row).map_err(text)
}

#[tauri::command]
async fn collections(app: State<'_, App>) -> Result<Vec<Collection>, String> {
    app.session()?.library.collections().map_err(text)
}

#[tauri::command]
async fn sync(app: AppHandle) -> Result<SyncReport, String> {
    tauri::async_runtime::spawn_blocking(move || run_sync(&app)).await.map_err(text)?
}

/// Pulls Capture One's changes in, and tells the window when anything moved.
fn run_sync(app: &AppHandle) -> Result<SyncReport, String> {
    let s = app.state::<App>().session()?;
    let _one = s.syncing.lock().unwrap_or_else(|e| e.into_inner());
    let report = s.library.sync(&s.source).map_err(|e| format!("{e:#}"))?;
    if report.full || report.added + report.updated + report.deleted > 0 {
        let _ = app.emit("library-changed", &report);
    }
    Ok(report)
}

/// `/thumb/<row>` serves Capture One's own thumbnail as stored: a JPEG with every edit.
/// `/preview/<row>` renders the large preview (~250 ms) and sends it as JPEG.
fn serve(app: &AppHandle, path: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let mut parts = path.trim_start_matches('/').split('/');
    let tier = match parts.next() {
        Some("thumb") => ImageTier::Thumb,
        Some("preview") => ImageTier::Preview,
        _ => return Ok(None),
    };
    let Some(row) = parts.next().and_then(|r| r.parse::<u64>().ok()) else { return Ok(None) };
    let s = app.state::<App>().session().map_err(anyhow::Error::msg)?;
    let Some(id) = s.library.photo_id(row) else { return Ok(None) };
    let Some(source) = s.source.image(&id, tier)? else { return Ok(None) };
    if tier == ImageTier::Thumb {
        return Ok(Some(std::fs::read(&source.path)?));
    }
    let pixels = c1image::load(&source)?;
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&pixels)?;
    Ok(Some(out))
}

fn image_response(app: &AppHandle, path: &str) -> Response<Vec<u8>> {
    let (status, mime, body) = match serve(app, path) {
        Ok(Some(bytes)) => (StatusCode::OK, "image/jpeg", bytes),
        Ok(None) => (StatusCode::NOT_FOUND, "text/plain", b"no such image".to_vec()),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, "text/plain", format!("{e:#}").into_bytes()),
    };
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        // Rows are stable and pixels change only when Capture One re-renders.
        .header(header::CACHE_CONTROL, "max-age=600")
        .body(body)
        .expect("a static response always builds")
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let session = open().map(Arc::new).map_err(|e| format!("{e:#}"));
            let ready = session.is_ok();
            app.manage(App(session));
            if ready {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(e) = run_sync(&handle) {
                        let _ = handle.emit("sync-failed", e);
                    }
                });
            }
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("c1img", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_owned();
            tauri::async_runtime::spawn_blocking(move || responder.respond(image_response(&app, &path)));
        })
        .invoke_handler(tauri::generate_handler![status, query, photo, collections, sync])
        .run(tauri::generate_context!())
        .expect("C1 Studio failed to start");
}
