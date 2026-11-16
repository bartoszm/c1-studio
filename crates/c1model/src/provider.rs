//! Providers: the kinds of photo source the app can open.
//!
//! A [`PhotoSource`] is one open source; a [`Provider`] knows how to find, recognise and
//! open sources of its kind. The app holds only a [`SourceSpec`] (which provider, and
//! where) and the providers it was built with, so a new kind of source (a folder of
//! files, another catalog format, a server) is a new provider, not a change to the app.

use std::env;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::source::{PhotoSource, Result};

/// What to open: which provider, and where. The location is the provider's own: a file
/// path for a Capture One catalog; a folder, URL or account for providers to come.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceSpec {
    /// [`Provider::id`].
    pub provider: String,
    pub location: String,
}

/// How the user points at a source of this kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Pick {
    /// A file of one of these types (extensions without the dot).
    File {
        label: &'static str,
        extensions: &'static [&'static str],
    },
    Folder,
}

/// A kind of photo source.
pub trait Provider: Send + Sync {
    /// Stable, stored in recent lists and settings: `capture-one`.
    fn id(&self) -> &'static str;

    /// For menus: "Capture One catalog".
    fn name(&self) -> &'static str;

    fn pick(&self) -> Pick;

    /// The location this provider would open for what the user gave (a file, or a
    /// folder holding one), or `None` if it is not one of its sources. Used to choose a
    /// provider for a path given on the command line.
    fn recognise(&self, given: &str) -> Option<String>;

    /// A short, stable, file-name-safe key for the app's data about this source (its
    /// library, settings and backups live under [`data_root`]`/<key>`).
    fn data_key(&self, location: &str) -> String;

    /// Opens a source. `work_dir` is the provider's own and is kept between runs: for its
    /// temporary files, its backups and any store it keeps about the source.
    fn open(&self, location: &str, work_dir: &Path) -> Result<Box<dyn PhotoSource>>;
}

/// Where the app keeps its data about each source: `$C1_DATA_DIR`, else
/// `%LOCALAPPDATA%` (Windows) or `$XDG_DATA_HOME` (`~/.local/share`), then `c1studio`.
pub fn data_root() -> PathBuf {
    env::var_os("C1_DATA_DIR")
        .or_else(|| env::var_os("LOCALAPPDATA"))
        .or_else(|| xdg("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local").join("share"))
        .join("c1studio")
}

/// Where the app keeps its own settings, such as the recent sources:
/// `%APPDATA%\c1studio` (Windows) or `$XDG_CONFIG_HOME/c1studio` (`~/.config`).
pub fn config_root() -> PathBuf {
    env::var_os("APPDATA")
        .or_else(|| xdg("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("c1studio")
}

/// An XDG base directory, honoured off Windows when it is set to an absolute path.
fn xdg(var: &str) -> Option<std::ffi::OsString> {
    if cfg!(windows) {
        return None;
    }
    env::var_os(var).filter(|v| Path::new(v).is_absolute())
}

fn home() -> PathBuf {
    env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}
