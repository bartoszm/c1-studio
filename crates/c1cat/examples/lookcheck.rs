//! How close do rendered previews come to Capture One's own thumbnail? Mean absolute
//! difference per channel (0–255) at thumbnail size, for three ways of rendering.
use c1index::{Filter, Library, QueryOptions, Sort};
use c1model::{ImageTier, PhotoSource};
use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};
use source_c1::{CaptureOne, data_dir, find_catalog};

fn mae(a: &c1image::RgbImage, b: &c1image::RgbImage) -> f64 {
    let a = image::imageops::resize(a, b.width(), b.height(), image::imageops::FilterType::Triangle);
    let sum: u64 = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| u64::from(x.abs_diff(*y))).sum();
    sum as f64 / b.as_raw().len() as f64
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
    let (mut grey, mut colour): (Vec<[f64; 3]>, Vec<[f64; 3]>) = (vec![], vec![]);
    while grey.len() + colour.len() < n {
        let id = library.photo_id(rows[rng.random_range(0..rows.len())]).unwrap();
        let Some(r) = source.image(&id, ImageTier::Preview)? else { continue };
        let Some(look_path) = r.look.clone() else { continue };
        let thumb = image::ImageReader::open(&look_path)?.with_guessed_format()?.decode()?.into_rgb8();
        let is_grey = c1image::Look::of(&thumb).grey || r.monochrome;
        let plain = c1model::ImageRef { monochrome: false, look: None, ..r.clone() };
        let shown = |img| c1image::rotate(img, r.rotation);
        let scores = [
            mae(&shown(c1image::render_preview(&plain, false)?), &thumb),
            mae(&shown(c1image::render_preview(&r, false)?), &thumb),
            mae(&shown(c1image::render_preview(&r, true)?), &thumb),
        ];
        if is_grey { grey.push(scores) } else { colour.push(scores) }
    }
    for (name, set) in [("grey", &grey), ("colour", &colour)] {
        let col = |i: usize| median(set.iter().map(|s| s[i]).collect());
        println!("{name:7} n={:3}  plain {:5.1}   +b&w {:5.1}   +b&w+tones {:5.1}", set.len(), col(0), col(1), col(2));
    }
    Ok(())
}
