//! How close do rendered previews come to what Capture One itself exports? Point it at a
//! folder of sRGB JPEGs exported from Capture One, named after their images
//! (`DSCF1234.jpg`); each is compared, at its own size, with the thumbnail and with the
//! preview rendered two ways. Mean absolute difference per channel, out of 255.
//!
//! File names repeat across cards and years, so of the photos with that name the one
//! whose thumbnail looks most like the export is taken. An export may also be framed
//! tighter than the cached renders (a crop made since, or another variant): it is found
//! inside the thumbnail by scale and offset, and only that window is compared. An export
//! of a second variant (`DSCF1234_1.jpg`, `untitled-0066-2.jpg`) is looked up under its
//! image's name when no file has the full one.
//!
//! `--dump <dir>` also writes, for each export, a sheet of the export, the thumbnail's
//! window, the preview as shown and their difference (four times amplified), and prints
//! how the colours were matched and where the preview is off: the mean signed error per
//! channel (preview − export) in the export's shadows, midtones and highlights.
use std::path::{Path, PathBuf};

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

/// `a` at `b`'s size.
fn sized(a: &RgbImage, b: &RgbImage) -> RgbImage {
    image::imageops::resize(a, b.width(), b.height(), image::imageops::FilterType::Triangle)
}

/// For shadows, midtones and highlights: the mean signed error per channel and the share
/// of pixels (percent).
type Bands = [([f64; 3], f64); 3];

/// Mean signed error per channel (`a` − `b`, out of 255) and share of pixels, for the
/// pixels of `b` (those `valid` says, or all) whose brightness is below 64, 64–191, and
/// 192 or more.
fn bands(a: &RgbImage, b: &RgbImage, valid: Option<&[bool]>) -> Bands {
    let a = sized(a, b);
    let mut out = [([0.0f64; 3], 0u64); 3];
    let all = vec![true; (b.width() * b.height()) as usize];
    let valid = valid.unwrap_or(&all);
    for ((p, q), _) in a.pixels().zip(b.pixels()).zip(valid).filter(|(_, ok)| **ok) {
        let y = (54 * u32::from(q.0[0]) + 183 * u32::from(q.0[1]) + 19 * u32::from(q.0[2])) >> 8;
        let band = &mut out[usize::from(y >= 64) + usize::from(y >= 192)];
        for c in 0..3 {
            band.0[c] += f64::from(p.0[c]) - f64::from(q.0[c]);
        }
        band.1 += 1;
    }
    let total = valid.iter().filter(|v| **v).count().max(1) as f64;
    out.map(|(sum, n)| (sum.map(|v| v / n.max(1) as f64), n as f64 * 100.0 / total))
}

/// Colour alone: `preview` lined up with `export` at 600 px wide, so that framing does
/// not count, then the mean absolute difference and [`bands`] over the pixels lined up.
fn colour_only(preview: &RgbImage, export: &RgbImage) -> Option<(f64, Bands)> {
    let w = 600.min(export.width());
    let e =
        image::imageops::resize(export, w, export.height() * w / export.width(), image::imageops::FilterType::Triangle);
    let a = c1image::align(&sized(preview, &e), &e)?;
    let keep = |img: &RgbImage| -> RgbImage {
        let mut out = img.clone();
        for (p, ok) in out.pixels_mut().zip(&a.valid) {
            if !ok {
                p.0 = [0, 0, 0];
            }
        }
        out
    };
    let (p, e) = (keep(&a.image), keep(&e));
    let n = a.valid.iter().filter(|v| **v).count().max(1) as f64;
    let sum: u64 = p.as_raw().iter().zip(e.as_raw()).map(|(x, y)| u64::from(x.abs_diff(*y))).sum();
    Some((sum as f64 / (n * 3.0), bands(&p, &e, Some(&a.valid))))
}

