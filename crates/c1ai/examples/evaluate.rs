//! Measures the local model's keyword suggestions against photos keyworded by hand: a
//! tool for choosing how keywords are scored, not part of the app.
//!
//! A manifest is a file of lines `image path<TAB>keyword,keyword…`: the photo and the
//! keywords a person gave it. Two are needed: photos to test on, and other photos of the
//! same catalog as the reference sample a keyword's usual score is read from.
//!
//! `evaluate embed <manifest>` writes `<manifest>.emb`: each photo's embedding whole and
//! in five crops. `evaluate score <test manifest> <reference manifest> <keywords file>`
//! then compares the ways of scoring. Models are kept in `$C1_MODELS`, or where the app
//! keeps them.

use std::path::{Path, PathBuf};

use c1ai::siglip::{DIM, Embedder, logit};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// A photo whole, then its crops: each 60 % of both sides, at the corners and the centre.
const VIEWS: usize = 6;
const CROP: f32 = 0.6;

fn main() -> Fallible<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["embed", manifest] => embed(Path::new(manifest)),
        ["score", test, reference, keywords] => score(Path::new(test), Path::new(reference), Path::new(keywords)),
        ["learn", test, keywords] => learn(Path::new(test), Path::new(keywords)),
        _ => Err("usage: evaluate embed <manifest> | evaluate score <test> <reference> <keywords file>".into()),
    }
}

fn models_root() -> PathBuf {
    let data = std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("XDG_DATA_HOME")).map(PathBuf::from);
    let data = data
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local").join("share"));
    std::env::var_os("C1_MODELS").map(Into::into).unwrap_or_else(|| data.join("c1studio").join("models"))
}

/// The manifest's lines: a path and the names given to the photo.
fn manifest(path: &Path) -> Fallible<Vec<(String, Vec<String>)>> {
    Ok(std::fs::read_to_string(path)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let (image, names) = line.split_once('\t').unwrap_or((line, ""));
            (image.to_string(), names.split(',').filter(|n| !n.is_empty()).map(str::to_string).collect())
        })
        .collect())
}

fn embeddings_path(manifest: &Path) -> PathBuf {
    manifest.with_extension("emb")
}

fn views(image: &image::RgbImage) -> Vec<image::RgbImage> {
    let (w, h) = image.dimensions();
    let (cw, ch) = ((w as f32 * CROP) as u32, (h as f32 * CROP) as u32);
    let at = |x: u32, y: u32| image::imageops::crop_imm(image, x, y, cw, ch).to_image();
    vec![image.clone(), at(0, 0), at(w - cw, 0), at(0, h - ch), at(w - cw, h - ch), at((w - cw) / 2, (h - ch) / 2)]
}

fn embed(path: &Path) -> Fallible<()> {
    let rows = manifest(path)?;
    let mut embedder = Embedder::open(&models_root())?;
    let mut out: Vec<u8> = Vec::with_capacity(rows.len() * VIEWS * DIM * 4);
    let started = std::time::Instant::now();
    for (i, (image, _)) in rows.iter().enumerate() {
        // A photo that cannot be read is all zeros, and left out when scoring.
        let mut numbers = vec![0f32; VIEWS * DIM];
        if let Ok(pixels) = std::fs::read(image)
            .map_err(|e| e.to_string())
            .and_then(|b| c1ai::siglip::decode(&b).map_err(|e| e.to_string()))
        {
            for (v, view) in views(&pixels).iter().enumerate() {
                numbers[v * DIM..(v + 1) * DIM].copy_from_slice(&embedder.image(view)?);
            }
        } else {
            eprintln!("not read: {image}");
        }
        numbers.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
        if (i + 1) % 100 == 0 {
            println!("{} of {} ({:.0} s)", i + 1, rows.len(), started.elapsed().as_secs_f32());
        }
    }
    std::fs::write(embeddings_path(path), out)?;
    Ok(())
}

fn floats(path: &Path) -> Fallible<Vec<f32>> {
    Ok(std::fs::read(path)?.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect())
}

/// Photos by keywords: a logit for each, row by row.
struct Scores {
    photos: usize,
    keywords: usize,
    of: Vec<f32>,
}

impl Scores {
    fn row(&self, photo: usize) -> &[f32] {
        &self.of[photo * self.keywords..(photo + 1) * self.keywords]
    }
}

