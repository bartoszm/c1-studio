//! `folders`: making and changing the definition files (`*.c1folders`) of photo folder
//! sources.

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Subcommand, ValueEnum};
use source_folders::definition::EXTENSION;
use source_folders::{Definition, ImageStore, Preset};

use crate::source::{SourceArgs, definition_file};

#[derive(Subcommand)]
pub enum Action {
    /// Make a definition for one or more photo folders, and save it.
    Init {
        /// Shown in menus and used to name the file.
        #[arg(long)]
        name: String,
        /// A path pattern to leave out, relative to a root, with `/`: `**/tmp/**`, `*.tmp`
        /// (repeatable).
        #[arg(long = "exclude", value_name = "GLOB")]
        excludes: Vec<String>,
        /// Where to save it; defaults to the app's own folder for definitions. A folder
        /// gets a file named after the source.
        #[arg(long, short, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Replace a definition already there (it gets a new id, so the library built for
        /// the old one is not used).
        #[arg(long)]
        force: bool,
        #[arg(required = true, value_name = "ROOT")]
        roots: Vec<PathBuf>,
    },
    /// Print a definition; defaults to the one --source names.
    Show {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Add a folder to a definition.
    AddRoot {
        root: PathBuf,
        /// Shown instead of the folder's own name.
        #[arg(long)]
        label: Option<String>,
        #[command(flatten)]
        file: DefinitionArg,
    },
    /// Take a folder out of a definition (by its path or its id from `show`). Its photos
    /// are kept, detached, and come back if it is added again.
    RemoveRoot {
        root: String,
        #[command(flatten)]
        file: DefinitionArg,
    },
    /// Choose where thumbnails and previews are kept.
    SetStore {
        /// central: the app's data folder; per-root: `.c1studio` under each root;
        /// beside: a `.previews` folder next to each photo.
        #[arg(value_enum)]
        preset: Store,
        /// Size limit of the central store, in megabytes; 0 for no limit.
        #[arg(long, value_name = "MB")]
        limit_mb: Option<u64>,
        #[command(flatten)]
        file: DefinitionArg,
    },
}

/// Which definition the editing commands change.
#[derive(clap::Args)]
pub struct DefinitionArg {
    /// The definition file; defaults to the one --source names.
    #[arg(long = "file", value_name = "FILE")]
    path: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Store {
    Central,
    PerRoot,
    Beside,
}

impl Store {
    fn preset(self) -> Preset {
        match self {
            Self::Central => Preset::Central,
            Self::PerRoot => Preset::PerRoot,
            Self::Beside => Preset::Beside,
        }
    }
}

pub fn run(action: Action, source: &SourceArgs) -> anyhow::Result<()> {
    match action {
        Action::Init { name, excludes, out, force, roots } => init(&name, excludes, out, force, &roots),
        Action::Show { path, json } => {
            let file = definition_file(path.as_deref(), source)?;
            let def = load(&file)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&def)?);
            } else {
                print!("{}", describe(&def, &file));
            }
            Ok(())
        }
        Action::AddRoot { root, label, file } => {
            if !root.is_dir() {
                anyhow::bail!("{} is not a folder", root.display());
            }
            edit(&file, source, |def, file| {
                let id = def.add_root(&root);
                if let Some(label) = label
                    && let Some(r) = def.roots.iter_mut().find(|r| r.id == id)
                {
                    r.label = Some(label);
                }
                println!("{} is root {id} of {}", root.display(), file.display());
                println!("hint: `c1cat --source {} rescan` reads it", file.display());
                Ok(())
            })
        }
        Action::RemoveRoot { root, file } => edit(&file, source, |def, file| {
            let path =
                std::path::absolute(&root).unwrap_or_else(|_| PathBuf::from(&root)).to_string_lossy().into_owned();
            let Some(at) = def.roots.iter().position(|r| r.id == root || r.path == path) else {
                anyhow::bail!("{root:?} is not a root of {}; `folders show` lists them", file.display());
            };
            let gone = def.roots.remove(at);
            println!("removed {} ({}) from {}", gone.path, gone.id, file.display());
            println!("its photos are kept, detached, and come back if it is added again");
            Ok(())
        }),
        Action::SetStore { preset, limit_mb, file } => edit(&file, source, |def, file| {
            let mut store = ImageStore::preset(preset.preset());
            match limit_mb {
                Some(0) => store.central_limit_mb = None,
                Some(mb) => store.central_limit_mb = Some(mb),
                // Keeps the limit the definition had.
                None => store.central_limit_mb = def.images.central_limit_mb,
            }
            def.images = store;
            println!("images of {} go to the {:?} store", file.display(), def.images.preset);
            println!("images already made stay where they are; they are made again in the new place when needed");
            Ok(())
        }),
    }
}

fn load(file: &Path) -> anyhow::Result<Definition> {
    Definition::load(file).with_context(|| format!("reading {}", file.display()))
}

