//! The definition file (`*.c1folders`): which folders make up the source and how it keeps
//! its images. Its path is the source's location; its `id` keys everything the app keeps
//! about the source, so the file can be moved or renamed without losing that.

use std::io;
use std::path::{Path, PathBuf};

use c1model::SourceId;
use c1model::provider::config_root;
use serde::{Deserialize, Serialize};

use crate::pattern::{self, Preset};

/// The definition file's extension, without the dot.
pub const EXTENSION: &str = "c1folders";

/// Excluded in every source: the folders the image store presets write into.
pub const ALWAYS_EXCLUDED: [&str; 2] = [".c1studio", ".previews"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    #[serde(default = "first_version")]
    pub version: u32,
    /// 32 hex digits, made once. Keys the app's data about the source.
    pub id: String,
    /// For menus and the window title: "Family archive".
    pub name: String,
    pub roots: Vec<RootDef>,
    /// Globs matched against a path relative to its root, with `/` separators:
    /// `**/tmp/**`, `*.tmp`. A folder that matches is not entered.
    #[serde(default)]
    pub excludes: Vec<String>,
    #[serde(default)]
    pub images: ImageStore,
    #[serde(default)]
    pub sidecar: SidecarNaming,
    /// Export edits as XMP: a sidecar beside each RAW, and (with `modify_originals`)
    /// embedded in JPEG, TIFF and DNG files.
    #[serde(default)]
    pub write_xmp: bool,
    /// Allows writing XMP into original JPEG, TIFF and DNG files. Off: originals are never
    /// changed.
    #[serde(default)]
    pub modify_originals: bool,
}

fn first_version() -> u32 {
    1
}

/// One folder scanned recursively.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootDef {
    /// 8 hex digits, made when the root is added; stays when its path changes (a drive
    /// that comes back under another letter).
    pub id: String,
    /// Absolute, in this machine's native form.
    pub path: String,
    /// Shown instead of the folder's own name.
    #[serde(default)]
    pub label: Option<String>,
}

impl RootDef {
    /// The name shown for the root: its label, else its folder's name, else its path.
    pub fn display_name(&self) -> String {
        if let Some(label) = self.label.as_deref().filter(|l| !l.trim().is_empty()) {
            return label.to_owned();
        }
        Path::new(&self.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.clone())
    }
}

/// Where generated thumbnails and previews go.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageStore {
    pub preset: Preset,
    /// Path patterns (see [`crate::pattern`]); the preset's own unless `preset` is
    /// [`Preset::Custom`].
    pub thumb: String,
    pub preview: String,
    /// Size limit of the [`Preset::Central`] store, in megabytes; least recently used
    /// images go first. The other stores belong to the user and are never trimmed.
    #[serde(default = "default_limit")]
    pub central_limit_mb: Option<u64>,
}

fn default_limit() -> Option<u64> {
    Some(4096)
}

impl ImageStore {
    pub fn preset(preset: Preset) -> Self {
        let (thumb, preview) = match preset {
            Preset::Custom => (pattern::preset(Preset::Central, true), pattern::preset(Preset::Central, false)),
            p => (pattern::preset(p, true), pattern::preset(p, false)),
        };
        Self { preset, thumb: thumb.to_owned(), preview: preview.to_owned(), central_limit_mb: default_limit() }
    }

    /// The size limit in bytes, for the one store that has one.
    pub fn limit_bytes(&self) -> Option<u64> {
        if self.preset == Preset::Central { self.central_limit_mb.map(|mb| mb * 1024 * 1024) } else { None }
    }
}

impl Default for ImageStore {
    fn default() -> Self {
        Self::preset(Preset::Central)
    }
}

/// How an XMP sidecar is named after its photo, `DSCF0001.RAF`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidecarNaming {
    /// `DSCF0001.xmp`: Lightroom, Capture One.
    #[default]
    Stem,
    /// `DSCF0001.RAF.xmp`: darktable.
    Name,
}

impl SidecarNaming {
    /// The sidecar for `photo` this naming gives.
    pub fn sidecar(self, photo: &Path) -> PathBuf {
        match self {
            Self::Stem => photo.with_extension("xmp"),
            Self::Name => {
                let mut name = photo.file_name().unwrap_or_default().to_os_string();
                name.push(".xmp");
                photo.with_file_name(name)
            }
        }
    }
}