/// Brightness at the 1st, 5th, 25th, 50th, 75th, 95th and 99th percentile, and the mean
/// chroma (|R−G| + |B−G|): the tone curve and the saturation, blind to sharpness and to
/// how well two images line up.
fn spread(image: &RgbImage) -> String {
    let mut y: Vec<u32> = image
        .pixels()
        .map(|p| (54 * u32::from(p.0[0]) + 183 * u32::from(p.0[1]) + 19 * u32::from(p.0[2])) >> 8)
        .collect();
    y.sort_unstable();
    let at = |q: f64| y[((y.len() - 1) as f64 * q) as usize];
    let chroma =
        image.pixels().map(|p| f64::from(p.0[0].abs_diff(p.0[1])) + f64::from(p.0[2].abs_diff(p.0[1]))).sum::<f64>()
            / f64::from(image.width() * image.height()).max(1.0);
    format!(
        "{:3} {:3} {:3} {:3} {:3} {:3} {:3}  chroma {chroma:5.1}",
        at(0.01),
        at(0.05),
        at(0.25),
        at(0.5),
        at(0.75),
        at(0.95),
        at(0.99)
    )
}

/// Local contrast at 1024 px wide: the spread (standard deviation, out of 255) of
/// brightness around its neighbourhood, fine (sigma 0.3% of the width, as Structure
/// works) and broad (2%, as Clarity does).
fn detail(image: &RgbImage) -> String {
    let w = 1024u32;
    let h = image.height() * w / image.width();
    let small = image::imageops::resize(image, w, h, image::imageops::FilterType::Triangle);
    let luma = image::GrayImage::from_fn(w, h, |x, y| {
        let p = small.get_pixel(x, y).0;
        image::Luma([((54 * u32::from(p[0]) + 183 * u32::from(p[1]) + 19 * u32::from(p[2])) >> 8) as u8])
    });
    let spread = |sigma: f32| {
        let b = image::imageops::blur(&luma, sigma);
        let n = f64::from(w * h);
        let sum: f64 =
            luma.pixels().zip(b.pixels()).map(|(a, b)| (f64::from(a.0[0]) - f64::from(b.0[0])).powi(2)).sum();
        (sum / n).sqrt()
    };
    format!("detail fine {:5.1} broad {:5.1}", spread(0.003 * w as f32), spread(0.02 * w as f32))
}

/// How the colour fit goes for `r`: the residual of a table fitted at the thumbnail's
/// size, or why none is fitted.
fn fit_report(r: &ImageRef, thumb: &RgbImage) -> anyhow::Result<String> {
    let plain = ImageRef { monochrome: false, look: None, ..r.clone() };
    let base = c1image::render_preview(&plain, Matching::None)?;
    Ok(match c1image::colour_fit(&base, thumb) {
        c1image::ColourFit::Table { table, shift: Some(s) } => {
            format!("table, residual {:.1}, lined up (moved up to {s:.1} px)", table.residual)
        }
        c1image::ColourFit::Table { table, shift: None } => {
            format!("table, residual {:.1}, paired as they are", table.residual)
        }
        c1image::ColourFit::ShapesDiffer => {
            format!(
                "shapes differ ({}x{} vs {}x{}): tones only",
                base.width(),
                base.height(),
                thumb.width(),
                thumb.height()
            )
        }
        c1image::ColourFit::NoFit { residual } => format!("no fit (residual {residual:?}): tones only"),
    })
}

