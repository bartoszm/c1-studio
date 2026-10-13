//! How close do rendered previews come to what Capture One itself exports? Point it at a
//! folder of sRGB JPEGs exported from Capture One, named after their images
//! (`DSCF1234.jpg`); each is compared, at its own size, with the thumbnail and with the
//! preview rendered two ways. Mean absolute difference per channel, out of 255.
//!
//! File names repeat across cards and years, so of the photos with that name the one
//! whose thumbnail looks most like the export is taken. An export may also be framed
//! tighter than the cached renders (a crop made since, or another variant): it is found
//! inside the thumbnail by scale and offset, and only that window is compared.
use std::path::Path;

use c1image::{Matching, RgbImage};
use c1index::{Filter, Library, QueryOptions, Sort};
use c1model::{ImageRef, ImageTier, PhotoSource};
use source_c1::{CaptureOne, data_dir, find_catalog};

fn mae(a: &RgbImage, b: &RgbImage) -> f64 {
    let a = image::imageops::resize(a, b.width(), b.height(), image::imageops::FilterType::Triangle);
    let sum: u64 = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| u64::from(x.abs_diff(*y))).sum();
    sum as f64 / b.as_raw().len() as f64
}

fn load(path: &Path) -> anyhow::Result<RgbImage> {
    Ok(image::ImageReader::open(path)?.with_guessed_format()?.decode()?.into_rgb8())
}

