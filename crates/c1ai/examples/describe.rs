//! Describes JPEGs with Claude Code signed in with a Claude plan, with the Claude API
//! when `ANTHROPIC_API_KEY` is set and `--api` given, or with Gemini through agy
//! (`--agy`; `--sheet` puts the photos on one contact sheet): a live check of a provider
//! outside the app.
//!
//! `cargo run -p c1ai --example describe -- photo.jpg [more.jpg…] [--model=ID] [--api | --agy [--sheet]]`

use c1ai::agy::{self, Agy};
use c1ai::claude::{Claude, Options};
use c1ai::claude_code::{self, ClaudeCode};
use c1ai::{Brief, PhotoRequest, RunRecord, SheetRequest, VisionProvider, Vocabulary};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let (api, gemini, sheet) = (flag("--api"), flag("--agy"), flag("--sheet"));
    let model = args.iter().find_map(|a| a.strip_prefix("--model=")).map(str::to_string);
    let paths: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if paths.is_empty() {
        return Err("usage: describe photo.jpg [more.jpg…] [--model=ID] [--api | --agy [--sheet]]".into());
    }
    let default = if gemini { agy::DEFAULT_MODEL } else { "claude-haiku-5-5" };
    let options = Options { model: model.unwrap_or_else(|| default.into()), ..Options::default() };
    let brief = Brief { vocabulary: vec!["street".into(), "people".into(), "Hong Kong".into()], ..Default::default() };
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
