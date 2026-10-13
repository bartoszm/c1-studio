//! How close do rendered previews come to Capture One's own thumbnail? Median absolute
//! difference per channel (0–255) at thumbnail size, for each way of rendering.
//!
//! The colour table is fitted to the thumbnail itself, so scoring it there flatters it.
//! Here it is fitted on half the thumbnail, a checkerboard of 16 px blocks, and every
//! method is scored on the other half only.
use std::time::{Duration, Instant};

use c1image::{Lut, Matching, RgbImage};
use c1index::{Filter, Library, QueryOptions, Sort};
use c1model::{ImageTier, PhotoSource};
use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};
use source_c1::{CaptureOne, data_dir, find_catalog};

const BLOCK: u32 = 16;

fn held_out(x: u32, y: u32) -> bool {
    (x / BLOCK + y / BLOCK) % 2 == 1
}

fn to_size(a: &RgbImage, b: &RgbImage) -> RgbImage {
    image::imageops::resize(a, b.width(), b.height(), image::imageops::FilterType::Triangle)
}

/// Mean absolute difference per channel over the held-out blocks.
fn mae(a: &RgbImage, b: &RgbImage) -> f64 {
    let a = to_size(a, b);
    let (mut sum, mut n) = (0u64, 0u64);
    for (x, y, p) in b.enumerate_pixels() {
        if held_out(x, y) {
            let q = a.get_pixel(x, y);
            sum += (0..3).map(|c| u64::from(p.0[c].abs_diff(q.0[c]))).sum::<u64>();
            n += 3;
        }
    }
    sum as f64 / n.max(1) as f64
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
}

fn main() -> anyhow::Result<()> {
    let n: usize = std::env::args().nth(1).map_or(120, |s| s.parse().unwrap());
    let catalog = find_catalog(None)?;
    let dir = data_dir(&catalog);
    let source = CaptureOne::open(&catalog, dir.join("work"))?;
    let library = Library::open(dir.join("library"))?;
    let rows = library.query(&Filter::default(), &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows;
    let mut rng = SmallRng::seed_from_u64(7);
    let (mut grey, mut colour): (Vec<[f64; 4]>, Vec<[f64; 4]>) = (vec![], vec![]);
    let (mut t_tones, mut t_colours, mut unfitted) = (Duration::ZERO, Duration::ZERO, 0);
    while grey.len() + colour.len() < n {
        let id = library.photo_id(rows[rng.random_range(0..rows.len())]).unwrap();
        let Some(r) = source.image(&id, ImageTier::Preview)? else { continue };
        let Some(look_path) = r.look.clone() else { continue };
        let thumb = image::ImageReader::open(&look_path)?.with_guessed_format()?.decode()?.into_rgb8();
        let is_grey = c1image::Look::of(&thumb).grey || r.monochrome;
        let plain = c1model::ImageRef { monochrome: false, look: None, ..r.clone() };
        let shown = |img| c1image::rotate(img, r.rotation);

        let base = shown(c1image::render_preview(&plain, Matching::None)?);
        let start = Instant::now();
        let tones = shown(c1image::render_preview(&r, Matching::Tones)?);
        t_tones += start.elapsed();
        let start = Instant::now();
        let _ = c1image::render_preview(&r, Matching::Colours)?;
        t_colours += start.elapsed();

        // The colour table, fitted on the blocks that are not scored.
        let small = to_size(&base, &thumb);
        let pairs = small
            .enumerate_pixels()
            .filter(|(x, y, _)| !held_out(*x, *y))
            .map(|(x, y, p)| (p.0, thumb.get_pixel(x, y).0));
        let fitted = match Lut::fit_pairs(pairs) {
            Some(lut) => {
                let mut out = base.clone();
                lut.apply(&mut out);
                if is_grey {
                    c1image::Look::make_grey(&mut out);
                }
                out
            }
            None => {
                unfitted += 1;
                tones.clone()
            }
        };
        let scores = [
            mae(&base, &thumb),
            mae(&shown(c1image::render_preview(&r, Matching::None)?), &thumb),
            mae(&tones, &thumb),
            mae(&fitted, &thumb),
        ];
        if is_grey { grey.push(scores) } else { colour.push(scores) }
    }
    for (name, set) in [("grey", &grey), ("colour", &colour)] {
        let col = |i: usize| median(set.iter().map(|s| s[i]).collect());
        println!(
            "{name:7} n={:3}  plain {:5.1}   +b&w {:5.1}   +b&w+tones {:5.1}   colour table {:5.1}",
            set.len(),
            col(0),
            col(1),
            col(2),
            col(3)
        );
    }
    let per = |t: Duration| t.as_millis() as usize / n.max(1);
    println!("render: tones {} ms, colour table {} ms; {unfitted} fell back to tones", per(t_tones), per(t_colours));
    Ok(())
}