/// How far the turned and straightened preview sits from the thumbnail, at the
/// thumbnail's size: for a 5×5 grid of blocks, the shift (dx, dy in thumbnail pixels)
/// that best lines the block up by normalised cross-correlation.
fn alignment(r: &ImageRef, thumb: &RgbImage) -> anyhow::Result<String> {
    let plain = ImageRef { monochrome: false, look: None, ..r.clone() };
    let base = sized(&c1image::render_preview(&plain, Matching::None)?, thumb);
    let (w, h) = thumb.dimensions();
    let g = |i: &RgbImage| grey(i, w, h);
    let (a, b) = (g(&base), g(thumb));
    let (bw, reach) = (40i64, 10i64);
    let mut out = String::new();
    for gy in 0..5 {
        out.push_str("\n     ");
        for gx in 0..5 {
            let x0 = reach + (i64::from(w) - 2 * reach - bw) * gx / 4;
            let y0 = reach + (i64::from(h) - 2 * reach - bw) * gy / 4;
            let mut best = (f64::NEG_INFINITY, 0, 0);
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0, 0.0, 0.0, 0.0, 0.0);
                    for y in y0..y0 + bw {
                        for x in x0..x0 + bw {
                            let p = f64::from(b[(y * i64::from(w) + x) as usize]);
                            let q = f64::from(a[((y + dy) * i64::from(w) + x + dx) as usize]);
                            sa += q;
                            sb += p;
                            saa += q * q;
                            sbb += p * p;
                            sab += p * q;
                        }
                    }
                    let n = (bw * bw) as f64;
                    let cov = sab - sa * sb / n;
                    let ncc = cov / ((saa - sa * sa / n) * (sbb - sb * sb / n)).sqrt().max(1e-9);
                    if ncc > best.0 {
                        best = (ncc, dx, dy);
                    }
                }
            }
            out.push_str(&format!(" {:+3},{:+3} ({:.2})", best.1, best.2, best.0));
        }
    }
    Ok(out)
}

fn modified(path: &Path) -> String {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| "?".into())
}

/// The export, the thumbnail's window, the preview and |preview − export| × 4, in a 2×2.
fn sheet(export: &RgbImage, thumb: &RgbImage, preview: &RgbImage) -> RgbImage {
    let w = export.width().min(1024);
    let h = export.height() * w / export.width();
    let fit = |i: &RgbImage| image::imageops::resize(i, w, h, image::imageops::FilterType::Triangle);
    let (e, t, p) = (fit(export), fit(thumb), fit(preview));
    let mut d = p.clone();
    for (px, q) in d.pixels_mut().zip(e.pixels()) {
        px.0 = std::array::from_fn(|c| px.0[c].abs_diff(q.0[c]).saturating_mul(4));
    }
    let mut out = RgbImage::new(w * 2, h * 2);
    for (i, img) in [e, t, p, d].iter().enumerate() {
        let i = i as u32;
        image::imageops::replace(&mut out, img, i64::from(w * (i % 2)), i64::from(h * (i / 2)));
    }
    out
}

