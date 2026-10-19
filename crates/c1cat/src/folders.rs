//! `folders`: making and changing the definition files (`*.c1folders`) of photo folder
//! sources.

use std::path::{Path, PathBuf};

use anyhow::Context;
use c1model::provider::data_root;
use clap::{Subcommand, ValueEnum};
use source_folders::pattern::Pattern;
use source_folders::{Definition, FolderSource, ImageStore, Migrate, Preset, STORE_FILE};

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
    /// Choose where thumbnails and previews are kept, and what happens to the ones made.
    SetStore {
        /// central: the app's data folder; per-root: `.c1studio` under each root;
        /// beside: a `.previews` folder next to each photo; custom: your own patterns
        /// (--thumb and --preview).
        #[arg(value_enum)]
        preset: Store,
        /// Size limit of the central store, in megabytes; 0 for no limit.
        #[arg(long, value_name = "MB")]
        limit_mb: Option<u64>,
        /// The thumbnails' path pattern, such as `{dir}/.previews/{stem}.{tier}.jpg`
        /// (custom only).
        #[arg(long, value_name = "PATTERN")]
        thumb: Option<String>,
        /// The previews' path pattern (custom only).
        #[arg(long, value_name = "PATTERN")]
        preview: Option<String>,
        /// Move the images already made to the new place. One of --move and --regenerate is
        /// needed when there are some.
        #[arg(long = "move", conflicts_with = "regenerate")]
        move_images: bool,
        /// Delete the images already made; they are made again in the new place as needed.
        #[arg(long)]
        regenerate: bool,
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
    Custom,
}

