//! How close do rendered previews come to Capture One's own thumbnail? Median absolute
//! difference per channel (0–255) at thumbnail size, for each way of rendering.
//!
//! The colour table is fitted to the thumbnail itself, so scoring it there flatters it.
//! Here it is fitted on half the thumbnail, a checkerboard of 16 px blocks, and every
//! method is scored on the other half only.
//!
//! The preview is framed a little unlike the thumbnail (lens and perspective corrections
//! only the thumbnail has), which costs every method alike where edges do not meet. Where
//! the two can be lined up (`c1image::align`), each render is warped onto the thumbnail
//! before scoring, so the scores measure colour, not framing.
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

/// Mean absolute difference per channel over the held-out blocks that `valid` allows.
fn mae(a: &RgbImage, b: &RgbImage, valid: Option<&[bool]>) -> f64 {
    let (mut sum, mut n) = (0u64, 0u64);
    for (i, (x, y, p)) in b.enumerate_pixels().enumerate() {
        if held_out(x, y) && valid.is_none_or(|v| v[i]) {
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

/// The table fitted to the pairs outside the held-out blocks, applied to `base` (greyed
/// for a black-and-white look); `None` when no table fits.
fn fitted(base: &RgbImage, pairs: impl Iterator<Item = ([u8; 3], [u8; 3])>, grey: bool) -> Option<RgbImage> {
    let lut = Lut::fit_pairs(pairs)?;
    let mut out = base.clone();
    lut.apply(&mut out);
    if grey {
        c1image::Look::make_grey(&mut out);
    }
    Some(out)
}

const METHODS: [&str; 5] = ["plain", "+b&w", "+b&w+tones", "table", "table lined up"];

fn main() -> anyhow::Result<()> {
    let n: usize = std::env::args().nth(1).map_or(120, |s| s.parse().unwrap());
    let catalog = find_catalog(None)?;
    let dir = data_dir(&catalog);
    let source = CaptureOne::open(&catalog, dir.join("work"))?;
    let library = Library::open(dir.join("library"))?;
    let rows = library.query(&Filter::default(), &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows;
    let mut rng = SmallRng::seed_from_u64(7);
    let (mut grey, mut colour): (Vec<[f64; 5]>, Vec<[f64; 5]>) = (vec![], vec![]);
    let (mut t_tones, mut t_colours) = (Duration::ZERO, Duration::ZERO);
    let (mut unaligned, mut unfitted, mut shifts) = (0, 0, vec![]);
    while grey.len() + colour.len() < n {
        let id = library.photo_id(rows[rng.random_range(0..rows.len())]).unwrap();
        let Some(r) = source.image(&id, ImageTier::Preview)? else { continue };
        let Some(look_path) = r.look.clone() else { continue };
        let thumb = image::ImageReader::open(&look_path)?.with_guessed_format()?.decode()?.into_rgb8();
        let is_grey = c1image::Look::of(&thumb).grey || r.monochrome;
        let plain = c1model::ImageRef { monochrome: false, look: None, ..r.clone() };

        let base = c1image::render_preview(&plain, Matching::None)?;
        let start = Instant::now();
        let tones = c1image::render_preview(&r, Matching::Tones)?;
        t_tones += start.elapsed();
        let start = Instant::now();
        let _ = c1image::render_preview(&r, Matching::Colours)?;
        t_colours += start.elapsed();

        let small = to_size(&base, &thumb);
        let aligned = c1image::align(&small, &thumb);
        match &aligned {
            Some(a) => shifts.push(f64::from(a.largest_shift)),
            None => unaligned += 1,
        }
        let pairs_as_they_are = small
            .enumerate_pixels()
            .filter(|(x, y, _)| !held_out(*x, *y))
            .map(|(x, y, p)| (p.0, thumb.get_pixel(x, y).0));
        let table = fitted(&base, pairs_as_they_are, is_grey);
        let lined_up = match &aligned {
            Some(a) => {
                let pairs = a
                    .image
                    .enumerate_pixels()
                    .zip(&a.valid)
                    .filter(|((x, y, _), ok)| **ok && !held_out(*x, *y))
                    .map(|((x, y, p), _)| (p.0, thumb.get_pixel(x, y).0));
                fitted(&base, pairs, is_grey)
            }
            None => table.clone(),
        };
        if table.is_none() {
            unfitted += 1;
        }
        let methods = [
            base.clone(),
            c1image::render_preview(&r, Matching::None)?,
            tones.clone(),
            table.unwrap_or_else(|| tones.clone()),
            lined_up.unwrap_or(tones),
        ];
        let scores = methods.map(|m| {
            let m = to_size(&m, &thumb);
            match &aligned {
                Some(a) => mae(&a.resample(&m), &thumb, Some(&a.valid)),
                None => mae(&m, &thumb, None),
            }
        });
        if is_grey { grey.push(scores) } else { colour.push(scores) }
    }
    for (name, set) in [("grey", &grey), ("colour", &colour)] {
        let cols: Vec<String> = (0..METHODS.len())
            .map(|i| format!("{} {:5.1}", METHODS[i], median(set.iter().map(|s| s[i]).collect())))
            .collect();
        println!("{name:7} n={:3}  {}", set.len(), cols.join("   "));
    }
    let per = |t: Duration| t.as_millis() as usize / n.max(1);
    println!("render: tones {} ms, colours {} ms; {unfitted} fell back to tones", per(t_tones), per(t_colours));
    let mut s = shifts.clone();
    s.sort_by(f64::total_cmp);
    let at = |q: f64| s.get(((s.len().max(1) - 1) as f64 * q) as usize).copied().unwrap_or(f64::NAN);
    println!(
        "lined up {} of {n} (the rest paired as they are); largest shift: median {:.1} px, 90th {:.1}, max {:.1}",
        n - unaligned,
        at(0.5),
        at(0.9),
        at(1.0)
    );
    Ok(())
}