/// `dscf1234_1` → `dscf1234`, `untitled-0066-2` → `untitled-0066`: the image a second
/// variant's export is named after.
fn image_stem(stem: &str) -> Option<&str> {
    let (head, tail) = stem.rsplit_once(['_', '-'])?;
    (!tail.is_empty() && tail.len() <= 2 && tail.bytes().all(|b| b.is_ascii_digit())).then_some(head)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let folder =
        args.first().ok_or_else(|| anyhow::anyhow!("usage: exportcheck <folder of exports> [--dump <dir>]"))?;
    let dump: Option<PathBuf> =
        args.iter().position(|a| a == "--dump").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    if let Some(d) = &dump {
        std::fs::create_dir_all(d)?;
    }
    let catalog = find_catalog(None)?;
    let dir = data_dir(&catalog);
    let source = CaptureOne::open(&catalog, dir.join("work"))?;
    let library = Library::open(dir.join("library"))?;
    let mut scores: Vec<[f64; 3]> = vec![];
    println!("{:22} {:>9} {:>7} {:>7} {:>8}", "export", "thumbnail", "tones", "colours", "as shown");
    let mut paths: Vec<_> = std::fs::read_dir(folder)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>()?;
    paths.sort();
    for path in paths {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        if !matches!(ext.as_deref(), Some("jpg" | "jpeg")) {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().to_lowercase();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let named = |stem: &str| -> anyhow::Result<Vec<_>> {
            let filter = Filter { filename: Some(format!("{stem}.*")), ..Filter::default() };
            Ok(library.query(&filter, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() })?.rows)
        };
        let mut rows = named(&stem)?;
        if rows.is_empty()
            && let Some(image) = image_stem(&stem)
        {
            rows = named(image)?;
        }
        let export = load(&path)?;
        let mut best: Option<(f64, Window, ImageRef, RgbImage, String)> = None;
        for &row in &rows {
            let id = library.photo_id(row).unwrap();
            let Some(r) = source.image(&id, ImageTier::Preview)? else { continue };
            let Some(thumb) = r.look.as_deref().and_then(|p| load(p).ok()) else { continue };
            let (window, score) = locate(&export, &thumb);
            if best.as_ref().is_none_or(|(s, ..)| score < *s) {
                best = Some((score, window, r, thumb, id.to_string()));
            }
        }
        let Some((closeness, window, r, thumb, id)) = best else {
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
        let shown = |m| -> anyhow::Result<RgbImage> { Ok(c1image::frame(c1image::render_preview(&r, m)?, &r)) };
        let colours = shown(Matching::Colours)?;
        let s = [compare(&thumb), compare(&shown(Matching::Tones)?), compare(&colours)];
        // Framed as Capture One frames it, the preview needs no search: `as shown` close to
        // `colours` means the crop and straightening are right.
        let direct = mae(&colours, &export);
        // A poor best match means the export is of no photo found, or was edited since.
        println!(
            "{name:22} {:9.1} {:7.1} {:7.1} {direct:8.1}   thumbnail window {:.0}% at ({:.2}, {:.2}), best of {} by {closeness:.1}, {id}",
            s[0],
            s[1],
            s[2],
            window.w * 100.0,
            window.x,
            window.y,
            rows.len()
        );
        if let Some(d) = &dump {
            println!("    fit: {}", fit_report(&r, &thumb)?);
            println!("    preview vs thumbnail, shift per block (correlation):{}", alignment(&r, &thumb)?);
            println!(
                "    .cop {}  .cot {}  monochrome {}  turn {}  straighten {:.2}  crop {}",
                modified(&r.path),
                r.look.as_deref().map_or("-".into(), modified),
                r.monochrome,
                r.rotation,
                r.straighten,
                r.crop.is_some()
            );
            // Colour alone, framing taken out: the preview, and the thumbnail for scale.
            // For comparison, the table fitted to pairs as they are, without lining up.
            let unaligned = {
                let plain = ImageRef { monochrome: false, look: None, ..r.clone() };
                let mut base = c1image::render_preview(&plain, Matching::None)?;
                if let Some(t) = c1image::Lut::fit(&sized(&base, &thumb), &thumb) {
                    t.apply(&mut base);
                }
                if r.monochrome || c1image::Look::of(&thumb).grey {
                    c1image::Look::make_grey(&mut base);
                }
                c1image::frame(base, &r)
            };
            let before = colour_only(&unaligned, &export).map_or(f64::NAN, |c| c.0);
            match (colour_only(&colours, &export), colour_only(&window.crop(&thumb), &export)) {
                (Some((pm, pb)), Some((tm, tb))) => {
                    println!("    colour only: preview {pm:.1} (unaligned fit {before:.1}), thumbnail {tm:.1}");
                    for (i, band) in ["shadows", "midtones", "highlights"].iter().enumerate() {
                        let f = |(e, s): ([f64; 3], f64)| format!("{s:4.0}% {:+6.1} {:+6.1} {:+6.1}", e[0], e[1], e[2]);
                        println!("    {band:10} preview {}   thumbnail {}", f(pb[i]), f(tb[i]));
                    }
                }
                _ => println!("    colour only: could not line up with the export"),
            }
            println!("    brightness   p1  p5 p25 p50 p75 p95 p99");
            println!("    export     {}  {}", spread(&export), detail(&export));
            println!("    thumbnail  {}  {}", spread(&window.crop(&thumb)), detail(&window.crop(&thumb)));
            println!("    preview    {}  {}", spread(&colours), detail(&colours));
            println!(
                "    local contrast: clarity {:.0} structure {:.0}",
                r.local_contrast.clarity, r.local_contrast.structure
            );
            sheet(&export, &window.crop(&thumb), &sized(&colours, &export)).save(d.join(format!("{stem}.png")))?;
        }
        scores.push(s);
    }
    let col = |i: usize| median(scores.iter().map(|s| s[i]).collect());
    println!("{:22} {:9.1} {:7.1} {:7.1}", format!("median of {}", scores.len()), col(0), col(1), col(2));
    Ok(())
}
