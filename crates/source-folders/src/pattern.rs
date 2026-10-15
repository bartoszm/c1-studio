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

use std::collections::BTreeMap;
use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::store::join_rel;

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
    /// The root folder's path, in this machine's native form.
    pub root: &'a str,
    /// Below the root, `/`-separated, with the file name.
    pub relpath: &'a str,
    pub hash: &'a str,
    pub tier: &'a str,
    pub size: u32,
}

/// The longest `{hash:N}`: the length of a photo id.
const HASH_LEN: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Data,
    Root,
    Dir,
    Relpath,
    Name,
    Stem,
    Ext,
    /// The first `n` digits of the photo id.
    Hash(usize),
    Tier,
    Size,
}

impl Token {
    fn parse(text: &str) -> Result<Self, String> {
        let (name, arg) = match text.split_once(':') {
            Some((name, arg)) => (name, Some(arg)),
            None => (text, None),
        };
        if name == "hash" {
            return match arg {
                None => Ok(Self::Hash(HASH_LEN)),
                Some(n) => match n.parse::<usize>() {
                    Ok(n) if (1..=HASH_LEN).contains(&n) => Ok(Self::Hash(n)),
                    _ => Err(format!("{{{text}}}: the number of digits is from 1 to {HASH_LEN}")),
                },
            };
        }
        let token = match name {
            "data" => Self::Data,
            "root" => Self::Root,
            "dir" => Self::Dir,
            "relpath" => Self::Relpath,
            "name" => Self::Name,
            "stem" => Self::Stem,
            "ext" => Self::Ext,
            "tier" => Self::Tier,
            "size" => Self::Size,
            _ => {
                return Err(format!(
                    "{{{text}}} is not a token; the tokens are {{data}} {{root}} {{dir}} {{relpath}} {{name}} \
                     {{stem}} {{ext}} {{hash}} {{hash:N}} {{tier}} {{size}}"
                ));
            }
        };
        match arg {
            None => Ok(token),
            Some(_) => Err(format!("{{{text}}}: only {{hash}} takes a number")),
        }
    }

    /// Stands for a path (several folders), not a single name.
    fn is_path(self) -> bool {
        matches!(self, Self::Data | Self::Root | Self::Dir | Self::Relpath)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Text(String),
    Token(Token),
}

/// The file name's pieces: `DSCF0001.RAF` is `DSCF0001` and `RAF`; `.hidden` has no extension.
fn split_name(name: &str) -> (&str, &str) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, ext),
        _ => (name, ""),
    }
}

fn push_rel(path: &mut PathBuf, rel: &str) {
    for component in rel.split('/').filter(|c| !c.is_empty()) {
        path.push(component);
    }
}

/// The values the tokens of one photo take.
struct Values<'a> {
    t: &'a Tokens<'a>,
    reldir: &'a str,
    name: &'a str,
    stem: &'a str,
    ext: &'a str,
}

impl<'a> Values<'a> {
    fn new(t: &'a Tokens<'a>) -> Self {
        let (reldir, name) = t.relpath.rsplit_once('/').unwrap_or(("", t.relpath));
        let (stem, ext) = split_name(name);
        Self { t, reldir, name, stem, ext }
    }

    /// The token as text, for use inside a name. Paths come out with native separators.
    fn text(&self, token: Token) -> String {
        match token {
            Token::Data => self.t.data.to_owned(),
            Token::Root => self.t.root.to_owned(),
            Token::Dir => join_rel(Path::new(self.t.root), self.reldir).to_string_lossy().into_owned(),
            Token::Relpath => self.t.relpath.replace('/', MAIN_SEPARATOR_STR),
            Token::Name => self.name.to_owned(),
            Token::Stem => self.stem.to_owned(),
            Token::Ext => self.ext.to_owned(),
            Token::Hash(n) => self.t.hash.chars().take(n).collect(),
            Token::Tier => self.t.tier.to_owned(),
            Token::Size => self.t.size.to_string(),
        }
    }