/// How a photo's views are put together into one logit per keyword.
#[derive(Clone, Copy, Debug)]
enum Look {
    Whole,
    /// The best of the whole photo and its crops.
    Best,
    /// The whole photo and the best crop, in equal parts.
    Blend,
}

/// `images` holds [`VIEWS`] embeddings a photo, `texts` one a keyword.
fn scores(images: &[f32], texts: &[f32], look: Look) -> Scores {
    let (photos, keywords) = (images.len() / (VIEWS * DIM), texts.len() / DIM);
    let mut of = Vec::with_capacity(photos * keywords);
    for photo in images.as_chunks::<{ VIEWS * DIM }>().0 {
        let views = photo.as_chunks::<DIM>().0;
        for text in texts.as_chunks::<DIM>().0 {
            let whole = logit(&views[0], text);
            let crops = || views[1..].iter().map(|v| logit(v, text)).fold(f32::MIN, f32::max);
            of.push(match look {
                Look::Whole => whole,
                Look::Best => whole.max(crops()),
                Look::Blend => (whole + crops()) / 2.0,
            });
        }
    }
    Scores { photos, keywords, of }
}

/// A keyword's embedding from several phrasings: their mean, at length 1.
fn texts(embedder: &mut Embedder, names: &[String], templates: &[&str]) -> Fallible<Vec<f32>> {
    let mut sum = vec![0f32; names.len() * DIM];
    for template in templates {
        let phrases: Vec<String> = names.iter().map(|n| template.replace("{}", n)).collect();
        let part = embedder.texts(&phrases, &|| false)?;
        sum.iter_mut().zip(&part).for_each(|(s, p)| *s += p);
    }
    for one in sum.as_chunks_mut::<DIM>().0 {
        let length = one.iter().map(|x| x * x).sum::<f32>().sqrt();
        one.iter_mut().for_each(|x| *x /= length);
    }
    Ok(sum)
}

/// A keyword's usual logit over the reference sample, and how far it strays.
#[derive(Clone, Copy)]
struct Usual {
    median: f32,
    /// From the median down to the 16th percentile: the spread of the photos the keyword
    /// does not fit, whatever share of the catalog it does fit.
    low: f32,
    mean: f32,
    deviation: f32,
    p90: f32,
}

fn usual(reference: &Scores) -> Vec<Usual> {
    (0..reference.keywords)
        .map(|k| {
            let mut column: Vec<f32> = (0..reference.photos).map(|p| reference.row(p)[k]).collect();
            column.sort_by(f32::total_cmp);
            let at = |share: f32| column[((column.len() - 1) as f32 * share) as usize];
            let mean = column.iter().sum::<f32>() / column.len() as f32;
            let deviation = (column.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / column.len() as f32).sqrt();
            Usual {
                median: at(0.5),
                low: (at(0.5) - at(0.16)).max(0.05),
                mean,
                deviation: deviation.max(0.05),
                p90: at(0.9),
            }
        })
        .collect()
}

/// How keywords are ranked on a photo.
#[derive(Clone, Copy, Debug)]
enum Rule {
    /// The model's own probability, as now.
    Raw,
    /// Standard deviations above the keyword's mean.
    Z,
    /// The same from the median, in the spread of the lower half.
    ZLow,
    /// Above the keyword's 90th percentile, in logits.
    Over90,
    /// [`Rule::ZLow`] and the raw logit, the logit counting for `0.1 *` this.
    Mix(f32),
}

fn ranked(rule: Rule, logits: &[f32], usual: &[Usual]) -> Vec<f32> {
    logits
        .iter()
        .zip(usual)
        .map(|(&x, u)| match rule {
            Rule::Raw => x,
            Rule::Z => (x - u.mean) / u.deviation,
            Rule::ZLow => (x - u.median) / u.low,
            Rule::Over90 => x - u.p90,
            Rule::Mix(weight) => (x - u.median) / u.low + 0.1 * weight * x,
        })
        .collect()
}

