//! Describes one JPEG with Claude Code signed in with a Claude plan, or with the Claude
//! API when `ANTHROPIC_API_KEY` is set and `--api` given: a live check of a provider
//! outside the app.
//!
//! `cargo run -p c1ai --example describe -- photo.jpg [model] [--api]`

use c1ai::claude::{Claude, Options};
use c1ai::claude_code::{self, ClaudeCode};
use c1ai::{Brief, PhotoRequest, VisionProvider, Vocabulary};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let api = args.iter().any(|a| a == "--api");
    let mut rest = args.iter().filter(|a| *a != "--api");
    let path = rest.next().ok_or("usage: describe photo.jpg [model] [--api]")?;
    let model = rest.next().cloned().unwrap_or_else(|| "claude-haiku-5-5".into());
    let options = Options { model, ..Options::default() };
    let brief = Brief { vocabulary: vec!["street".into(), "people".into(), "Hong Kong".into()], ..Default::default() };
    let vocabulary = Vocabulary::new([("street", 10), ("people", 5), ("places|China|Hong Kong", 3)]);
    let provider: Box<dyn VisionProvider> = if api {
        let key = c1ai::keys::get(&c1ai::keys::ANTHROPIC)?.ok_or("no API key")?;
        Box::new(Claude::new(key, options, &brief, vocabulary)?)
    } else {
        let program = claude_code::find().ok_or("Claude Code is not installed")?;
        println!("{:?}", claude_code::status(&program)?);
        Box::new(ClaudeCode::new(program, options, &brief, vocabulary)?)
    };
    let photo = PhotoRequest { jpeg: std::fs::read(path)?, context: format!("Catalog data: {path}") };
    let reply = provider.describe(&photo)?;
    println!("{:#?}", reply.suggestion);
    println!("warnings: {:?}", reply.warnings);
    let r = &reply.record;
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
    Ok(())
}