    /// A path token that fills a whole folder position, pushed one component at a time.
    fn push_path(&self, path: &mut PathBuf, token: Token) {
        match token {
            Token::Data => path.push(self.t.data),
            Token::Root => path.push(self.t.root),
            Token::Dir => {
                path.push(self.t.root);
                push_rel(path, self.reldir);
            }
            _ => push_rel(path, self.t.relpath),
        }
    }
}

/// A pattern parsed and checked for unknown tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    text: String,
    /// The pattern cut at its `/`: each folder or file name in pieces.
    components: Vec<Vec<Part>>,
}

impl Pattern {
    /// Fails on an unknown token, a `{hash:N}` whose N is not 1 to 16, an unclosed or
    /// stray brace, an empty pattern, or a pattern that does not end in `.jpg` or `.jpeg`
    /// (the images are JPEGs).
    pub fn parse(s: &str) -> Result<Self, String> {
        if s.trim().is_empty() {
            return Err("the pattern is empty".into());
        }
        let mut components = vec![Vec::new()];
        let mut text = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            match c {
                '{' => {
                    let mut name = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some('{') | None => return Err("a { in the pattern is not closed by a }".into()),
                            Some(c) => name.push(c),
                        }
                    }
                    flush(&mut text, components.last_mut().unwrap());
                    components.last_mut().unwrap().push(Part::Token(Token::parse(&name)?));
                }
                '}' => return Err("a } in the pattern has no { before it".into()),
                '/' => {
                    flush(&mut text, components.last_mut().unwrap());
                    components.push(Vec::new());
                }
                c => text.push(c),
            }
        }
        flush(&mut text, components.last_mut().unwrap());
        let lower = s.to_ascii_lowercase();
        if !(lower.ends_with(".jpg") || lower.ends_with(".jpeg")) {
            return Err("the pattern must end in .jpg, because the images are JPEG files".into());
        }
        Ok(Self { text: s.to_owned(), components })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The path for one photo and tier. The pattern's `/` become the system's separator,
    /// and the values that are paths (`{root}`, `{dir}`, `{relpath}`, `{data}`) keep their
    /// own, in native form.
    pub fn render(&self, t: &Tokens) -> PathBuf {
        let values = Values::new(t);
        let mut path = PathBuf::new();
        for (i, parts) in self.components.iter().enumerate() {
            match parts.as_slice() {
                [] if i == 0 && self.components.len() > 1 => path.push(MAIN_SEPARATOR_STR),
                [] => {}
                [Part::Token(token)] if token.is_path() => values.push_path(&mut path, *token),
                parts => {
                    let name: String = parts
                        .iter()
                        .map(|p| match p {
                            Part::Text(text) => text.clone(),
                            Part::Token(token) => values.text(*token),
                        })
                        .collect();
                    path.push(name);
                }
            }
        }
        path
    }

    fn has(&self, wanted: impl Fn(Token) -> bool) -> bool {
        self.components.iter().flatten().any(|p| matches!(p, Part::Token(t) if wanted(*t)))
    }

    /// Why the pattern cannot give every photo its own file, judged from the pattern alone:
    /// it needs `{hash}`, or `{relpath}` with `{root}`, or `{stem}`/`{name}` with `{dir}`,
    /// and `{tier}` (or `{size}`) so the two tiers differ. `None` when it can.
    pub fn static_problem(&self) -> Option<String> {
        let hash = self.has(|t| t == Token::Hash(HASH_LEN));
        let relpath = self.has(|t| t == Token::Relpath);
        let root = self.has(|t| t == Token::Root);
        let dir = self.has(|t| t == Token::Dir);
        let file = self.has(|t| matches!(t, Token::Name | Token::Stem));
        if !(hash || (relpath && (root || dir)) || (dir && file)) {
            return Some(
                "Photos would share files: the pattern needs {hash}, or {root} with {relpath}, or {dir} with {name}"
                    .into(),
            );
        }
        if !self.has(|t| matches!(t, Token::Tier | Token::Size)) {
            return Some("The thumbnail and the preview would share a file: the pattern needs {tier} or {size}".into());
        }
        None
    }
}