impl Store {
    fn preset(self) -> Preset {
        match self {
            Self::Central => Preset::Central,
            Self::PerRoot => Preset::PerRoot,
            Self::Beside => Preset::Beside,
            Self::Custom => Preset::Custom,
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
        Action::SetStore { preset, limit_mb, thumb, preview, move_images, regenerate, file } => {
            let file = definition_file(file.path.as_deref(), source)?;
            let wanted = StoreChange { preset: preset.preset(), limit_mb, thumb, preview };
            let how = match (move_images, regenerate) {
                (true, _) => Some(Migrate::Move),
                (_, true) => Some(Migrate::Regenerate),
                _ => None,
            };
            set_store(&file, &work_dir(&file)?, &wanted, how)
        }
    }
}

/// The image store the user asked for.
struct StoreChange {
    preset: Preset,
    limit_mb: Option<u64>,
    thumb: Option<String>,
    preview: Option<String>,
}

impl StoreChange {
    /// The store this makes of `old`, with its patterns checked on their own.
    fn build(&self, old: &ImageStore) -> anyhow::Result<ImageStore> {
        let mut store = ImageStore::preset(self.preset);
        if self.preset == Preset::Custom {
            let (Some(thumb), Some(preview)) = (&self.thumb, &self.preview) else {
                anyhow::bail!("custom needs both --thumb PATTERN and --preview PATTERN");
            };
            store.thumb = thumb.clone();
            store.preview = preview.clone();
        } else if self.thumb.is_some() || self.preview.is_some() {
            anyhow::bail!("--thumb and --preview go with the custom store");
        }
        match self.limit_mb {
            Some(0) => store.central_limit_mb = None,
            Some(mb) => store.central_limit_mb = Some(mb),
            // Keeps the limit the definition had.
            None => store.central_limit_mb = old.central_limit_mb,
        }
        for (label, text) in [("thumbnail", &store.thumb), ("preview", &store.preview)] {
            let pattern = Pattern::parse(text).map_err(|e| anyhow::anyhow!("the {label} pattern: {e}"))?;
            if let Some(problem) = pattern.static_problem() {
                anyhow::bail!("the {label} pattern {text:?}: {problem}");
            }
        }
        Ok(store)
    }
}

/// The folder where the app keeps what the source at `definition` made, and its store.
fn work_dir(definition: &Path) -> anyhow::Result<PathBuf> {
    let provider = c1sources::provider("folders").map_err(anyhow::Error::msg)?;
    Ok(data_root().join(provider.data_key(&definition.to_string_lossy())).join("work"))
}

/// Changes the image store of the definition at `file`, whose source keeps its data in
/// `work_dir`. A source that has been read is opened: its photos show whether the new store
/// would give two of them one file, and the images it made are moved or deleted as `how`
/// says. Nothing is saved when there are images to deal with and `how` is `None`.
fn set_store(file: &Path, work_dir: &Path, wanted: &StoreChange, how: Option<Migrate>) -> anyhow::Result<()> {
    let mut def = load(file)?;
    let store = wanted.build(&def.images)?;
    let source = if work_dir.join(STORE_FILE).is_file() {
        let source = FolderSource::open(file, work_dir)
            .with_context(|| format!("opening {} (is the app using it?)", file.display()))?;
        if let Some(problem) = source.check_image_store(&store) {
            anyhow::bail!("{problem}");
        }
        Some(source)
    } else {
        None
    };
    let elsewhere = def.images.thumb != store.thumb || def.images.preview != store.preview;
    let made = source.as_ref().map_or(0, FolderSource::image_count);
    if made > 0 && elsewhere && how.is_none() {
        anyhow::bail!(
            "{made} images were made in the old place. Pass --move to move them to the new place, or \
             --regenerate to delete them and make them again as needed. Nothing was changed."
        );
    }
    def.images = store;
    def.save(file).with_context(|| format!("writing {}", file.display()))?;
    println!("images of {} go to the {:?} store", file.display(), def.images.preset);
    let (Some(source), Some(how)) = (source, how) else {
        if made == 0 {
            println!("no images have been made yet; they are made in the new place when needed");
        }
        return Ok(());
    };
    let Some(report) = source.reload_definition(Some(how)).context("applying the new store")? else {
        println!("the images made stay where they are: the new store puts them in the same places");
        return Ok(());
    };
    println!("{} images moved, {} deleted, {} failed", report.moved, report.removed, report.failed.len());
    for failure in &report.failed {
        println!("  {failure}");
    }
    if report.failed.is_empty() { Ok(()) } else { anyhow::bail!("{} images could not be handled", report.failed.len()) }
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
        Some(dir) if dir.is_dir() => dir.join(def.file_name()),
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
        let set = |preset, limit_mb| Action::SetStore {
            preset,
            limit_mb,
            thumb: None,
            preview: None,
            move_images: false,
            regenerate: false,
            file: arg(),
        };
        // No source has been read, so there are no images to deal with.
        run(set(Store::Beside, None), &nothing()).unwrap();
        assert_eq!(Definition::load(&file).unwrap().images.preset, Preset::Beside);
        run(set(Store::Central, Some(0)), &nothing()).unwrap();
        assert_eq!(Definition::load(&file).unwrap().images.limit_bytes(), None);
    }

    fn change(preset: Preset, thumb: Option<&str>, preview: Option<&str>) -> StoreChange {
        StoreChange { preset, limit_mb: None, thumb: thumb.map(str::to_owned), preview: preview.map(str::to_owned) }
    }

    #[test]
    fn a_custom_store_needs_both_patterns_and_good_ones() {
        let old = ImageStore::default();
        let build = |c: StoreChange| c.build(&old).map_err(|e| e.to_string());
        let good = "{dir}/.previews/{stem}.{tier}.jpg";
        let store = build(change(Preset::Custom, Some(good), Some(good))).unwrap();
        assert_eq!((store.preset, store.thumb.as_str()), (Preset::Custom, good));
        assert!(build(change(Preset::Custom, Some(good), None)).unwrap_err().contains("both"));
        assert!(build(change(Preset::Central, Some(good), None)).unwrap_err().contains("custom"));
        let error = build(change(Preset::Custom, Some(good), Some("{dir}/{stem}.jpg"))).unwrap_err();
        assert!(error.contains("preview") && error.contains("{tier}"), "{error}");
        let error = build(change(Preset::Custom, Some("{nope}.jpg"), Some(good))).unwrap_err();
        assert!(error.contains("thumbnail") && error.contains("{nope}"), "{error}");
    }

    /// A definition over a folder of `names` (JPEGs or PNGs by extension), read once with a
    /// thumbnail made for each, as the app would leave it.
    struct Read {
        dir: tempfile::TempDir,
        file: PathBuf,
        work: PathBuf,
    }