/// Loads the definition `arg` names, lets `change` alter it, and saves it.
fn edit(
    arg: &DefinitionArg,
    source: &SourceArgs,
    change: impl FnOnce(&mut Definition, &Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let file = definition_file(arg.path.as_deref(), source)?;
    let mut def = load(&file)?;
    change(&mut def, &file)?;
    def.save(&file).with_context(|| format!("writing {}", file.display()))
}

fn init(name: &str, excludes: Vec<String>, out: Option<PathBuf>, force: bool, roots: &[PathBuf]) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("--name is empty");
    }
    for root in roots {
        if !root.is_dir() {
            anyhow::bail!("{} is not a folder", root.display());
        }
    }
    let mut def = Definition::new(name.trim(), roots);
    def.excludes = excludes;
    let file = match out {
        Some(dir) if dir.is_dir() => dir.join(format!("{}.{EXTENSION}", def.data_key())),
        Some(file) => file,
        None => def.default_path(),
    };
    if file.exists() && !force {
        anyhow::bail!("{} already exists; pass --force to replace it, or --out for another file", file.display());
    }
    def.save(&file).with_context(|| format!("writing {}", file.display()))?;
    println!("{}", file.display());
    println!("hint: c1cat --source {} sync", quote(&file));
    Ok(())
}

/// A path as it is typed in a shell: in quotes if it holds a space.
fn quote(path: &Path) -> String {
    let text = path.display().to_string();
    if text.contains(' ') { format!("\"{text}\"") } else { text }
}

fn describe(def: &Definition, file: &Path) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "name      {}", def.name);
    let _ = writeln!(out, "file      {}", file.display());
    let _ = writeln!(out, "id        {}", def.id);
    let _ = writeln!(out, "roots     {}", def.roots.len());
    for root in &def.roots {
        let label = root.label.as_deref().map(|l| format!("  ({l})")).unwrap_or_default();
        let _ = writeln!(out, "  {}  {}{label}", root.id, root.path);
    }
    if !def.excludes.is_empty() {
        let _ = writeln!(out, "excludes  {}", def.excludes.join("  "));
    }
    let limit = match (def.images.preset, def.images.limit_bytes()) {
        (Preset::Central, Some(bytes)) => format!(", limit {} MB", bytes / (1024 * 1024)),
        (Preset::Central, None) => ", no limit".to_owned(),
        _ => String::new(),
    };
    let _ = writeln!(out, "images    {:?}{limit}", def.images.preset);
    let _ = writeln!(out, "  thumb   {}", def.images.thumb);
    let _ = writeln!(out, "  preview {}", def.images.preview);
    let _ = writeln!(out, "sidecars  {:?}", def.sidecar);
    let _ = writeln!(out, "xmp       write {}, change originals {}", def.write_xmp, def.modify_originals);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing() -> SourceArgs {
        SourceArgs::default()
    }

    fn definition(dir: &Path) -> PathBuf {
        let file = dir.join("one.c1folders");
        let photos = dir.join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        init("One", vec!["*.tmp".into()], Some(file.clone()), false, &[photos]).unwrap();
        file
    }

    #[test]
    fn init_saves_a_definition_that_loads_and_refuses_to_replace_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = definition(dir.path());
        let def = Definition::load(&file).unwrap();
        assert_eq!((def.name.as_str(), def.roots.len(), def.excludes.clone()), ("One", 1, vec!["*.tmp".to_owned()]));
        let again = init("One", vec![], Some(file.clone()), false, &[dir.path().join("photos")]);
        assert!(again.unwrap_err().to_string().contains("--force"));
        init("One", vec![], Some(file.clone()), true, &[dir.path().join("photos")]).unwrap();
        assert_ne!(Definition::load(&file).unwrap().id, def.id, "a replaced definition is a new source");
        assert!(init("One", vec![], None, false, &[dir.path().join("missing")]).is_err());
        // Into a folder: named after the source.
        let into = dir.path().join("defs");
        std::fs::create_dir_all(&into).unwrap();
        init("Two", vec![], Some(into.clone()), false, &[dir.path().join("photos")]).unwrap();
        assert!(
            std::fs::read_dir(&into).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("two-"))
        );
    }

    #[test]
    fn roots_come_and_go_and_the_store_changes() {
        let dir = tempfile::tempdir().unwrap();
        let file = definition(dir.path());
        let arg = || DefinitionArg { path: Some(file.clone()) };
        let more = dir.path().join("more");
        std::fs::create_dir_all(&more).unwrap();
        run(Action::AddRoot { root: more.clone(), label: Some("More".into()), file: arg() }, &nothing()).unwrap();
        let def = Definition::load(&file).unwrap();
        assert_eq!(def.roots.len(), 2);
        assert_eq!(def.roots[1].label.as_deref(), Some("More"));
        run(Action::RemoveRoot { root: def.roots[1].id.clone(), file: arg() }, &nothing()).unwrap();
        assert_eq!(Definition::load(&file).unwrap().roots.len(), 1);
        let missing = run(Action::RemoveRoot { root: "nope".into(), file: arg() }, &nothing());
        assert!(missing.is_err());
        run(Action::SetStore { preset: Store::Beside, limit_mb: None, file: arg() }, &nothing()).unwrap();
        assert_eq!(Definition::load(&file).unwrap().images.preset, Preset::Beside);
        run(Action::SetStore { preset: Store::Central, limit_mb: Some(0), file: arg() }, &nothing()).unwrap();
        assert_eq!(Definition::load(&file).unwrap().images.limit_bytes(), None);
    }

    #[test]
    fn show_lists_the_roots() {
        let dir = tempfile::tempdir().unwrap();
        let file = definition(dir.path());
        let def = Definition::load(&file).unwrap();
        let text = describe(&def, &file);
        assert!(text.contains("name      One") && text.contains(&def.roots[0].id) && text.contains("*.tmp"), "{text}");
        run(Action::Show { path: Some(file), json: false }, &nothing()).unwrap();
    }
}