fn flush(text: &mut String, parts: &mut Vec<Part>) {
    if !text.is_empty() {
        parts.push(Part::Text(std::mem::take(text)));
    }
}

/// Groups `(path, label)` pairs by path, ignoring case (Windows and macOS file systems do),
/// and keeps the paths with more than one label. Ordered by path.
pub(crate) fn shared_paths(rendered: impl IntoIterator<Item = (PathBuf, String)>) -> Vec<(PathBuf, Vec<String>)> {
    let mut groups: BTreeMap<String, (PathBuf, Vec<String>)> = BTreeMap::new();
    for (path, label) in rendered {
        let key = path.to_string_lossy().to_lowercase();
        groups.entry(key).or_insert_with(|| (path, Vec::new())).1.push(label);
    }
    groups.into_values().filter(|(_, labels)| labels.len() > 1).collect()
}

/// Paths that more than one photo maps to, with the photos (by hash) that share each.
/// Paths that differ only in case count as the same. A photo listed twice is listed twice.
pub fn collisions<'a>(pattern: &Pattern, items: impl IntoIterator<Item = Tokens<'a>>) -> Vec<(PathBuf, Vec<String>)> {
    shared_paths(items.into_iter().map(|t| (pattern.render(&t), t.hash.to_owned())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens<'a>(root: &'a str, relpath: &'a str, hash: &'a str) -> Tokens<'a> {
        Tokens { data: "/data", root, relpath, hash, tier: "thumb", size: 450 }
    }

    fn render(pattern: &str, t: &Tokens) -> PathBuf {
        Pattern::parse(pattern).unwrap().render(t)
    }

    fn path(parts: &[&str]) -> PathBuf {
        parts.iter().collect()
    }

    #[test]
    fn every_token_has_its_value() {
        let t = tokens("/photos/r", "2024/x/DSCF0001.RAF", "0123456789abcdef");
        let cases = [
            ("{data}/a.jpg", path(&["/data", "a.jpg"])),
            ("{root}/a.jpg", path(&["/photos/r", "a.jpg"])),
            ("{dir}/a.jpg", path(&["/photos/r", "2024", "x", "a.jpg"])),
            ("{relpath}.jpg", path(&["2024", "x", "DSCF0001.RAF.jpg"])),
            ("{name}.jpg", path(&["DSCF0001.RAF.jpg"])),
            ("{stem}.jpg", path(&["DSCF0001.jpg"])),
            ("{ext}/a.jpg", path(&["RAF", "a.jpg"])),
            ("{hash}.jpg", path(&["0123456789abcdef.jpg"])),
            ("{hash:2}/{hash:16}.jpg", path(&["01", "0123456789abcdef.jpg"])),
            ("{tier}/{size}.jpg", path(&["thumb", "450.jpg"])),
            ("x/{stem}-{ext}-{tier}-{size}.jpg", path(&["x", "DSCF0001-RAF-thumb-450.jpg"])),
            ("fixed.jpg", path(&["fixed.jpg"])),
        ];
        for (pattern, expected) in cases {
            assert_eq!(render(pattern, &t), expected, "{pattern}");
        }
    }

    #[test]
    fn a_photo_in_the_root_has_no_folder() {
        let t = tokens("/photos/r", "DSCF0001.RAF", "0123456789abcdef");
        assert_eq!(render("{dir}/{stem}.jpg", &t), path(&["/photos/r", "DSCF0001.jpg"]));
        assert_eq!(render("{root}/{relpath}.jpg", &t), path(&["/photos/r", "DSCF0001.RAF.jpg"]));
    }

    #[test]
    fn a_name_without_extension_or_with_a_leading_dot() {
        let t = tokens("/r", "x/README", "0123456789abcdef");
        assert_eq!(render("{stem}|{ext}.jpg", &t), path(&["README|.jpg"]));
        let t = tokens("/r", ".hidden", "0123456789abcdef");
        assert_eq!(render("{stem}|{ext}.jpg", &t), path(&[".hidden|.jpg"]));
        let t = tokens("/r", "a.b.c.jpg", "0123456789abcdef");
        assert_eq!(render("{stem}|{ext}.jpg", &t), path(&["a.b.c|jpg.jpg"]));
    }

    #[test]
    fn the_presets_render() {
        let hash = "0123456789abcdef";
        let t = tokens("/photos/r", "2024/x/DSCF0001.RAF", hash);
        let central = preset(Preset::Central, true);
        assert_eq!(render(central, &t), path(&["/data", "images", "thumb", "01", "0123456789abcdef.jpg"]));
        assert_eq!(preset(Preset::Custom, false), central);
        let per_root = preset(Preset::PerRoot, false);
        let p = Tokens { tier: "preview", size: 2000, ..t.clone() };
        assert_eq!(render(per_root, &p), path(&["/photos/r", ".c1studio", "preview", "2024", "x", "DSCF0001.RAF.jpg"]));
        let beside = preset(Preset::Beside, true);
        assert_eq!(render(beside, &t), path(&["/photos/r", "2024", "x", ".previews", "DSCF0001.thumb.jpg"]));
        for p in [Preset::Central, Preset::PerRoot, Preset::Beside, Preset::Custom] {
            let pattern = Pattern::parse(preset(p, true)).unwrap();
            assert_eq!(pattern.static_problem(), None, "{p:?}");
        }
    }

    #[test]
    fn windows_style_roots_round_trip() {
        let t = Tokens {
            data: r"C:\Users\me\AppData\c1",
            ..tokens(r"D:\Photos", "2024/x/DSCF0001.RAF", "0123456789abcdef")
        };
        let per_root = render(preset(Preset::PerRoot, true), &t);
        let expected: PathBuf = [r"D:\Photos", ".c1studio", "thumb", "2024", "x", "DSCF0001.RAF.jpg"].iter().collect();
        assert_eq!(per_root, expected);
        assert!(per_root.to_string_lossy().starts_with(r"D:\Photos"));
        let beside = render(preset(Preset::Beside, true), &t);
        let expected: PathBuf = [r"D:\Photos", "2024", "x", ".previews", "DSCF0001.thumb.jpg"].iter().collect();
        assert_eq!(beside, expected);
        let central = render(preset(Preset::Central, true), &t);
        assert!(central.to_string_lossy().starts_with(r"C:\Users\me\AppData\c1"));
        // A path token inside a name gets the system's separators.
        let inside = render("x/{dir}-{relpath}.jpg", &t);
        let expected = format!("D:\\Photos{s}2024{s}x-2024{s}x{s}DSCF0001.RAF.jpg", s = MAIN_SEPARATOR_STR);
        assert_eq!(inside, path(&["x", &expected]));
    }

    #[test]
    fn hash_digits_are_one_to_sixteen() {
        for ok in ["{hash:1}", "{hash:2}", "{hash:16}"] {
            assert!(Pattern::parse(&format!("{ok}.jpg")).is_ok(), "{ok}");
        }
        for bad in ["{hash:0}", "{hash:17}", "{hash:}", "{hash:x}", "{hash:-1}"] {
            let err = Pattern::parse(&format!("{bad}.jpg")).unwrap_err();
            assert!(err.contains("1 to 16"), "{bad}: {err}");
        }
    }

    #[test]
    fn bad_patterns_are_refused() {
        for (pattern, wanted) in [
            ("", "empty"),
            ("   ", "empty"),
            ("{nope}/{hash}.jpg", "{nope} is not a token"),
            ("{stem:3}.jpg", "only {hash}"),
            ("{hash.jpg", "not closed"),
            ("{hash{tier}}.jpg", "not closed"),
            ("{hash}}.jpg", "no { before"),
            ("}{hash}.jpg", "no { before"),
            ("{hash}.png", ".jpg"),
            ("{hash}", ".jpg"),
            ("{hash}.jpg.tmp", ".jpg"),
        ] {
            let err = Pattern::parse(pattern).unwrap_err();
            assert!(err.contains(wanted), "{pattern:?}: {err}");
        }
        for ok in ["{hash}.jpg", "{hash}.JPG", "{hash}.Jpeg", "a/{hash}.jpeg"] {
            assert!(Pattern::parse(ok).is_ok(), "{ok}");
        }
        assert_eq!(Pattern::parse("{hash}.jpg").unwrap().as_str(), "{hash}.jpg");
    }

    #[test]
    fn static_problems() {
        let problem = |p: &str| Pattern::parse(p).unwrap().static_problem();
        // Fine.
        for ok in [
            "{data}/{tier}/{hash}.jpg",
            "{data}/{size}/{hash}.jpg",
            "{root}/{tier}/{relpath}.jpg",
            "{dir}/{tier}/{relpath}.jpg",
            "{dir}/{name}.{tier}.jpg",
            "{dir}/{stem}.{size}.jpg",
            "{data}/{tier}/{hash:2}/{hash}.jpg",
        ] {
            assert_eq!(problem(ok), None, "{ok}");
        }
        // Photos would share a file.
        for bad in [
            "{data}/{tier}/{hash:2}.jpg",
            "{data}/{tier}/{hash:15}.jpg",
            "{data}/{tier}/{relpath}.jpg",
            "{root}/{tier}/{stem}.jpg",
            "{dir}/{tier}/{ext}.jpg",
            "{data}/{tier}/fixed.jpg",
        ] {
            let p = problem(bad).unwrap_or_default();
            assert!(p.contains("share files"), "{bad}: {p}");
        }
        // The two tiers would share one.
        for bad in ["{data}/{hash}.jpg", "{dir}/{stem}.jpg", "{root}/{relpath}.jpg"] {
            let p = problem(bad).unwrap_or_default();
            assert!(p.contains("{tier} or {size}"), "{bad}: {p}");
        }
    }

    #[test]
    fn collisions_ignore_case_and_name_both_photos() {
        let pattern = Pattern::parse("{dir}/{stem}.{tier}.jpg").unwrap();
        let items = vec![
            tokens("/r", "a/DSCF1.RAF", "aaaaaaaaaaaaaaaa"),
            tokens("/r", "a/DSCF1.JPG", "bbbbbbbbbbbbbbbb"),
            tokens("/r", "a/dscf1.dng", "cccccccccccccccc"),
            tokens("/r", "b/DSCF1.RAF", "dddddddddddddddd"),
            tokens("/r", "a/DSCF2.RAF", "eeeeeeeeeeeeeeee"),
        ];
        let found = collisions(&pattern, items.clone());
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, path(&["/r", "a", "DSCF1.thumb.jpg"]));
        assert_eq!(found[0].1, ["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb", "cccccccccccccccc"]);

        let central = Pattern::parse(preset(Preset::Central, true)).unwrap();
        assert!(collisions(&central, items).is_empty());
        assert!(collisions(&central, []).is_empty());
    }

    #[test]
    fn a_short_hash_collides_when_the_prefix_is_shared() {
        let pattern = Pattern::parse("{data}/{hash:2}.jpg").unwrap();
        let items = vec![
            tokens("/r", "a.jpg", "0123456789abcdef"),
            tokens("/r", "b.jpg", "01ffffffffffffff"),
            tokens("/r", "c.jpg", "02ffffffffffffff"),
        ];
        let found = collisions(&pattern, items);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1.len(), 2);
    }
}
