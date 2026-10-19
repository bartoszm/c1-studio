//! The commands that manage a photo folders source: its definition and image store.
//! The only place the app looks past the source contract at a [`FolderSource`].

use std::path::{Path, PathBuf};

use c1model::PhotoSource;
use serde::Serialize;
use source_folders::pattern::Pattern;
use source_folders::{Definition, FolderSource, ImageStore, Migrate};
use tauri::{AppHandle, Manager};

use crate::{App, sync_in_background, text};

/// The open source as a folder source, when it is one.
fn folder_source(source: &dyn PhotoSource) -> Option<&FolderSource> {
    source.as_any().and_then(|a| a.downcast_ref::<FolderSource>())
}

/// A folder source's definition, with where it is kept.
#[derive(Serialize)]
pub(crate) struct FoldersView {
    /// The definition file; none for one not saved yet.
    location: Option<String>,
    definition: Definition,
    /// This is the source that is open.
    is_open: bool,
}

/// Whether `location` is the definition file of the source open now.
fn is_open_definition(app: &AppHandle, location: &Path) -> bool {
    let state = app.state::<App>();
    let current = state.0.read().unwrap();
    let Some(spec) =
        current.spec.as_ref().filter(|s| s.provider == source_folders::PROVIDER_ID && current.session.is_ok())
    else {
        return false;
    };
    let open = Path::new(&spec.location);
    open == location || matches!((open.canonicalize(), location.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// A definition for a new folder source, not saved yet.
#[tauri::command]
pub(crate) async fn folders_new(name: String, roots: Vec<String>) -> Result<FoldersView, String> {
    let roots: Vec<PathBuf> = roots.into_iter().map(PathBuf::from).collect();
    Ok(FoldersView { location: None, definition: Definition::new(&name, &roots), is_open: false })
}

/// The definition in a `.c1folders` file.
#[tauri::command]
pub(crate) async fn folders_load(app: AppHandle, location: String) -> Result<FoldersView, String> {
    let path = PathBuf::from(&location);
    let definition = Definition::load(&path).map_err(text)?;
    let is_open = is_open_definition(&app, &path);
    Ok(FoldersView { location: Some(location), definition, is_open })
}

/// Writes a definition, to `location` or (a new one) to its place in the app's folder, and
/// returns where. If that is the source open, it is read again (with `migrate` saying what
/// becomes of the images when their store changed) and the roots are walked; otherwise the
/// window opens it.
#[tauri::command]
pub(crate) async fn folders_save(
    app: AppHandle,
    def: Definition,
    location: Option<String>,
    migrate: Option<Migrate>,
) -> Result<String, String> {
    let path = location.map_or_else(|| def.default_path(), PathBuf::from);
    tauri::async_runtime::spawn_blocking(move || {
        // Not trusting the dialog's own (debounced) check: a store that would write over a
        // photo is refused here, before anything is saved.
        if is_open_definition(&app, &path)
            && let Ok(s) = app.state::<App>().session()
            && let Some(source) = folder_source(s.source.as_ref())
            && let Some(problem) = source.check_image_store(&def.images)
        {
            return Err(format!("Image store: {problem}"));
        }
        def.save(&path).map_err(text)?;
        if is_open_definition(&app, &path) {
            let s = app.state::<App>().session()?;
            let source = folder_source(s.source.as_ref()).ok_or("the open source is not a folder source")?;
            if let Some(report) = source.reload_definition(migrate).map_err(text)? {
                let detail = report.failed.iter().map(|f| format!("failed: {f}")).collect::<Vec<_>>();
                s.log.event(
                    &format!("Image store changed: {} moved, {} removed", report.moved, report.removed),
                    &detail,
                );
            }
            s.log.event("Folders changed", &[]);
            sync_in_background(&app);
        }
        Ok(path.display().to_string())
    })
    .await
    .map_err(text)?
}

/// Why `store` would not do as an image store, or `None`. Checked against the photos when a
/// folder source is open; otherwise only the patterns themselves.
#[tauri::command]
pub(crate) async fn folders_check_store(app: AppHandle, store: ImageStore) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Ok(s) = app.state::<App>().session()
            && let Some(source) = folder_source(s.source.as_ref())
        {
            return Ok(source.check_image_store(&store));
        }
        for (what, pattern) in [("Thumbnails", &store.thumb), ("Previews", &store.preview)] {
            match Pattern::parse(pattern) {
                Ok(parsed) => {
                    if let Some(problem) = parsed.static_problem() {
                        return Ok(Some(format!("{what}: {problem}")));
                    }
                }
                Err(e) => return Ok(Some(format!("{what}: {e}"))),
            }
        }
        Ok(None)
    })
    .await
    .map_err(text)?
}
