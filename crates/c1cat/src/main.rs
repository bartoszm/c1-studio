//! `c1cat`: the c1-studio library from the command line.

mod selector;

use std::path::PathBuf;

use anyhow::Context;
use c1index::{Dim, Library, QueryOptions, Sort};
use c1model::PhotoSource;
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
        .map(|s| serde_json::from_value(serde_json::Value::String(s.to_owned())).with_context(|| format!("unknown dimension {s:?}")))
        .collect()
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
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
            let options = QueryOptions { sort: Sort::parse(&sort)?, limit, offset, facets: vec![] };
            let result = session.library.query(&filter, &options)?;
            if print_ids {
                for row in &result.rows {
                    if let Some(id) = session.library.photo_id(*row) {
                        println!("{}", id.ext);
                    }
                }
            } else if json {
                let photos: Vec<_> = result.rows.iter().filter_map(|r| session.library.photo(*r).transpose()).collect::<Result<_, _>>()?;
                println!("{}", serde_json::to_string_pretty(&photos)?);
            } else {
                for row in &result.rows {
                    if let Some(p) = session.library.photo(*row)? {
                        let date = p
                            .captured_at
                            .and_then(chrono::DateTime::from_timestamp_millis)
                            .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_default();
                        let stars = p.rating.map_or(String::new(), |r| "â˜…".repeat(r.into()));
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