/// The eight best keywords of each photo, best first, with their ranks' values.
fn top(rule: Rule, scores: &Scores, usual: &[Usual], live: &[bool]) -> Vec<Vec<(usize, f32)>> {
    (0..scores.photos)
        .map(|p| {
            if !live[p] {
                return Vec::new();
            }
            let mut all: Vec<(usize, f32)> = ranked(rule, scores.row(p), usual).into_iter().enumerate().collect();
            all.sort_by(|a, b| b.1.total_cmp(&a.1));
            all.truncate(8);
            all
        })
        .collect()
}

/// The least value a suggestion must have for `mean` suggestions a photo, on average.
fn cut(tops: &[Vec<(usize, f32)>], photos: usize, mean: f32) -> f32 {
    let mut values: Vec<f32> = tops.iter().flatten().map(|t| t.1).collect();
    values.sort_by(|a, b| b.total_cmp(a));
    values[((photos as f32 * mean) as usize).min(values.len() - 1)]
}

/// Average precision of a keyword: photos ranked by its logit, against the photos that
/// have it.
fn average_precision(scores: &Scores, truth: &[Vec<usize>], live: &[bool], keyword: usize) -> Option<f32> {
    let mut photos: Vec<(f32, bool)> = (0..scores.photos)
        .filter(|&p| live[p])
        .map(|p| (scores.row(p)[keyword], truth[p].contains(&keyword)))
        .collect();
    let positives = photos.iter().filter(|p| p.1).count();
    if positives < 5 {
        return None;
    }
    photos.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (mut hits, mut sum) = (0f32, 0f32);
    for (i, (_, has)) in photos.iter().enumerate() {
        if *has {
            hits += 1.0;
            sum += hits / (i + 1) as f32;
        }
    }
    Some(sum / positives as f32)
}