    fn read_photos(names: &[&str]) -> Read {
        use c1model::{ImageTier, PhotoSource, SourceEvent};
        let dir = tempfile::tempdir().unwrap();
        let photos = dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        for name in names {
            image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([x as u8, y as u8, 7])).save(photos.join(name)).unwrap();
        }
        let file = dir.path().join("p.c1folders");
        Definition::new("P", &[photos]).save(&file).unwrap();
        let work = dir.path().join("work");
        let source = FolderSource::open(&file, &work).unwrap();
        let mut ids = Vec::new();
        source
            .snapshot(None, &mut |event| {
                if let SourceEvent::Upsert(photo) = event {
                    ids.push(photo.id.clone());
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(ids.len(), names.len());
        for id in &ids {
            source.make_image(id, ImageTier::Thumb).unwrap().unwrap();
        }
        assert!(source.image_count() >= names.len());
        Read { dir, file, work }
    }

    fn beside() -> StoreChange {
        change(Preset::Beside, None, None)
    }

    /// The JPEGs below `dir` that are not photos: the ones in a `.previews` folder or the
    /// central store.
    fn made_below(dir: &Path) -> Vec<PathBuf> {
        let photo = |p: &PathBuf| p.parent().is_some_and(|d| d.ends_with("photos"));
        walk(dir).into_iter().filter(|p| p.extension().is_some_and(|e| e == "jpg") && !photo(p)).collect()
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() { found.extend(walk(&path)) } else { found.push(path) }
        }
        found
    }

    #[test]
    fn images_already_made_must_be_moved_or_dropped_before_the_store_changes() {
        let read = read_photos(&["a.jpg", "b.jpg"]);
        let error = set_store(&read.file, &read.work, &beside(), None).unwrap_err().to_string();
        assert!(error.contains("--move") && error.contains("--regenerate"), "{error}");
        let def = Definition::load(&read.file).unwrap();
        assert_eq!(def.images.preset, Preset::Central, "nothing was saved");
        let central = made_below(&read.work);
        assert_eq!(central.len(), 2);

        set_store(&read.file, &read.work, &beside(), Some(Migrate::Move)).unwrap();
        assert_eq!(Definition::load(&read.file).unwrap().images.preset, Preset::Beside);
        assert!(central.iter().all(|p| !p.exists()), "the old files are gone");
        let moved = made_below(&read.dir.path().join("photos"));
        assert_eq!(moved.len(), 2, "{moved:?}");
        assert!(moved.iter().all(|p| p.parent().unwrap().ends_with(".previews")));
        // The source reads the moved files back, so they were recorded in the new place.
        let again = FolderSource::open(&read.file, &read.work).unwrap();
        assert_eq!(again.image_count(), 2);
    }

    #[test]
    fn regenerate_deletes_the_images_made() {
        let read = read_photos(&["a.jpg"]);
        set_store(&read.file, &read.work, &beside(), Some(Migrate::Regenerate)).unwrap();
        assert!(made_below(&read.dir.path().join("photos")).is_empty());
        assert!(made_below(&read.work).is_empty());
        assert_eq!(FolderSource::open(&read.file, &read.work).unwrap().image_count(), 0);
        assert_eq!(Definition::load(&read.file).unwrap().images.preset, Preset::Beside);
    }

    #[test]
    fn a_store_that_would_give_two_photos_one_file_is_refused() {
        // a.jpg and a.png are both `a` to {stem}.
        let read = read_photos(&["a.jpg", "a.png"]);
        let stem = "{dir}/.previews/{stem}.{tier}.jpg";
        let clash =
            set_store(&read.file, &read.work, &change(Preset::Custom, Some(stem), Some(stem)), Some(Migrate::Move));
        let error = clash.unwrap_err().to_string();
        assert!(error.contains("would share"), "{error}");
        assert_eq!(Definition::load(&read.file).unwrap().images.preset, Preset::Central, "nothing was saved");
        let name = "{dir}/.previews/{name}.{tier}.jpg";
        set_store(&read.file, &read.work, &change(Preset::Custom, Some(name), Some(name)), Some(Migrate::Move))
            .unwrap();
        assert_eq!(Definition::load(&read.file).unwrap().images.thumb, name);
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
