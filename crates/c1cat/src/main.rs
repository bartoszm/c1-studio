//! `c1cat`: the c1-studio library from the command line.

mod bench;
mod selector;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Context;
use c1index::{Dim, Filter, Library, QueryOptions, Sort};
use c1model::{Change, ColorLabel, ImageTier, KeywordPath, Op, Photo, PhotoSource};
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
    /// Add or remove keywords on the matching photos. A dry run unless --apply is given.
    #[command(alias = "kw")]
    Keyword {
        #[command(subcommand)]
        action: KeywordAction,
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
}

fn open(catalog: Option<&PathBuf>) -> anyhow::Result<Session> {
    let path = find_catalog(catalog.map(PathBuf::as_path))?;
    let dir = data_dir(&path);
    let source = CaptureOne::open(&path, dir.join("work"))?;
    let library = Library::open(dir.join("library")).context("opening the library")?;
    Ok(Session { source, library })
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

/// Plans `op` for every matching photo and, with `--apply`, writes it. Capture One must
/// be closed for the write; the plan works either way.
fn edit(
    catalog: Option<&PathBuf>,
    select: &Selector,
    flags: &WriteFlags,
    op: impl Fn(&Photo) -> Op,
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
            changes.push(Change { op: op(&photo), photo: photo.id });
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
            };
            edit(cli.catalog.as_ref(), &select, &write, |_| op.clone())?;
        }
        Command::Bench { photos, keywords, queries, seed, dir, rebuild, dims } => {
            bench::bench(&bench::Config { photos, keywords, queries, seed, dir, rebuild, dims: parse_dims(&dims)? })?;
        }
        Command::Sync => {
            let session = open(cli.catalog.as_ref())?;
            let report = session.library.sync(&session.source)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
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
