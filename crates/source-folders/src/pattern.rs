//! Path patterns for generated images (and their checks).
//!
//! A pattern is a path with tokens in braces:
//!
//! | Token | Value |
//! |---|---|
//! | `{data}` | the source's own data folder (the provider's `work_dir`) |
//! | `{root}` | the root folder the photo is under |
//! | `{dir}` | the folder the photo is in |
//! | `{relpath}` | the photo's path below its root, with its extension: `2024/x/DSCF0001.RAF` |
//! | `{name}` | the file name: `DSCF0001.RAF` |
//! | `{stem}` | the file name without extension: `DSCF0001` |
//! | `{ext}` | the extension as written, without the dot: `RAF` |
//! | `{hash}` | the photo's id, 16 hex digits; `{hash:2}` its first 2 |
//! | `{tier}` | `thumb` or `preview` |
//! | `{size}` | the tier's long edge in pixels: `450`, `2000` |
//!
//! `/` separates folders on every system.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The image store layouts offered by name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    /// In the app's data folder: `{data}/images/{tier}/{hash:2}/{hash}.jpg`.
    #[default]
    Central,
    /// Under each root, so it travels with an external drive.
    PerRoot,
    /// In a `.previews` folder beside each photo.
    Beside,
    /// A pattern of the user's own.
    Custom,
}

/// A preset's pattern for the thumbnail (`thumb`) or the preview tier. `Custom` has none of
/// its own and gives `Central`'s.
pub fn preset(preset: Preset, thumb: bool) -> &'static str {
    let _ = thumb;
    match preset {
        Preset::Central | Preset::Custom => "{data}/images/{tier}/{hash:2}/{hash}.jpg",
        Preset::PerRoot => "{root}/.c1studio/{tier}/{relpath}.jpg",
        Preset::Beside => "{dir}/.previews/{stem}.{tier}.jpg",
    }
}

/// What the tokens stand for, for one photo and tier.
#[derive(Clone, Debug, Default)]
pub struct Tokens<'a> {
    pub data: &'a str,
    pub root: &'a str,
    /// Below the root, `/`-separated, with the file name.
    pub relpath: &'a str,
    pub hash: &'a str,
    pub tier: &'a str,
    pub size: u32,
}

/// A pattern parsed and checked for unknown tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern(String);

impl Pattern {
    /// Fails on an unknown token, an unclosed brace, or an empty pattern.
    pub fn parse(s: &str) -> Result<Self, String> {
        let _ = s;
        todo!("WP4")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The path for one photo and tier.
    pub fn render(&self, t: &Tokens) -> PathBuf {
        let _ = t;
        todo!("WP4")
    }

    /// Why the pattern cannot give every photo its own file, judged from the pattern alone:
    /// it needs `{hash}`, or `{relpath}` with `{root}`, or `{stem}`/`{name}` with `{dir}`,
    /// and `{tier}` (or `{size}`) so the two tiers differ. `None` when it can.
    pub fn static_problem(&self) -> Option<String> {
        todo!("WP4")
    }
}

/// Paths that more than one photo maps to, with the photos (by hash) that share each.
pub fn collisions<'a>(pattern: &Pattern, items: impl IntoIterator<Item = Tokens<'a>>) -> Vec<(PathBuf, Vec<String>)> {
    let _ = (pattern, items.into_iter());
    todo!("WP4")
}