/// Keywords from the photos that look most alike among those keyworded by hand: each
/// photo of the manifest is scored from the others, leaving out those whose number is
/// within `apart` of its own (photos of one shoot sit together and share keywords, which
/// a new shoot would not have).
fn learn(test: &Path, keywords: &Path) -> Fallible<()> {
    let names: Vec<String> = std::fs::read_to_string(keywords)?.lines().map(|l| l.trim().to_string()).collect();
    let rows = manifest(test)?;
    let truth: Vec<Vec<usize>> = rows
        .iter()
        .map(|(_, given)| given.iter().filter_map(|g| names.iter().position(|n| n == g)).collect())
        .collect();
    let number = |path: &str| -> i64 {
        Path::new(path).file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse().ok()).unwrap_or(0)
    };
    let numbers: Vec<i64> = rows.iter().map(|(path, _)| number(path)).collect();
    let images = floats(&embeddings_path(test))?;
    let whole: Vec<&[f32]> = images.as_chunks::<{ VIEWS * DIM }>().0.iter().map(|p| &p[..DIM]).collect();
    let live: Vec<bool> = whole.iter().zip(&truth).map(|(e, t)| e.iter().any(|x| *x != 0.0) && !t.is_empty()).collect();
    let photos = live.iter().filter(|l| **l).count();
    let given: usize = truth.iter().zip(&live).filter(|(_, l)| **l).map(|(t, _)| t.len()).sum();
    let watch: Vec<usize> =
        ["cigarette", "smoke", "smoker", "cigar"].iter().filter_map(|w| names.iter().position(|n| n == w)).collect();
    let cosine = |a: &[f32], b: &[f32]| -> f32 { a.iter().zip(b).map(|(x, y)| x * y).sum() };
    let alike: Vec<Vec<(f32, usize)>> = (0..rows.len())
        .map(|p| {
            let mut others: Vec<(f32, usize)> =
                (0..rows.len()).filter(|&o| o != p && live[o]).map(|o| (cosine(whole[p], whole[o]), o)).collect();
            others.sort_by(|a, b| b.0.total_cmp(&a.0));
            others
        })
        .collect();
    println!("{photos} photos, {:.1} keywords a photo", given as f32 / photos as f32);
    println!(
        "{:<6} {:<4} {:<6} | {:>5} {:>5} {:>5} | {:>5} {:>5} | {:>5} {:>5} | watch found at 2",
        "apart", "k", "sharp", "R@5", "R@8", "P@5", "P~2", "R~2", "P~3", "R~3"
    );
    for apart in [0i64, 300, 3000] {
        for k in [10usize, 30] {
            for sharp in [0.02f32, 0.05] {
                let tops: Vec<Vec<(usize, f32)>> = (0..rows.len())
                    .map(|p| {
                        if !live[p] {
                            return Vec::new();
                        }
                        let near: Vec<&(f32, usize)> =
                            alike[p].iter().filter(|(_, o)| (numbers[*o] - numbers[p]).abs() > apart).take(k).collect();
                        let best = near.first().map_or(0.0, |n| n.0);
                        let mut votes = vec![0f32; names.len()];
                        let mut total = 0f32;
                        for (similarity, other) in near {
                            let weight = ((similarity - best) / sharp).exp();
                            total += weight;
                            truth[*other].iter().for_each(|&word| votes[word] += weight);
                        }
                        let mut all: Vec<(usize, f32)> =
                            votes.iter().map(|v| v / total.max(f32::MIN_POSITIVE)).enumerate().collect();
                        all.sort_by(|a, b| b.1.total_cmp(&a.1));
                        all.truncate(8);
                        all.retain(|a| a.1 > 0.0);
                        all
                    })
                    .collect();
                let hits_at = |n: usize| -> usize {
                    tops.iter()
                        .zip(&truth)
                        .map(|(t, has)| t.iter().take(n).filter(|s| has.contains(&s.0)).count())
                        .sum()
                };
                let at = |mean: f32| -> (f32, f32, Vec<(usize, usize)>) {
                    let least = cut(&tops, photos, mean);
                    let (mut said, mut hit) = (0usize, 0usize);
                    let mut found = vec![(0usize, 0usize); watch.len()];
                    for (t, has) in tops.iter().zip(&truth) {
                        let kept: Vec<usize> = t.iter().filter(|s| s.1 >= least).map(|s| s.0).collect();
                        said += kept.len();
                        hit += kept.iter().filter(|k| has.contains(k)).count();
                        for (w, word) in watch.iter().enumerate() {
                            found[w].1 += has.contains(word) as usize;
                            found[w].0 += (has.contains(word) && kept.contains(word)) as usize;
                        }
                    }
                    (hit as f32 / said.max(1) as f32, hit as f32 / given as f32, found)
                };
                let ((p2, r2, found), (p3, r3, _)) = (at(2.0), at(3.0));
                let found: Vec<String> = found.iter().map(|(f, of)| format!("{f}/{of}")).collect();
                println!(
                    "{apart:<6} {k:<4} {sharp:<6} | {:>5.3} {:>5.3} {:>5.3} | {p2:>5.3} {r2:>5.3} | {p3:>5.3} {r3:>5.3} | {}",
                    hits_at(5) as f32 / given as f32,
                    hits_at(8) as f32 / given as f32,
                    hits_at(5) as f32 / (photos * 5) as f32,
                    found.join(" ")
                );
                if (apart, k) == (300, 30) {
                    for least in [0.1f32, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5] {
                        let (mut said, mut hit) = (0usize, 0usize);
                        for (t, has) in tops.iter().zip(&truth) {
                            let kept = t.iter().filter(|s| s.1 >= least);
                            said += kept.clone().count();
                            hit += kept.filter(|s| has.contains(&s.0)).count();
                        }
                        println!(
                            "    share >= {least:.2}: {:.2} a photo, {:.3} right, {:.3} of the real ones found",
                            said as f32 / photos as f32,
                            hit as f32 / said.max(1) as f32,
                            hit as f32 / given as f32
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn score(test: &Path, reference: &Path, keywords: &Path) -> Fallible<()> {
    let names: Vec<String> = std::fs::read_to_string(keywords)?.lines().map(|l| l.trim().to_string()).collect();
    let rows = manifest(test)?;
    let truth: Vec<Vec<usize>> = rows
        .iter()
        .map(|(_, given)| given.iter().filter_map(|g| names.iter().position(|n| n == g)).collect())
        .collect();
    let (test_images, reference_images) = (floats(&embeddings_path(test))?, floats(&embeddings_path(reference))?);
    let live: Vec<bool> = test_images
        .as_chunks::<{ VIEWS * DIM }>()
        .0
        .iter()
        .zip(&truth)
        .map(|(e, t)| e.iter().any(|x| *x != 0.0) && !t.is_empty())
        .collect();
    let photos = live.iter().filter(|l| **l).count();
    let given: usize = truth.iter().zip(&live).filter(|(_, l)| **l).map(|(t, _)| t.len()).sum();
    println!("{photos} photos, {:.1} keywords a photo, {} keywords offered", given as f32 / photos as f32, names.len());
    let watch: Vec<usize> =
        ["cigarette", "smoke", "smoker", "cigar"].iter().filter_map(|w| names.iter().position(|n| n == w)).collect();

    let mut embedder = Embedder::open(&models_root())?;
    let phrasings: [(&str, &[&str]); 5] = [
        ("photo of", &["This is a photo of {}."]),
        ("bare", &["{}"]),
        ("three", &["This is a photo of {}.", "{}", "a photo showing {}"]),
        ("five", &["This is a photo of {}.", "{}", "a photo showing {}", "a photo with {} in it", "{}, a photograph"]),
        ("scene", &["a photo of {}", "a street photo of {}", "a photo with {} in it"]),
    ];
    println!(
        "\n{:<9} {:<6} {:<9} | {:>5} {:>5} | {:>5} {:>5} {:>5} | {:>5} {:>5} | {:>5} {:>5} | watch: AP, found at 3",
        "phrases", "look", "rule", "mAP", "wAP", "R@5", "R@8", "P@5", "P~3", "R~3", "P~5", "R~5"
    );
    for (phrasing, templates) in phrasings {
        let texts = texts(&mut embedder, &names, templates)?;
        for look in [Look::Whole, Look::Best, Look::Blend] {
            let (scores, reference) = (scores(&test_images, &texts, look), scores(&reference_images, &texts, look));
            let usual = usual(&reference);
            let aps: Vec<f32> = (0..names.len()).filter_map(|k| average_precision(&scores, &truth, &live, k)).collect();
            let map = aps.iter().sum::<f32>() / aps.len() as f32;
            let watched: Vec<f32> =
                watch.iter().map(|&k| average_precision(&scores, &truth, &live, k).unwrap_or(0.0)).collect();
            let watch_ap = watched.iter().sum::<f32>() / watched.len().max(1) as f32;
            for rule in [Rule::Raw, Rule::Z, Rule::ZLow, Rule::Over90, Rule::Mix(1.0), Rule::Mix(3.0)] {
                let tops = top(rule, &scores, &usual, &live);
                let hits_at = |n: usize| -> usize {
                    tops.iter()
                        .zip(&truth)
                        .map(|(t, has)| t.iter().take(n).filter(|s| has.contains(&s.0)).count())
                        .sum()
                };
                let (r5, r8) = (hits_at(5) as f32 / given as f32, hits_at(8) as f32 / given as f32);
                let p5 = hits_at(5) as f32 / (photos * 5) as f32;
                let at = |mean: f32| -> (f32, f32, Vec<(usize, usize)>) {
                    let least = cut(&tops, photos, mean);
                    let (mut said, mut hit) = (0usize, 0usize);
                    let mut found = vec![(0usize, 0usize); watch.len()];
                    for (t, has) in tops.iter().zip(&truth) {
                        let kept: Vec<usize> = t.iter().filter(|s| s.1 >= least).map(|s| s.0).collect();
                        said += kept.len();
                        hit += kept.iter().filter(|k| has.contains(k)).count();
                        for (w, k) in watch.iter().enumerate() {
                            found[w].1 += has.contains(k) as usize;
                            found[w].0 += (has.contains(k) && kept.contains(k)) as usize;
                        }
                    }
                    (hit as f32 / said.max(1) as f32, hit as f32 / given as f32, found)
                };
                let ((p3, r3, found), (p5m, r5m, _)) = (at(3.0), at(5.0));
                let found: Vec<String> = found.iter().map(|(f, of)| format!("{f}/{of}")).collect();
                println!(
                    "{phrasing:<9} {:<6} {:<9} | {:>5.3} {:>5.3} | {:>5.3} {:>5.3} {:>5.3} | {:>5.3} {:>5.3} | {:>5.3} {:>5.3} | {}",
                    format!("{look:?}"),
                    format!("{rule:?}"),
                    map,
                    watch_ap,
                    r5,
                    r8,
                    p5,
                    p3,
                    r3,
                    p5m,
                    r5m,
                    found.join(" ")
                );
            }
        }
    }
    Ok(())
}