/// A part of an image, in fractions of its width and height.
#[derive(Clone, Copy, Debug)]
struct Window {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Window {
    fn crop(&self, image: &RgbImage) -> RgbImage {
        let (iw, ih) = (f64::from(image.width()), f64::from(image.height()));
        let x = (self.x * iw).round() as u32;
        let y = (self.y * ih).round() as u32;
        let w = ((self.w * iw).round() as u32).clamp(1, image.width() - x);
        let h = ((self.h * ih).round() as u32).clamp(1, image.height() - y);
        image::imageops::crop_imm(image, x, y, w, h).to_image()
    }
}

fn grey(image: &RgbImage, w: u32, h: u32) -> Vec<f32> {
    let small = image::imageops::resize(image, w, h, image::imageops::FilterType::Triangle);
    small.pixels().map(|p| 0.299 * f32::from(p.0[0]) + 0.587 * f32::from(p.0[1]) + 0.114 * f32::from(p.0[2])).collect()
}

/// Where `export` sits inside `thumb`, and how far apart they are there (out of 255).
/// Scales from the whole thumbnail down to half of it, searched at a third of the
/// thumbnail's size and refined at full size.
fn locate(export: &RgbImage, thumb: &RgbImage) -> (Window, f64) {
    let (tw, th) = thumb.dimensions();
    let aspect = f64::from(export.height()) / f64::from(export.width());
    let search = |level: u32, scales: Vec<f64>, near: Option<(i64, i64)>, reach: i64| {
        let (gw, gh) = (tw / level, th / level);
        let base = grey(thumb, gw, gh);
        let mut best = (f64::INFINITY, 0.0, 0i64, 0i64, 0u32, 0u32);
        for s in scales {
            let w = (f64::from(gw) * s).round() as u32;
            let h = (f64::from(w) * aspect).round() as u32;
            if w < 8 || h < 8 || w > gw || h > gh {
                continue;
            }
            let e = grey(export, w, h);
            let (range_x, range_y) = (i64::from(gw - w), i64::from(gh - h));
            let (xs, ys) = match near {
                Some((x, y)) => {
                    ((x - reach).max(0)..=(x + reach).min(range_x), (y - reach).max(0)..=(y + reach).min(range_y))
                }
                None => (0..=range_x, 0..=range_y),
            };
            for oy in ys {
                for ox in xs.clone() {
                    let mut sum = 0.0f64;
                    for y in 0..h as usize {
                        let row = &base[(oy as usize + y) * gw as usize + ox as usize..][..w as usize];
                        let erow = &e[y * w as usize..][..w as usize];
                        sum += row.iter().zip(erow).map(|(a, b)| f64::from((a - b).abs())).sum::<f64>();
                    }
                    let d = sum / f64::from(w * h);
                    if d < best.0 {
                        best = (d, s, ox, oy, w, h);
                    }
                }
            }
        }
        best
    };
    let coarse = search(3, (0..=25).map(|k| 1.0 - f64::from(k) * 0.02).collect(), None, 0);
    let fine_scales = (-6..=6).map(|k| coarse.1 + f64::from(k) * 0.004).filter(|s| *s <= 1.0).collect();
    let (d, _, ox, oy, w, h) = search(1, fine_scales, Some((coarse.2 * 3, coarse.3 * 3)), 5);
    let (tw, th) = (f64::from(tw), f64::from(th));
    (Window { x: ox as f64 / tw, y: oy as f64 / th, w: f64::from(w) / tw, h: f64::from(h) / th }, d)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
}

fn main() -> anyhow::Result<()> {
    let folder = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("usage: exportcheck <folder of exports>"))?;
    let catalog = find_catalog(None)?;
    let dir = data_dir(&catalog);
    let source = CaptureOne::open(&catalog, dir.join("work"))?;
    let library = Library::open(dir.join("library"))?;
    let mut scores: Vec<[f64; 3]> = vec![];
    println!("{:22} {:>9} {:>7} {:>7}", "export", "thumbnail", "tones", "colours");
    let mut paths: Vec<_> = std::fs::read_dir(&folder)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>()?;
    paths.sort();
    for path in paths {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        if !matches!(ext.as_deref(), Some("jpg" | "jpeg")) {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().to_lowercase();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let filter = Filter { filename: Some(format!("{stem}.*")), ..Filter::default() };
        let rows = library.query(&filter, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows;
        let export = load(&path)?;
        let mut best: Option<(f64, Window, ImageRef, RgbImage)> = None;
        for &row in &rows {
            let id = library.photo_id(row).unwrap();
            let Some(r) = source.image(&id, ImageTier::Preview)? else { continue };
            let Some(thumb) = r.look.as_deref().and_then(|p| load(p).ok()) else { continue };
            let (window, score) = locate(&export, &thumb);
            if best.as_ref().is_none_or(|(s, ..)| score < *s) {
                best = Some((score, window, r, thumb));
            }
        }
        let Some((closeness, window, r, thumb)) = best else {
            println!("{name:22} none of {} photos named {stem}.* has a thumbnail", rows.len());
            continue;
        };
        // The preview may be framed unlike the thumbnail (a crop or a perspective fix that
        // only the thumbnail shows), so each render is searched for the export on its own.
        let compare = |image: &RgbImage| {
            let small = image::imageops::resize(
                image,
                thumb.width(),
                image.height() * thumb.width() / image.width(),
                image::imageops::FilterType::Triangle,
            );
            let (w, _) = locate(&export, &small);
            mae(&w.crop(image), &export)
        };
        let shown =
            |m| -> anyhow::Result<RgbImage> { Ok(c1image::rotate(c1image::render_preview(&r, m)?, r.rotation)) };
        let s = [compare(&thumb), compare(&shown(Matching::Tones)?), compare(&shown(Matching::Colours)?)];
        // A poor best match means the export is of no photo found, or was edited since.
        println!(
            "{name:22} {:9.1} {:7.1} {:7.1}   window {:.0}% at ({:.2}, {:.2}), best of {} by {closeness:.1}",
            s[0],
            s[1],
            s[2],
            window.w * 100.0,
            window.x,
            window.y,
            rows.len()
        );
        scores.push(s);
    }
    let col = |i: usize| median(scores.iter().map(|s| s[i]).collect());
    println!("{:22} {:9.1} {:7.1} {:7.1}", format!("median of {}", scores.len()), col(0), col(1), col(2));
    Ok(())
}
