//! Does the black & white class agree with Capture One? Every variant's thumbnail is
//! measured and set against the catalog's own black and white switch (`ZBWENABLED`, or a
//! monochrome film curve), for the old rule (mean chroma alone) and the new one (mean
//! chroma and tint spread) at a few spreads.
//!
//! `bwcheck <catalog copy> <catalog folder> <out folder>`: the catalog is read from a copy
//! (a backup), the thumbnails from the live catalog's `Cache\Thumbnails`. Thumbnails the
//! catalog calls colour but the old rule calls black and white are written to the out
//! folder, named by their spread, to look at.
use std::path::{Path, PathBuf};

use c1analysis::{Analysis, ClassSettings};
use rayon::prelude::*;

const SPREADS: [f32; 7] = [2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0];

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (db, dir, out) = (Path::new(&args[1]), Path::new(&args[2]), PathBuf::from(&args[3]));
    std::fs::create_dir_all(&out)?;
    let con = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut st = con.prepare(
        "SELECT v.Z_PK, c.ZBWENABLED, c.ZFILMCURVE FROM ZVARIANT v
           LEFT JOIN ZVARIANTLAYER c ON c.Z_PK = v.ZCOMBINEDSETTINGS",
    )?;
    let variants: Vec<(i64, bool)> = st
        .query_map([], |r| {
            let curve = r.get::<_, Option<String>>(2)?.unwrap_or_default().to_lowercase();
            let mono = r.get::<_, Option<i64>>(1)?.unwrap_or(0) != 0
                || ["acros", "monochrome", "sepia"].iter().any(|k| curve.contains(k));
            Ok((r.get(0)?, mono))
        })?
        .collect::<Result<_, _>>()?;
    let measured: Vec<(i64, bool, PathBuf, Analysis)> = variants
        .par_iter()
        .filter_map(|&(pk, mono)| {
            let path = dir.join("Cache").join("Thumbnails").join(format!("{pk:08}.cot"));
            let img = image::ImageReader::open(&path).ok()?.with_guessed_format().ok()?.decode().ok()?.into_rgb8();
            Some((pk, mono, path, c1analysis::analyse(&img)))
        })
        .collect();
    let d = ClassSettings::default();
    let n_mono = measured.iter().filter(|m| m.1).count();
    println!(
        "{} variants, {} measured: {} black and white in Capture One, {} colour",
        variants.len(),
        measured.len(),
        n_mono,
        measured.len() - n_mono
    );
    let row = |name: String, s: &ClassSettings| {
        let (mut mono_in, mut colour_in) = (0, 0);
        for (_, mono, _, a) in &measured {
            if a.is_black_and_white(s) {
                if *mono { mono_in += 1 } else { colour_in += 1 }
            }
        }
        println!(
            "{name:<22} B&W {:>6}: C1 B&W {:>6} ({:>5.1}% of them), C1 colour {:>6}",
            mono_in + colour_in,
            mono_in,
            100.0 * mono_in as f64 / n_mono.max(1) as f64,
            colour_in
        );
    };
    row("old (chroma only)".into(), &ClassSettings { mono_spread: f32::INFINITY, ..d.clone() });
    for sp in SPREADS {
        row(format!("spread < {sp}"), &ClassSettings { mono_spread: sp, ..d.clone() });
    }
    // Spread distribution of each group the old rule calls B&W.
    let old = ClassSettings { mono_spread: f32::INFINITY, ..d.clone() };
    for want in [true, false] {
        let mut v: Vec<f32> =
            measured.iter().filter(|m| m.1 == want && m.3.is_black_and_white(&old)).map(|m| m.3.tint_spread).collect();
        v.sort_by(f32::total_cmp);
        if v.is_empty() {
            continue;
        }
        let at = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
        println!(
            "spread, C1 {:<6} p5 {:.2} p25 {:.2} p50 {:.2} p75 {:.2} p95 {:.2} p99 {:.2}",
            if want { "B&W" } else { "colour" },
            at(0.05),
            at(0.25),
            at(0.5),
            at(0.75),
            at(0.95),
            at(0.99)
        );
    }
    // Spread of C1 B&W ones the old rule misses (chroma ≥ 4: toned or wrongly flagged).
    let missed = measured.iter().filter(|m| m.1 && !m.3.is_black_and_white(&old)).count();
    println!("C1 B&W not B&W by the old rule: {missed}");
    // A sample of C1-colour, old-rule-B&W thumbnails, spread across their spread.
    let mut odd: Vec<_> = measured.iter().filter(|m| !m.1 && m.3.is_black_and_white(&old)).collect();
    odd.sort_by(|a, b| a.3.tint_spread.total_cmp(&b.3.tint_spread));
    let step = (odd.len() / 60).max(1);
    for m in odd.iter().step_by(step) {
        let name = format!("{:06.2}_c{:.2}_{:08}.jpg", m.3.tint_spread, m.3.chroma, m.0);
        std::fs::copy(&m.2, out.join(name))?;
    }
    // And C1 B&W ones with the highest spread (what a line would cut).
    let mut mono: Vec<_> = measured.iter().filter(|m| m.1 && m.3.is_black_and_white(&old)).collect();
    mono.sort_by(|a, b| b.3.tint_spread.total_cmp(&a.3.tint_spread));
    for m in mono.iter().take(15) {
        let name = format!("mono_{:06.2}_c{:.2}_{:08}.jpg", m.3.tint_spread, m.3.chroma, m.0);
        std::fs::copy(&m.2, out.join(name))?;
    }
    Ok(())
}
