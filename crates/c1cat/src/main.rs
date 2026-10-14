//! `c1cat`: the c1-studio library from the command line.

mod bench;
mod selector;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Context;
use c1index::{Dim, Filter, Library, QueryOptions, Sort};
use c1model::{
    ApplyPlan, Change, CollectionChange, CollectionId, CollectionKind, ColorLabel, Condition, GeoOrigin, GeoPoint,
    ImageTier, KeywordPath, KeywordRemap, Op, Photo, PhotoSource, SmartRule,
};
use clap::{Parser, Subcommand};
use source_c1::{CaptureOne, data_dir, find_catalog};

use crate::selector::Selector;

#[derive(Parser)]
#[command(name = "c1cat", version, about = "Search and inspect a Capture One catalog through the c1-studio library")]
struct Cli {
    /// Catalog file; defaults to C1_CATALOG, then %APPDATA%\c1studio\config.json.
    #[arg(long, global = true)]
    catalog: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Pull the catalog into the library (fast when nothing changed).
    Sync,
    /// Catalog and library state.
    Status,
    /// Measure each photo's thumbnail (shape, colours, light, focus, a hash for
    /// duplicates and bursts); only photos not measured yet, or whose thumbnail changed.
    Analyse {
        /// Worker threads; all cores but one by default.
        #[arg(long)]
        threads: Option<usize>,
        /// Print the spread of each measure and how many photos fall in each class.
        #[arg(long)]
        stats: bool,
        #[arg(long)]
        no_sync: bool,
    },
    /// The backups made before writes, newest first.
    Backups {
        #[arg(long)]
        json: bool,
    },
    /// Put a backup (by file name, from `backups`) back as the catalog. The catalog as it
    /// is now is backed up first. A dry run unless --apply is given.
    Restore {
        #[arg(value_name = "BACKUP")]
        name: String,
        #[arg(long)]
        apply: bool,
    },
    /// List matching photos.
    Find {
        #[command(flatten)]
        select: Selector,
        /// date, date-desc, rating, rating-desc, imported, imported-desc, filename, filename-desc, id.
        #[arg(long, default_value = "date-desc")]
        sort: String,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Print only source ids, one per line.
        #[arg(long = "ids")]
        print_ids: bool,
        /// Print full records as JSON.
        #[arg(long)]
        json: bool,
        /// Skip syncing first.
        #[arg(long)]
        no_sync: bool,
    },
    /// Count matching photos.
    Count {
        #[command(flatten)]
        select: Selector,
        #[arg(long)]
        no_sync: bool,
    },
    /// Count values per dimension over the matches.
    Facets {
        #[command(flatten)]
        select: Selector,
        /// Comma-separated: keyword, keyword_tree, collection, camera, lens, country, state,
        /// city, year, month, rating, color, ext, style, flag, source.
        #[arg(long, default_value = "rating,color,year,camera,keyword_tree")]
        dims: String,
        /// Values shown per dimension.
        #[arg(long, default_value_t = 15)]
        top: usize,
        #[arg(long)]
        no_sync: bool,
    },
    /// Trips found from when and where photos were taken (and what was done to them), each
    /// with how many of its photos match. Trips are found once home is set.
    Trips {
        #[command(flatten)]
        select: Selector,
        /// Set home first: `Country` (only trips abroad) or `Country|City`; `none` clears it.
        #[arg(long)]
        home: Option<String>,
        /// Set the hours without photos that start a new trip first.
        #[arg(long)]
        gap_hours: Option<f64>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        no_sync: bool,
    },
    /// Write viewable JPEGs of matching photos, from Capture One's thumbnail, its
    /// preview (white-balanced here) or the original's embedded JPEG.
    Image {
        #[command(flatten)]
        select: Selector,
        /// thumb, preview or original.
        #[arg(long, default_value = "preview")]
        tier: String,
        /// Folder to write into.
        #[arg(long, short)]
        out: PathBuf,
        /// Longer edge in pixels; 0 keeps the size.
        #[arg(long, default_value_t = 0)]
        size: u32,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long)]
        no_sync: bool,
    },
    /// Set or clear the star rating. A dry run unless --apply is given.
    Rate {
        #[command(flatten)]
        select: Selector,
        /// 0 to 5; 0 means unrated.
        #[arg(long, value_name = "0-5")]
        set: Option<u8>,
        #[arg(long)]
        clear: bool,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Set the colour tag (none, red, orange, yellow, green, blue, pink, purple).
    #[command(alias = "colour")]
    Color {
        #[command(flatten)]
        select: Selector,
        #[arg(long, value_name = "NAME")]
        set: String,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Set or clear the caption, headline or title. An empty value clears.
    Describe {
        #[command(flatten)]
        select: Selector,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        headline: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Set or clear IPTC contact and location fields. An empty value clears. Prefixed
    /// with --set- (as the Python tool does) so they never read as filters.
    Iptc {
        #[command(flatten)]
        select: Selector,
        #[arg(long)]
        set_creator: Option<String>,
        #[arg(long)]
        set_copyright: Option<String>,
        #[arg(long)]
        set_instructions: Option<String>,
        #[arg(long)]
        set_job_id: Option<String>,
        #[arg(long)]
        set_source: Option<String>,
        #[arg(long)]
        set_city: Option<String>,
        #[arg(long)]
        set_state: Option<String>,
        #[arg(long)]
        set_country: Option<String>,
        #[arg(long)]
        set_country_code: Option<String>,
        #[arg(long)]
        set_location: Option<String>,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Set or clear GPS coordinates. Stored per image: every variant of it moves.
    Geo {
        #[command(flatten)]
        select: Selector,
        #[arg(long, allow_hyphen_values = true)]
        lat: Option<f64>,
        #[arg(long, allow_hyphen_values = true)]
        lon: Option<f64>,
        /// Both at once, as written anywhere: "50.0614, 19.9366", "44°58′N 14°45′E".
        #[arg(long, allow_hyphen_values = true, conflicts_with_all = ["lat", "lon"])]
        at: Option<String>,
        #[arg(long)]
        clear: bool,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Add or remove keywords on the matching photos. A dry run unless --apply is given.
    #[command(alias = "kw")]
    Keyword {
        #[command(subcommand)]
        action: KeywordAction,
    },
    /// Make, rename and delete albums, and put photos in or take them out. A dry run
    /// unless --apply is given.
    Album {
        #[command(subcommand)]
        action: AlbumAction,
    },
    /// Album groups.
    Group {
        #[command(subcommand)]
        action: GroupAction,
    },
    /// Smart albums: a rule instead of members.
    Smart {
        #[command(subcommand)]
        action: SmartAction,
    },
    /// Time the library against the stage-0 speed gate on synthetic photos.
    Bench {
        #[arg(long, default_value_t = 250_000)]
        photos: usize,
        #[arg(long, default_value_t = 2_000)]
        keywords: usize,
        /// Queries timed per class.
        #[arg(long, default_value_t = 500)]
        queries: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Library directory; kept and reused across runs with the same size and seed.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Build the library again even if one is there.
        #[arg(long)]
        rebuild: bool,
        /// Dimensions counted with every query, as the sidebar would.
        #[arg(long, default_value = "rating,color,year,camera,lens,keyword_tree,collection,country")]
        dims: String,
    },
}

#[derive(Subcommand)]
enum KeywordAction {
    /// Add keywords (paths like `best|best2018`); missing ones join the keyword tree.
    Add {
        #[arg(required = true, value_name = "KEYWORD")]
        paths: Vec<String>,
        #[command(flatten)]
        select: Selector,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Remove keywords from the photos; the keyword tree keeps them.
    Remove {
        #[arg(required = true, value_name = "KEYWORD")]
        paths: Vec<String>,
        #[command(flatten)]
        select: Selector,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Rename a keyword everywhere.
    Rename {
        #[arg(long)]
        path: String,
        #[arg(long, value_name = "NAME")]
        to: String,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Move a keyword (and what is under it) under another, or to the top level.
    Move {
        #[arg(long)]
        path: String,
        #[arg(long)]
        to_parent: Option<String>,
        #[arg(long)]
        to_root: bool,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Join keywords into one, existing or new, on every photo.
    Merge {
        #[arg(long = "from", required = true)]
        from: Vec<String>,
        #[arg(long)]
        into: String,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Put keywords under a parent keyword, existing or new.
    Group {
        #[arg(long = "path", required = true)]
        paths: Vec<String>,
        #[arg(long)]
        under: String,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Move a group's keywords up a level; the group goes if no photo carries it.
    Ungroup {
        #[arg(long)]
        path: String,
        #[command(flatten)]
        write: WriteFlags,
    },
}

// Albums are named as the Python `--album` named them: `#906`, a name, or a path suffix.
#[derive(Subcommand)]
enum AlbumAction {
    /// Make an album, at the top level or in a group or project.
    Create {
        name: String,
        #[arg(long, value_name = "NAME|#PK")]
        parent: Option<String>,
        #[arg(long)]
        allow_duplicate: bool,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Rename an album, smart album or group.
    Rename {
        #[arg(value_name = "NAME|#PK")]
        album: String,
        #[arg(long)]
        to: String,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Delete an album, smart album or group (and what is in it). Photos stay.
    Delete {
        #[arg(value_name = "NAME|#PK")]
        album: String,
        /// Needed to delete a group with collections in it.
        #[arg(long)]
        force_children: bool,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Put the matching photos into an album. (The album is positional: `--album` is a
    /// filter here, so `album add portfolio --album street` copies street into portfolio.)
    Add {
        #[arg(value_name = "ALBUM")]
        album: String,
        #[command(flatten)]
        select: Selector,
        /// Make the album if there is none of that name.
        #[arg(long)]
        create: bool,
        /// Where `--create` makes it.
        #[arg(long, value_name = "NAME|#PK")]
        parent: Option<String>,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Take the matching photos out of an album.
    Remove {
        #[arg(value_name = "ALBUM")]
        album: String,
        #[command(flatten)]
        select: Selector,
        #[command(flatten)]
        write: WriteFlags,
    },
}

#[derive(Subcommand)]
enum GroupAction {
    /// Make a group, to hold albums.
    Create {
        name: String,
        #[arg(long, value_name = "NAME|#PK")]
        parent: Option<String>,
        #[arg(long)]
        allow_duplicate: bool,
        #[command(flatten)]
        write: WriteFlags,
    },
}

#[derive(Subcommand)]
enum SmartAction {
    /// Make a smart album from a rule.
    Create {
        name: String,
        #[arg(long, value_name = "NAME|#PK")]
        parent: Option<String>,
        #[command(flatten)]
        rule: RuleArgs,
        #[arg(long)]
        allow_duplicate: bool,
        #[command(flatten)]
        write: WriteFlags,
    },
    /// Give a smart album a new rule.
    Rule {
        #[arg(value_name = "NAME|#PK")]
        album: String,
        #[command(flatten)]
        rule: RuleArgs,
        #[command(flatten)]
        write: WriteFlags,
    },
}

/// A smart album's rule, in the order the Python `smart create` writes it: rating,
/// keywords, year.
#[derive(clap::Args)]
struct RuleArgs {
    /// Every condition (all) or any one of them (any).
    #[arg(long = "match", default_value = "all", value_parser = ["all", "any"])]
    match_: String,
    /// '>=4' (at least), '>3' (written as at least 4) or '5'.
    #[arg(long)]
    rating: Option<String>,
    /// Some keyword's name contains this (repeatable).
    #[arg(long)]
    keyword: Vec<String>,
    /// Captured in this year.
    #[arg(long)]
    year: Option<i32>,
}

impl RuleArgs {
    fn rule(&self) -> anyhow::Result<SmartRule> {
        let mut conditions = Vec::new();
        if let Some(text) = &self.rating {
            let t = text.trim();
            let n = |s: &str| s.trim().parse::<u8>().ok().filter(|n| *n <= 5);
            conditions.push(
                match (t.strip_prefix(">="), t.strip_prefix('>'), t.strip_prefix('=')) {
                    (Some(v), _, _) => n(v).map(|min| Condition::Rating { min, max: 5 }),
                    (None, Some(v), _) => n(v).filter(|v| *v < 5).map(|v| Condition::Rating { min: v + 1, max: 5 }),
                    (None, None, Some(v)) => n(v).map(|v| Condition::Rating { min: v, max: v }),
                    _ => n(t).map(|v| Condition::Rating { min: v, max: v }),
                }
                .with_context(|| format!("cannot read the rating {text:?}; use '>=4', '>3' or '4'"))?,
            );
        }
        conditions.extend(self.keyword.iter().map(|k| Condition::KeywordContains(k.clone())));
        conditions.extend(self.year.map(Condition::Year));
        if conditions.is_empty() {
            anyhow::bail!("a smart album needs at least one condition (--rating, --keyword or --year)");
        }
        Ok(SmartRule { match_all: self.match_ == "all", conditions })
    }
}

#[derive(clap::Args)]
struct WriteFlags {
    /// Write to the catalog. Without it nothing is written and the plan is shown.
    #[arg(long)]
    apply: bool,
    /// Print the outcome as JSON.
    #[arg(long)]
    json: bool,
    #[arg(long)]
    no_sync: bool,
}

struct Session {
    source: CaptureOne,
    library: Library,
    /// Thumbnails made for photos Capture One has none for (shared with the app).
    thumbs: c1index::thumbs::Thumbs,
}

fn open(catalog: Option<&PathBuf>) -> anyhow::Result<Session> {
    let path = find_catalog(catalog.map(PathBuf::as_path))?;
    let dir = data_dir(&path);
    let source = CaptureOne::open(&path, dir.join("work"))?;
    let library = Library::open(dir.join("library")).context("opening the library")?;
    Ok(Session { source, library, thumbs: c1index::thumbs::Thumbs::new(dir.join("thumbs")) })
}

fn sync(session: &Session, quiet: bool) -> anyhow::Result<()> {
    let report = session.library.sync(&session.source)?;
    if !quiet && (report.full || report.added + report.updated + report.deleted > 0) {
        eprintln!(
            "synced {}: +{} ~{} -{} ={} in {} ms",
            session.source.info().name,
            report.added,
            report.updated,
            report.deleted,
            report.unchanged,
            report.millis
        );
    }
    Ok(())
}

fn parse_dims(text: &str) -> anyhow::Result<Vec<Dim>> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            serde_json::from_value(serde_json::Value::String(s.to_owned()))
                .with_context(|| format!("unknown dimension {s:?}"))
        })
        .collect()
}

/// A text field from the command line: absent leaves it, empty clears it.
fn set_text(field: &mut Option<String>, given: &Option<String>) {
    if let Some(text) = given {
        *field = (!text.is_empty()).then(|| text.clone());
    }
}

/// Plans `op` for every matching photo and, with `--apply`, writes it. Capture One must
/// be closed for the write; the plan works either way.
fn edit(
    catalog: Option<&PathBuf>,
    select: &Selector,
    flags: &WriteFlags,
    op: impl Fn(&Photo) -> Op,
) -> anyhow::Result<()> {
    edit_many(catalog, select, flags, |p| vec![op(p)])
}

/// As [`edit`], with any number of ops per photo.
fn edit_many(
    catalog: Option<&PathBuf>,
    select: &Selector,
    flags: &WriteFlags,
    ops: impl Fn(&Photo) -> Vec<Op>,
) -> anyhow::Result<()> {
    let session = open(catalog)?;
    if !flags.no_sync {
        sync(&session, true)?;
    }
    let filter = select.to_filter(&session.library)?;
    if filter == Filter::default() {
        anyhow::bail!("refusing to run without a selection; add at least one filter (e.g. --id, --album, --rating)");
    }
    let result = session.library.query(&filter, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?;
    let mut changes = Vec::with_capacity(result.rows.len());
    for row in result.rows {
        if let Some(photo) = session.library.photo(row)? {
            changes.extend(ops(&photo).into_iter().map(|op| Change { op, photo: photo.id.clone() }));
        }
    }
    let plan = session.source.plan(&changes)?;
    let report = if flags.apply { Some(session.source.apply(&plan)?) } else { None };
    if report.is_some() {
        sync(&session, true)?;
    }
    let ids = |list: &[Change]| list.iter().map(|c| c.photo.ext.clone()).collect::<Vec<_>>();
    if flags.json {
        let out = serde_json::json!({
            "applied": report.as_ref().map(|r| r.applied),
            "backup": report.as_ref().and_then(|r| r.backup.clone()),
            "apply": ids(&plan.apply),
            "unchanged": ids(&plan.no_ops),
            "conflicts": plan.conflicts.iter().map(|c| (c.change.photo.ext.clone(), c.current.clone())).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    println!("{} to change, {} already so, {} in conflict", plan.apply.len(), plan.no_ops.len(), plan.conflicts.len());
    for c in plan.conflicts.iter().take(20) {
        println!("  conflict {}: the catalog now holds {}", c.change.photo.ext, c.current);
    }
    match report {
        Some(r) => {
            println!("written: {}", r.applied);
            if let Some(b) = r.backup {
                println!("backup: {b}");
            }
        }
        None if !plan.apply.is_empty() => println!("dry run: nothing written; add --apply to write"),
        None => {}
    }
    Ok(())
}

/// Plans a keyword tree change and, with `--apply`, writes it.
fn remap(
    catalog: Option<&PathBuf>,
    flags: &WriteFlags,
    build: impl FnOnce(&Library) -> anyhow::Result<KeywordRemap>,
) -> anyhow::Result<()> {
    let session = open(catalog)?;
    if !flags.no_sync {
        sync(&session, true)?;
    }
    let remap = build(&session.library)?;
    if let Some(why) = session.source.check_remap(&[], &remap)? {
        anyhow::bail!("{}: {why}", remap.label);
    }
    let froms: Vec<String> = remap.map.iter().map(|(f, _)| f.to_pipe()).collect();
    let carrying = Filter { keywords_under: froms, trashed: c1index::Trashed::Include, ..Filter::default() };
    let photos = session.library.query(&carrying, &QueryOptions { limit: Some(0), ..QueryOptions::default() })?.total;
    let report = if flags.apply {
        let plan = ApplyPlan { remaps: vec![remap.clone()], ..ApplyPlan::default() };
        let report = session.source.apply(&plan)?;
        sync(&session, true)?;
        Some(report)
    } else {
        None
    };
    if flags.json {
        let out = serde_json::json!({
            "remap": remap.label,
            "photos": photos,
            "applied": report.as_ref().map(|r| r.applied),
            "backup": report.as_ref().and_then(|r| r.backup.clone()),
            "conflicts": [],
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("{}: {photos} photo(s) carry these keywords", remap.label);
        match report {
            Some(r) => println!("written; backup: {}", r.backup.unwrap_or_default()),
            None => println!("dry run: nothing written; add --apply to write"),
        }
    }
    Ok(())
}

/// The photos a selection matches; refuses an empty selection, as `edit` does.
fn selected(session: &Session, select: &Selector) -> anyhow::Result<Vec<Photo>> {
    let filter = select.to_filter(&session.library)?;
    if filter == Filter::default() {
        anyhow::bail!("refusing to run without a selection; add at least one filter (e.g. --id, --album, --rating)");
    }
    let rows = session.library.query(&filter, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.extend(session.library.photo(row)?);
    }
    Ok(out)
}

/// A new collection, refused if one of that name is already in the same place.
fn create(
    session: &Session,
    name: &str,
    kind: CollectionKind,
    parent: Option<&str>,
    rule: Option<SmartRule>,
    allow_duplicate: bool,
) -> anyhow::Result<CollectionChange> {
    let parent = parent.map(|p| session.library.resolve_collection(p).map(CollectionId)).transpose()?;
    let all = session.library.collections()?;
    if let Some(same) = all
        .iter()
        .find(|c| c.parent == parent && c.name.to_lowercase() == name.to_lowercase())
        .filter(|_| !allow_duplicate)
    {
        anyhow::bail!("{:?} already exists (#{}); pass --allow-duplicate to make another", same.path, same.id.0);
    }
    Ok(CollectionChange::Create { id: c1index::new_collection_id(), name: name.to_owned(), kind, parent, rule })
}

/// Plans album changes, then photo changes into and out of albums, and with `--apply`
/// writes them together.
fn albums(
    catalog: Option<&PathBuf>,
    flags: &WriteFlags,
    build: impl FnOnce(&Session) -> anyhow::Result<(Vec<CollectionChange>, Vec<Change>)>,
) -> anyhow::Result<()> {
    let session = open(catalog)?;
    if !flags.no_sync {
        sync(&session, true)?;
    }
    let (collections, changes) = build(&session)?;
    let before = session.library.collections()?;
    let labels: Vec<String> = collections.iter().map(|c| c.label(&before)).collect();
    for (i, change) in collections.iter().enumerate() {
        if let Some(why) = session.source.check_collection(&collections[..i], change)? {
            anyhow::bail!("{}: {why}", labels[i]);
        }
    }
    let plan = session.source.plan_after(&[], &collections, &changes)?;
    let report = if flags.apply {
        let report = session.source.apply(&plan)?;
        sync(&session, true)?;
        Some(report)
    } else {
        None
    };
    let ids = |list: &[Change]| list.iter().map(|c| c.photo.ext.clone()).collect::<Vec<_>>();
    if flags.json {
        let out = serde_json::json!({
            "collections": labels,
            "applied": report.as_ref().map(|r| r.applied),
            "backup": report.as_ref().and_then(|r| r.backup.clone()),
            "apply": ids(&plan.apply),
            "unchanged": ids(&plan.no_ops),
            "conflicts": plan.conflicts.iter().map(|c| (c.change.photo.ext.clone(), c.current.clone())).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    for label in &labels {
        println!("{label}");
    }
    if !changes.is_empty() {
        println!(
            "{} photo(s) to move, {} already so, {} refused",
            plan.apply.len(),
            plan.no_ops.len(),
            plan.conflicts.len()
        );
        for c in plan.conflicts.iter().take(20) {
            println!("  refused {}: {}", c.change.photo.ext, c.current);
        }
    }
    match report {
        Some(r) => println!("written: {}; backup: {}", r.applied, r.backup.unwrap_or_default()),
        None => println!("dry run: nothing written; add --apply to write"),
    }
    Ok(())
}

/// The spread of each measure (percentiles) and the size of each class.
fn print_stats(library: &Library) {
    let looks = library.looks();
    let n = looks.len();
    if n == 0 {
        println!("nothing measured");
        return;
    }
    let spread = |name: &str, mut v: Vec<f32>| {
        v.sort_by(f32::total_cmp);
        let at = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
        println!(
            "{name:<11} p1 {:>8.2}  p5 {:>8.2}  p10 {:>8.2}  p25 {:>8.2}  p50 {:>8.2}  p75 {:>8.2}  p95 {:>8.2}",
            at(0.01),
            at(0.05),
            at(0.1),
            at(0.25),
            at(0.5),
            at(0.75),
            at(0.95)
        );
    };
    println!("{n} photos measured");
    spread("sharpness", looks.iter().map(|l| l.analysis.sharpness).collect());
    spread("brightness", looks.iter().map(|l| l.analysis.brightness).collect());
    spread("contrast", looks.iter().map(|l| l.analysis.contrast).collect());
    spread("chroma", looks.iter().map(|l| l.analysis.chroma).collect());
    spread("highlights", looks.iter().map(|l| l.analysis.highlights).collect());
    spread("shadows", looks.iter().map(|l| l.analysis.shadows).collect());
    let mut classes: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for l in &looks {
        let a = &l.analysis;
        *classes.entry(format!("shape  {}", a.shape())).or_default() += 1;
        *classes.entry(format!("focus  {}", a.focus())).or_default() += 1;
        for h in a.hues() {
            *classes.entry(format!("hue    {h}")).or_default() += 1;
        }
        for v in a.light() {
            *classes.entry(format!("light  {v}")).or_default() += 1;
        }
    }
    for (class, count) in classes {
        println!("{class:<28} {count:>7}  {:>5.1}%", 100.0 * count as f64 / n as f64);
    }
    let dups: std::collections::HashSet<_> = looks.iter().filter_map(|l| l.duplicates.clone()).collect();
    let series: std::collections::HashSet<_> = looks.iter().filter_map(|l| l.series.clone()).collect();
    println!(
        "duplicates: {} photos in {} groups; series: {} photos in {}",
        looks.iter().filter(|l| l.duplicates.is_some()).count(),
        dups.len(),
        looks.iter().filter(|l| l.series.is_some()).count(),
        series.len()
    );
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Image { select, tier, out, size, limit, no_sync } => {
            let tier = match tier.as_str() {
                "thumb" => ImageTier::Thumb,
                "preview" => ImageTier::Preview,
                "original" => ImageTier::Original,
                other => anyhow::bail!("--tier is thumb, preview or original, not {other:?}"),
            };
            let session = open(cli.catalog.as_ref())?;
            if !no_sync {
                sync(&session, false)?;
            }
            let filter = select.to_filter(&session.library)?;
            let result =
                session.library.query(&filter, &QueryOptions { limit: Some(limit), ..QueryOptions::default() })?;
            std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
            for row in result.rows {
                let Some(id) = session.library.photo_id(row) else { continue };
                let started = Instant::now();
                let Some(source) = session.source.image(&id, tier)? else {
                    eprintln!("{}: no {tier:?} image", id.ext);
                    continue;
                };
                match c1image::load_fitted(&source, size) {
                    Ok(pixels) => {
                        let path = out.join(format!("{}-{}.jpg", id.ext, format!("{tier:?}").to_lowercase()));
                        let file = std::io::BufWriter::new(std::fs::File::create(&path)?);
                        image::codecs::jpeg::JpegEncoder::new_with_quality(file, 90).encode_image(&pixels)?;
                        let ms = started.elapsed().as_secs_f64() * 1000.0;
                        println!("{}  {}×{}  {ms:.0} ms", path.display(), pixels.width(), pixels.height());
                    }
                    Err(e) => eprintln!("{}: {e}", id.ext),
                }
            }
        }
        Command::Rate { select, set, clear, write } => {
            let value = match (set, clear) {
                (Some(n @ 0..=5), false) => (n > 0).then_some(n),
                (None, true) => None,
                _ => anyhow::bail!("pass --set 0..5 or --clear"),
            };
            edit(cli.catalog.as_ref(), &select, &write, |p| Op::SetRating { base: p.rating, value })?;
        }
        Command::Color { select, set, write } => {
            let value = match set.trim().to_ascii_lowercase().as_str() {
                "none" => None,
                name => Some(ColorLabel::from_name(name).with_context(|| {
                    format!("unknown colour {set:?}; choose none, red, orange, yellow, green, blue, pink or purple")
                })?),
            };
            edit(cli.catalog.as_ref(), &select, &write, |p| Op::SetColor { base: p.color, value })?;
        }
        Command::Describe { select, description, headline, title, write } => {
            if description.is_none() && headline.is_none() && title.is_none() {
                anyhow::bail!("pass --description, --headline or --title");
            }
            edit(cli.catalog.as_ref(), &select, &write, |p| {
                let mut value = p.iptc.clone();
                set_text(&mut value.description, &description);
                set_text(&mut value.headline, &headline);
                set_text(&mut value.title, &title);
                Op::SetIptc { base: p.iptc.clone(), value }
            })?;
        }
        Command::Iptc {
            select,
            set_creator,
            set_copyright,
            set_instructions,
            set_job_id,
            set_source,
            set_city,
            set_state,
            set_country,
            set_country_code,
            set_location,
            write,
        } => {
            let iptc = [&set_creator, &set_copyright, &set_instructions, &set_job_id, &set_source];
            let place = [&set_city, &set_state, &set_country, &set_country_code, &set_location];
            if iptc.iter().chain(&place).all(|v| v.is_none()) {
                anyhow::bail!("pass at least one field, e.g. --set-city 'Kraków' --set-country Poland");
            }
            let touches_iptc = iptc.iter().any(|v| v.is_some());
            let touches_place = place.iter().any(|v| v.is_some());
            // One command, up to two ops per photo: planned and written together.
            edit_many(cli.catalog.as_ref(), &select, &write, |p| {
                let mut ops = Vec::new();
                if touches_iptc {
                    let mut value = p.iptc.clone();
                    set_text(&mut value.creator, &set_creator);
                    set_text(&mut value.copyright, &set_copyright);
                    set_text(&mut value.instructions, &set_instructions);
                    set_text(&mut value.job_id, &set_job_id);
                    set_text(&mut value.source, &set_source);
                    ops.push(Op::SetIptc { base: p.iptc.clone(), value });
                }
                if touches_place {
                    let mut value = p.place.clone();
                    set_text(&mut value.city, &set_city);
                    set_text(&mut value.state, &set_state);
                    set_text(&mut value.country, &set_country);
                    set_text(&mut value.country_code, &set_country_code);
                    set_text(&mut value.location, &set_location);
                    ops.push(Op::SetPlace { base: p.place.clone(), value });
                }
                ops
            })?;
        }
        Command::Geo { select, lat, lon, at, clear, write } => {
            let (lat, lon) = match at {
                Some(text) => {
                    let (lat, lon) = c1geo::parse_point(&text).map_err(|e| anyhow::anyhow!("--at {text:?}: {e}"))?;
                    (Some(lat), Some(lon))
                }
                None => (lat, lon),
            };
            let value = match (lat, lon, clear) {
                (Some(lat), Some(lon), false) => {
                    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                        anyhow::bail!("latitude is -90..90 and longitude -180..180");
                    }
                    Some(GeoPoint { lat, lon, alt: None, origin: GeoOrigin::Manual })
                }
                (None, None, true) => None,
                _ => anyhow::bail!("pass --lat and --lon (or --at), or --clear"),
            };
            edit(cli.catalog.as_ref(), &select, &write, |p| Op::SetGeo { base: p.geo, value })?;
        }
        Command::Keyword { action } => {
            let (select, write, op) = match action {
                KeywordAction::Add { paths, select, write } => {
                    let paths = paths.iter().map(|p| KeywordPath::from_pipe(p.trim())).collect();
                    (select, write, Op::AddKeywords { paths })
                }
                KeywordAction::Remove { paths, select, write } => {
                    let paths = paths.iter().map(|p| KeywordPath::from_pipe(p.trim())).collect();
                    (select, write, Op::RemoveKeywords { paths })
                }
                KeywordAction::Rename { path, to, write } => {
                    let path = KeywordPath::from_pipe(&path);
                    return remap(cli.catalog.as_ref(), &write, |_| Ok(KeywordRemap::rename(path, &to)));
                }
                KeywordAction::Move { path, to_parent, to_root, write } => {
                    if to_root == to_parent.is_some() {
                        anyhow::bail!("pass --to-parent PATH or --to-root");
                    }
                    let under = to_parent.map(|p| KeywordPath::from_pipe(&p));
                    let path = KeywordPath::from_pipe(&path);
                    return remap(cli.catalog.as_ref(), &write, |_| Ok(KeywordRemap::group(vec![path], under)));
                }
                KeywordAction::Merge { from, into, write } => {
                    let from = from.iter().map(|p| KeywordPath::from_pipe(p)).collect();
                    let into = KeywordPath::from_pipe(&into);
                    return remap(cli.catalog.as_ref(), &write, |_| Ok(KeywordRemap::join(from, into)));
                }
                KeywordAction::Group { paths, under, write } => {
                    let paths = paths.iter().map(|p| KeywordPath::from_pipe(p)).collect();
                    let under = Some(KeywordPath::from_pipe(&under));
                    return remap(cli.catalog.as_ref(), &write, |_| Ok(KeywordRemap::group(paths, under)));
                }
                KeywordAction::Ungroup { path, write } => {
                    let group = KeywordPath::from_pipe(&path);
                    return remap(cli.catalog.as_ref(), &write, |library| {
                        let children: Vec<KeywordPath> = library
                            .keywords()?
                            .into_iter()
                            .map(|n| n.path)
                            .filter(|p| p.0.len() == group.0.len() + 1 && p.0.starts_with(&group.0))
                            .collect();
                        if children.is_empty() {
                            anyhow::bail!("{path:?} has no keywords under it");
                        }
                        Ok(KeywordRemap::ungroup(group, children))
                    });
                }
            };
            edit(cli.catalog.as_ref(), &select, &write, |_| op.clone())?;
        }
        Command::Album { action } => {
            let catalog = cli.catalog.as_ref();
            let member = |photos: Vec<Photo>, id: &CollectionId, add: bool| -> Vec<Change> {
                let op = if add {
                    Op::AddToCollection { collection: id.clone() }
                } else {
                    Op::RemoveFromCollection { collection: id.clone() }
                };
                photos.into_iter().map(|p| Change { photo: p.id, op: op.clone() }).collect()
            };
            match action {
                AlbumAction::Create { name, parent, allow_duplicate, write } => albums(catalog, &write, |s| {
                    Ok((
                        vec![create(s, &name, CollectionKind::Album, parent.as_deref(), None, allow_duplicate)?],
                        vec![],
                    ))
                })?,
                AlbumAction::Rename { album, to, write } => albums(catalog, &write, |s| {
                    let id = CollectionId(s.library.resolve_collection(&album)?);
                    Ok((vec![CollectionChange::Rename { id, name: to }], vec![]))
                })?,
                AlbumAction::Delete { album, force_children, write } => albums(catalog, &write, |s| {
                    let id = CollectionId(s.library.resolve_collection(&album)?);
                    let all = s.library.collections()?;
                    let inside = all.iter().filter(|c| c.parent.as_ref() == Some(&id)).count();
                    if inside > 0 && !force_children {
                        anyhow::bail!("it holds {inside} collection(s); pass --force-children to delete them too");
                    }
                    Ok((vec![CollectionChange::Delete { id }], vec![]))
                })?,
                AlbumAction::Add { album, select, create: make, parent, write } => albums(catalog, &write, |s| {
                    let photos = selected(s, &select)?;
                    match s.library.resolve_collection(&album) {
                        Ok(id) => Ok((vec![], member(photos, &CollectionId(id), true))),
                        Err(_) if make => {
                            let new = create(s, &album, CollectionKind::Album, parent.as_deref(), None, false)?;
                            let changes = member(photos, new.id(), true);
                            Ok((vec![new], changes))
                        }
                        Err(e) => Err(e),
                    }
                })?,
                AlbumAction::Remove { album, select, write } => albums(catalog, &write, |s| {
                    let id = CollectionId(s.library.resolve_collection(&album)?);
                    Ok((vec![], member(selected(s, &select)?, &id, false)))
                })?,
            }
        }
        Command::Group { action: GroupAction::Create { name, parent, allow_duplicate, write } } => {
            albums(cli.catalog.as_ref(), &write, |s| {
                Ok((vec![create(s, &name, CollectionKind::Group, parent.as_deref(), None, allow_duplicate)?], vec![]))
            })?
        }
        Command::Smart { action } => match action {
            SmartAction::Create { name, parent, rule, allow_duplicate, write } => {
                let rule = rule.rule()?;
                albums(cli.catalog.as_ref(), &write, |s| {
                    let new = create(s, &name, CollectionKind::Smart, parent.as_deref(), Some(rule), allow_duplicate)?;
                    Ok((vec![new], vec![]))
                })?
            }
            SmartAction::Rule { album, rule, write } => {
                let rule = rule.rule()?;
                albums(cli.catalog.as_ref(), &write, |s| {
                    let id = CollectionId(s.library.resolve_collection(&album)?);
                    Ok((vec![CollectionChange::SetRule { id, rule }], vec![]))
                })?
            }
        },
        Command::Bench { photos, keywords, queries, seed, dir, rebuild, dims } => {
            bench::bench(&bench::Config { photos, keywords, queries, seed, dir, rebuild, dims: parse_dims(&dims)? })?;
        }
        Command::Sync => {
            let session = open(cli.catalog.as_ref())?;
            let report = session.library.sync(&session.source)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Analyse { threads, stats, no_sync } => {
            let session = open(cli.catalog.as_ref())?;
            if !no_sync {
                sync(&session, false)?;
            }
            let stop = std::sync::atomic::AtomicBool::new(false);
            let threads = threads.unwrap_or_else(c1index::analyse::default_threads);
            let done =
                c1index::analyse::run(&session.library, &session.source, &session.thumbs, threads, &stop, |p| {
                    eprintln!(
                        "measured {} of {} ({} unreadable) in {:.1} s",
                        p.done,
                        p.total,
                        p.failed,
                        p.millis as f64 / 1000.0
                    );
                })?;
            let per = if done.done > 0 { done.millis as f64 / done.done as f64 } else { 0.0 };
            println!(
                "{} measured, {} unreadable, {:.1} s ({per:.2} ms a photo, {threads} threads)",
                done.done,
                done.failed,
                done.millis as f64 / 1000.0
            );
            if stats {
                print_stats(&session.library);
            }
        }
        Command::Backups { json } => {
            let session = open(cli.catalog.as_ref())?;
            let list = session.source.backups();
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else {
                for b in &list {
                    let note = b.note.as_deref().map(|n| format!("  ({n})")).unwrap_or_default();
                    println!(
                        "{}  {:>6} MB  {}{note}",
                        b.created.replace('T', " "),
                        b.size / 1_000_000,
                        b.backup.display()
                    );
                }
                println!("{} backup(s)", list.len());
            }
        }
        Command::Restore { name, apply } => {
            let session = open(cli.catalog.as_ref())?;
            let Some(chosen) = session.source.backups().into_iter().find(|b| b.backup.ends_with(&name)) else {
                anyhow::bail!("no backup {name:?}; `backups` lists them");
            };
            println!("restore   {} ({})", chosen.backup.display(), chosen.created.replace('T', " "));
            println!("over      {}", session.source.catalog_path().display());
            if !apply {
                println!("dry run: nothing changed; add --apply to restore (the catalog is backed up first)");
                return Ok(());
            }
            let file = chosen.backup.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let before = PhotoSource::restore(&session.source, &file)?;
            println!("restored; the catalog it replaced is kept as {}", before.path);
            sync(&session, true)?;
        }
        Command::Status => {
            let session = open(cli.catalog.as_ref())?;
            let info = session.source.info();
            println!("catalog   {}", session.source.catalog_path().display());
            println!("source    {} (schema {})", info.name, info.version);
            println!("writes    {:?}", session.source.write_status());
            println!("library   {} photos in {}", session.library.len(), session.library.dir().display());
        }
        Command::Find { select, sort, limit, offset, print_ids, json, no_sync } => {
            let session = open(cli.catalog.as_ref())?;
            if !no_sync {
                sync(&session, print_ids || json)?;
            }
            let filter = select.to_filter(&session.library)?;
            let options = QueryOptions { sort: Sort::parse(&sort)?, limit, offset, ..QueryOptions::default() };
            let result = session.library.query(&filter, &options)?;
            if print_ids {
                for row in &result.rows {
                    if let Some(id) = session.library.photo_id(*row) {
                        println!("{}", id.ext);
                    }
                }
            } else if json {
                let photos: Vec<_> = result
                    .rows
                    .iter()
                    .filter_map(|r| session.library.photo(*r).transpose())
                    .collect::<Result<_, _>>()?;
                println!("{}", serde_json::to_string_pretty(&photos)?);
            } else {
                for row in &result.rows {
                    if let Some(p) = session.library.photo(*row)? {
                        let date = p
                            .captured_at
                            .and_then(chrono::DateTime::from_timestamp_millis)
                            .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_default();
                        let stars = p.rating.map_or(String::new(), |r| "★".repeat(r.into()));
                        let kws: Vec<String> = p.keywords.iter().map(|k| k.to_pipe()).collect();
                        println!("{:>7}  {}  {:<16}  {:<5}  {}", p.id.ext, date, p.file.name, stars, kws.join(", "));
                    }
                }
                eprintln!("{} of {} in {:.1} ms", result.rows.len(), result.total, result.micros as f64 / 1000.0);
            }
        }
        Command::Count { select, no_sync } => {
            let session = open(cli.catalog.as_ref())?;
            if !no_sync {
                sync(&session, true)?;
            }
            let filter = select.to_filter(&session.library)?;
            let result = session.library.query(&filter, &QueryOptions { limit: Some(0), ..QueryOptions::default() })?;
            println!("{}", result.total);
        }
        Command::Trips { select, home, gap_hours, json, no_sync } => {
            let session = open(cli.catalog.as_ref())?;
            if !no_sync {
                sync(&session, false)?;
            }
            let library = &session.library;
            if home.is_some() || gap_hours.is_some() {
                let mut settings = library.trip_settings()?;
                if let Some(home) = home {
                    settings.home = match home.trim() {
                        "none" | "" => None,
                        text => {
                            let (country, city) = match text.split_once('|') {
                                Some((country, city)) => (country, Some(city.to_owned())),
                                None => (text, None),
                            };
                            c1index::trips::Home { country: Some(country.to_owned()), city }.normalised()
                        }
                    };
                }
                if let Some(hours) = gap_hours {
                    settings.gap_hours = hours;
                }
                library.set_trip_settings(&settings)?;
            }
            let filter = select.to_filter(library)?;
            let started = Instant::now();
            let list = library.trips(&filter)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
                return Ok(());
            }
            let Some(home) = &list.settings.home else {
                let guess = list.home_guess.as_ref().map_or(String::new(), |h| {
                    format!(
                        " (photographed on most days: --home '{}|{}')",
                        h.country.as_deref().unwrap_or(""),
                        h.city.as_deref().unwrap_or("")
                    )
                });
                println!("No home set, so no trips are found{guess}.");
                return Ok(());
            };
            let day = |ms: i64| {
                chrono::DateTime::from_timestamp_millis(ms)
                    .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
                    .unwrap_or_default()
            };
            for row in &list.trips {
                let t = &row.trip;
                let kind = format!("{:?}", t.kind).to_lowercase();
                let name = t.name.as_deref().unwrap_or(if row.label.is_empty() { "(no place)" } else { &row.label });
                println!(
                    "{:>4}  {:<9}  {} .. {}  {:>5} of {:>5}  {name}",
                    t.id,
                    kind,
                    day(t.start),
                    day(t.end),
                    row.count,
                    row.photos
                );
            }
            println!(
                "{} trips, home {} ({:.1} ms)",
                list.trips.len(),
                [home.city.as_deref(), home.country.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(", "),
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        Command::Facets { select, dims, top, no_sync } => {
            let session = open(cli.catalog.as_ref())?;
            if !no_sync {
                sync(&session, false)?;
            }
            let filter = select.to_filter(&session.library)?;
            let dims = parse_dims(&dims)?;
            let options = QueryOptions { limit: Some(0), facets: dims.clone(), ..QueryOptions::default() };
            let result = session.library.query(&filter, &options)?;
            let names: std::collections::HashMap<String, String> =
                session.library.collections()?.into_iter().map(|c| (c.id.0, c.path)).collect();
            println!("{} photos ({:.1} ms)", result.total, result.micros as f64 / 1000.0);
            for dim in dims {
                println!("\n{dim:?}");
                for (value, n) in result.facets[&dim].iter().take(top) {
                    let label = if dim == Dim::Collection { names.get(value).unwrap_or(value) } else { value };
                    println!("  {n:>7}  {label}");
                }
            }
        }
    }
    Ok(())
}