impl Definition {
    pub fn new(name: &str, roots: &[PathBuf]) -> Self {
        let mut def = Self {
            version: first_version(),
            id: uuid::Uuid::new_v4().simple().to_string(),
            name: name.to_owned(),
            roots: Vec::new(),
            excludes: Vec::new(),
            images: ImageStore::default(),
            sidecar: SidecarNaming::default(),
            write_xmp: false,
            modify_originals: false,
        };
        for root in roots {
            def.add_root(root);
        }
        def
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let def: Self = serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", path.display())))?;
        if def.id.is_empty() || !def.id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{}: id is not hex", path.display())));
        }
        Ok(def)
    }

    /// Writes the file whole, through a temporary file, so a crash never leaves half of it.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }

    /// Adds a root (made absolute) unless it is already there; returns its id.
    pub fn add_root(&mut self, path: &Path) -> String {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned()).to_string_lossy().into_owned();
        if let Some(root) = self.roots.iter().find(|r| r.path == path) {
            return root.id.clone();
        }
        let id = loop {
            let id = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
            if !self.roots.iter().any(|r| r.id == id) {
                break id;
            }
        };
        self.roots.push(RootDef { id: id.clone(), path, label: None });
        id
    }

    pub fn root(&self, id: &str) -> Option<&RootDef> {
        self.roots.iter().find(|r| r.id == id)
    }

    /// `folders:` and the first 8 hex digits of the id.
    pub fn source_id(&self) -> SourceId {
        SourceId(format!("folders:{}", &self.id[..self.id.len().min(8)]))
    }

    /// [`c1model::Provider::data_key`]: `folders-` and the short id. Never the name: the
    /// source can be renamed without losing its store, edits and library.
    pub fn data_key(&self) -> String {
        format!("folders-{}", &self.id[..self.id.len().min(8)])
    }

    /// A file name for the definition: the name made file-safe, then the short id.
    pub fn file_name(&self) -> String {
        format!("{}-{}.{EXTENSION}", slug(&self.name), &self.id[..self.id.len().min(8)])
    }

    /// Where the app keeps definitions it makes: `<config>/sources`.
    pub fn default_dir() -> PathBuf {
        config_root().join("sources")
    }

    /// A new definition's file in [`Definition::default_dir`].
    pub fn default_path(&self) -> PathBuf {
        Self::default_dir().join(self.file_name())
    }

    /// Every exclude, the always-excluded store folders first.
    pub fn all_excludes(&self) -> Vec<String> {
        ALWAYS_EXCLUDED.iter().map(|d| format!("**/{d}")).chain(self.excludes.iter().cloned()).collect()
    }
}

/// Lower-case letters and digits, anything else one `-`; never empty.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').chars().take(40).collect::<String>();
    if out.is_empty() { "folders".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_keys_survive_a_move() {
        let dir = tempfile::tempdir().unwrap();
        let mut def = Definition::new("Family Archive!", &[dir.path().join("a"), dir.path().join("b")]);
        assert_eq!(def.roots.len(), 2);
        assert_eq!(def.add_root(&dir.path().join("a")), def.roots[0].id, "a root is added once");
        let at = dir.path().join("x").join("one.c1folders");
        def.save(&at).unwrap();
        let back = Definition::load(&at).unwrap();
        assert_eq!(back, def);
        let moved = dir.path().join("two.c1folders");
        std::fs::rename(&at, &moved).unwrap();
        assert_eq!(Definition::load(&moved).unwrap().data_key(), def.data_key());
        assert!(def.file_name().starts_with("family-archive-"));
        let mut renamed = def.clone();
        renamed.name = "Something else".into();
        assert_eq!(renamed.data_key(), def.data_key(), "renaming keeps the data");
        assert_eq!(def.source_id().0.len(), "folders:".len() + 8);
    }

    #[test]
    fn old_files_get_defaults() {
        let def: Definition =
            serde_json::from_str(r#"{"id":"0123456789abcdef0123456789abcdef","name":"x","roots":[]}"#).unwrap();
        assert_eq!(def.images, ImageStore::default());
        assert!(!def.write_xmp && !def.modify_originals);
        assert_eq!(def.sidecar, SidecarNaming::Stem);
    }

    #[test]
    fn sidecars_are_named_both_ways() {
        let photo = Path::new("/p/DSCF0001.RAF");
        assert_eq!(SidecarNaming::Stem.sidecar(photo), Path::new("/p/DSCF0001.xmp"));
        assert_eq!(SidecarNaming::Name.sidecar(photo), Path::new("/p/DSCF0001.RAF.xmp"));
    }

    #[test]
    fn slugs_are_file_safe() {
        assert_eq!(slug("  Ździebko / 2024 "), "dziebko-2024");
        assert_eq!(slug("///"), "folders");
        assert_eq!(slug("Trips 2024"), "trips-2024");
    }
}
