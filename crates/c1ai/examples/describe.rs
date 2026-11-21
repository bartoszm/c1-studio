//! Describes JPEGs with Claude Code signed in with a Claude plan, with the Claude API
//! when `ANTHROPIC_API_KEY` is set and `--api` given, or with Gemini through agy
//! (`--agy`; `--sheet` puts the photos on one contact sheet): a live check of a provider
//! outside the app. `--locate` asks where the photos were taken instead.
//!
//! `--local` scores keywords with the model run on this computer instead, calling no one:
//! `--keywords=street,dog,ulica` (or `--keywords-file=`, one a line) are the keywords it
//! picks from, `--threshold=1` the least one must score (%); `--download` fetches the model first (about 1.5 GB). Models are
//! kept in `$C1_MODELS`, or where the app keeps them.
//!
//! `cargo run -p c1ai --example describe -- photo.jpg [more.jpg…] [--model=ID] [--api | --agy [--sheet] | --local] [--locate]`

use c1ai::agy::{self, Agy};
use c1ai::claude::{Claude, Options};
use c1ai::claude_code::{self, ClaudeCode};
use c1ai::{Brief, PhotoRequest, RunRecord, SheetRequest, Task, VisionProvider, Vocabulary};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let (api, gemini, sheet) = (flag("--api"), flag("--agy"), flag("--sheet"));
    let task = if flag("--locate") { Task::Locate } else { Task::Describe };
    let model = args.iter().find_map(|a| a.strip_prefix("--model=")).map(str::to_string);
    let paths: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    #[cfg(feature = "local")]
    if flag("--local") {
        return local(&args, &paths);
    }
    if paths.is_empty() {
        return Err("usage: describe photo.jpg [more.jpg…] [--model=ID] [--api | --agy [--sheet]]".into());
    }
    let default = if gemini { agy::DEFAULT_MODEL } else { "claude-haiku-5-5" };
    let options = Options { model: model.unwrap_or_else(|| default.into()), ..Options::default() };
    let brief =
        Brief { task, vocabulary: vec!["street".into(), "people".into(), "Hong Kong".into()], ..Default::default() };
    let vocabulary = Vocabulary::new([("street", 10), ("people", 5), ("places|China|Hong Kong", 3)]);
    let provider: Box<dyn VisionProvider> = if gemini {
        let program = agy::find().ok_or("agy is not installed")?;
        Box::new(Agy::new(program, options, &brief, vocabulary, sheet))
    } else if api {
        let key = c1ai::keys::get(&c1ai::keys::ANTHROPIC)?.ok_or("no API key")?;
        Box::new(Claude::new(key, options, &brief, vocabulary)?)
    } else {
        let program = claude_code::find().ok_or("Claude Code is not installed")?;
        println!("{:?}", claude_code::status(&program)?);
        Box::new(ClaudeCode::new(program, options, &brief, vocabulary)?)
    };
    if provider.sheets() {
        let cells = paths
            .iter()
            .map(|p| c1image::load_jpeg_file(std::path::Path::new(p.as_str())))
            .collect::<Result<Vec<_>, _>>()?;
        let jpeg = c1image::encode_jpeg(&c1image::contact_sheet(&cells), 85);
        let contexts = paths.iter().map(|p| format!("Catalog data: {p}")).collect();
        let reply = provider.describe_sheet(&SheetRequest { jpeg, contexts, progress: None })?;
        for (path, answer) in paths.iter().zip(&reply.replies) {
            match answer {
                Some((suggestion, warnings)) => println!("{path}: {suggestion:#?}\nwarnings: {warnings:?}"),
                None => println!("{path}: left out"),
            }
        }
        println!("sheet warnings: {:?}", reply.warnings);
        print_record(&reply.record);
        return Ok(());
    }
    for path in paths {
        let photo =
            PhotoRequest { jpeg: std::fs::read(path)?, context: format!("Catalog data: {path}"), progress: None };
        let reply = provider.describe(&photo)?;
        println!("{path}: {:#?}", reply.suggestion);
        println!("warnings: {:?}", reply.warnings);
        print_record(&reply.record);
    }
    Ok(())
}

/// Where the app keeps models: `c1studio/models` under the local data folder.
#[cfg(feature = "local")]
fn models_root() -> std::path::PathBuf {
    let data = std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("XDG_DATA_HOME")).map(Into::into);
    let data: std::path::PathBuf = data.unwrap_or_else(|| {
        std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local").join("share")
    });
    std::env::var_os("C1_MODELS").map(Into::into).unwrap_or_else(|| data.join("c1studio").join("models"))
}

#[cfg(feature = "local")]
fn local(args: &[String], paths: &[&String]) -> Result<(), Box<dyn std::error::Error>> {
    use c1ai::models::SIGLIP2;
    use c1ai::siglip::{DEFAULT_THRESHOLD, Siglip};
    let value = |name: &str| args.iter().find_map(|a| a.strip_prefix(name));
    let root = models_root();
    if args.iter().any(|a| a == "--download") {
        println!("fetching {} ({} MB) into {}", SIGLIP2.name, SIGLIP2.size() >> 20, root.display());
        let shown = std::sync::atomic::AtomicU64::new(0);
        SIGLIP2.download(&root, &Default::default(), &|done, total| {
            let percent = done * 100 / total.max(1);
            if shown.swap(percent, std::sync::atomic::Ordering::SeqCst) != percent && percent % 5 == 0 {
                println!("{percent}%");
            }
        })?;
    }
    // A file of keywords, one a line, or a list of them.
    let file = value("--keywords-file=").map(std::fs::read_to_string).transpose()?;
    let names: Vec<&str> = match &file {
        Some(file) => file.lines().map(str::trim).filter(|l| !l.is_empty()).collect(),
        None => value("--keywords=").unwrap_or("street,people,Hong Kong").split(',').collect(),
    };
    let threshold = value("--threshold=").map(str::parse).transpose()?.unwrap_or(DEFAULT_THRESHOLD);
    let brief = Brief { vocabulary: names.iter().map(|n| n.to_string()).collect(), ..Default::default() };
    let vocabulary = Vocabulary::new(names.iter().map(|n| (*n, 1)));
    let provider = Siglip::new(&root, Options::default(), &brief, vocabulary, threshold)?;
    for path in paths {
        let photo = PhotoRequest { jpeg: std::fs::read(path)?, context: String::new(), progress: None };
        let reply = provider.describe(&photo)?;
        println!("{path}: {} ({} ms)", reply.record.raw, reply.record.millis);
    }
    Ok(())
}

fn print_record(r: &RunRecord) {
    println!(
        "{} {}: {} in, {} out, {} cache write, {} cache read, ${:.4}, {} ms",
        r.provider,
        r.model,
        r.input_tokens,
        r.output_tokens,
        r.cache_write_tokens,
        r.cache_read_tokens,
        r.cost,
        r.millis
    );
}
