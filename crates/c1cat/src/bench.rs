//! `c1cat bench`: the stage-0 speed gate, run on a synthetic library.
//!
//! The library is shaped like a real catalog: photos come in shoots that share a camera,
//! a place and a few keywords; a handful of keywords, cameras and places cover most
//! photos; most photos are unrated. It is fed through the same `Library::sync` a real
//! source uses, then the bench times what the app does most: open the library, filter
//! with per-dimension counts, and search text with filters.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use c1index::{Dim, Filter, Library, Presence, QueryOptions, Range, Sort, TimeRange};
use c1model::memory::MemorySource;
use c1model::*;
use chrono::{Datelike, TimeZone};
use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};

pub struct Config {
    pub photos: usize,
    pub keywords: usize,
    pub queries: usize,
    pub seed: u64,
    pub dir: Option<PathBuf>,
    pub rebuild: bool,
    pub dims: Vec<Dim>,
}

/// The gate from the plan, in milliseconds.
const OPEN_MAX: f64 = 1000.0;
const FILTERS_P95_MAX: f64 = 10.0;
const TEXT_P95_MAX: f64 = 30.0;

/// What a grid asks for: the first screenful, newest first.
const PAGE: usize = 200;
const WARMUP: usize = 20;

const DAY: i64 = 86_400_000;

const SYLLABLES: [&str; 40] = [
    "ka", "ło", "wro", "cła", "rze", "szy", "ść", "ny", "po", "la", "mi", "to", "gó", "ra", "ża", "be", "dą", "si",
    "en", "kie", "mo", "rt", "na", "wi", "ce", "dzi", "ko", "lu", "pa", "źró", "dło", "sta", "re", "mia", "sto", "ję",
    "zio", "ro", "las", "ul",
];

const CAMERAS: [(&str, &str); 12] = [
    ("FUJIFILM", "X-T5"),
    ("FUJIFILM", "X100VI"),
    ("FUJIFILM", "X-Pro2"),
    ("FUJIFILM", "X-H2"),
    ("FUJIFILM", "GFX100S"),
    ("FUJIFILM", "X-E4"),
    ("SONY", "ILCE-7M4"),
    ("Canon", "EOS R5"),
    ("NIKON CORPORATION", "NIKON Z 6_2"),
    ("Apple", "iPhone 15 Pro"),
    ("Panasonic", "DC-S5M2"),
    ("RICOH", "GR III"),
];

const APERTURES: [f64; 9] = [1.2, 1.4, 2.0, 2.8, 4.0, 5.6, 8.0, 11.0, 16.0];
const ISOS: [u32; 10] = [125, 160, 200, 400, 640, 800, 1600, 3200, 6400, 12800];

/// An index below `len` where low indices are far more likely: a few keywords, cameras
/// and places cover most of a real catalog.
fn skewed(rng: &mut SmallRng, len: usize) -> usize {
    let u: f64 = rng.random();
    ((u * u * u * len as f64) as usize).min(len - 1)
}

fn pick<'a, T>(rng: &mut SmallRng, items: &'a [T]) -> &'a T {
    &items[rng.random_range(0..items.len())]
}

/// Made-up words with Polish letters, so text search exercises diacritic folding.
struct Words {
    seen: HashSet<String>,
}

impl Words {
    fn next(&mut self, rng: &mut SmallRng) -> String {
        loop {
            let n = rng.random_range(2..=4);
            let word: String = (0..n).map(|_| *pick(rng, &SYLLABLES)).collect();
            if self.seen.insert(word.clone()) {
                return word;
            }
        }
    }

    fn title(&mut self, rng: &mut SmallRng) -> String {
        let word = self.next(rng);
        let mut chars = word.chars();
        chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
    }
}

struct City {
    country: String,
    state: String,
    name: String,
    lat: f64,
    lon: f64,
}

/// Everything photos and queries draw their values from. Built from the seed alone, so a
/// reused library gets the same vocabulary back.
struct Vocab {
    keywords: Vec<KeywordPath>,
    lenses: Vec<(String, f64)>,
    cities: Vec<City>,
    albums: Vec<Collection>,
    styles: Vec<String>,
    words: Vec<String>,
}

impl Vocab {
    fn new(rng: &mut SmallRng, keyword_count: usize) -> Self {
        let mut words = Words { seen: HashSet::new() };

        // A keyword tree up to four deep; random parents grow a few big branches.
        let mut keywords: Vec<KeywordPath> =
            (0..20.min(keyword_count)).map(|_| KeywordPath(vec![words.next(rng)])).collect();
        while keywords.len() < keyword_count {
            let parent = &keywords[rng.random_range(0..keywords.len())];
            if parent.0.len() < 4 {
                let mut child = parent.clone();
                child.0.push(words.next(rng));
                keywords.push(child);
            }
        }

        let lenses = (0..40)
            .map(|_| {
                let focal = *pick(rng, &[16.0, 18.0, 23.0, 27.0, 33.0, 35.0, 50.0, 56.0, 90.0, 150.0]);
                let aperture = *pick(rng, &APERTURES[..5]);
                (format!("XF{focal}mmF{aperture} R {}", words.next(rng).to_uppercase()), focal)
            })
            .collect();

        let countries: Vec<(String, f64, f64, Vec<String>)> = (0..40)
            .map(|_| {
                let states = (0..rng.random_range(1..=4)).map(|_| words.title(rng)).collect();
                (words.title(rng), rng.random_range(-50.0..65.0), rng.random_range(-120.0..150.0), states)
            })
            .collect();
        let cities = (0..600)
            .map(|_| {
                let (country, lat, lon, states) = &countries[skewed(rng, countries.len())];
                City {
                    country: country.clone(),
                    state: pick(rng, states).clone(),
                    name: words.title(rng),
                    lat: lat + rng.random_range(-3.0..3.0),
                    lon: lon + rng.random_range(-3.0..3.0),
                }
            })
            .collect();

        let groups: Vec<Collection> = (0..30)
            .map(|i| {
                let name = words.title(rng);
                Collection {
                    id: CollectionId(format!("g{i}")),
                    path: name.clone(),
                    name,
                    kind: CollectionKind::Group,
                    parent: None,
                }
            })
            .collect();
        let albums = (0..270)
            .map(|i| {
                let name = words.title(rng);
                let parent = rng.random_bool(0.7).then(|| pick(rng, &groups));
                Collection {
                    id: CollectionId(i.to_string()),
                    path: parent.map_or(name.clone(), |g| format!("{} / {name}", g.name)),
                    name,
                    kind: CollectionKind::Album,
                    parent: parent.map(|g| g.id.clone()),
                }
            })
            .collect();

        Vocab {
            keywords,
            lenses,
            cities,
            albums,
            styles: (0..12).map(|_| words.title(rng)).collect(),
            words: (0..3000).map(|_| words.next(rng)).collect(),
        }
    }

    fn collections(&self) -> Vec<Collection> {
        self.albums.clone()
    }
}

/// What a run of photos taken together shares.
struct Shoot {
    camera: usize,
    lenses: Vec<usize>,
    city: Option<usize>,
    geo: bool,
    keywords: Vec<usize>,
    albums: Vec<usize>,
    style: Option<usize>,
}

impl Shoot {
    fn new(rng: &mut SmallRng, v: &Vocab) -> Self {
        let keywords = if rng.random_bool(0.3) {
            vec![]
        } else {
            (0..rng.random_range(1..=6)).map(|_| skewed(rng, v.keywords.len())).collect()
        };
        Shoot {
            camera: skewed(rng, CAMERAS.len()),
            lenses: (0..rng.random_range(1..=3)).map(|_| skewed(rng, v.lenses.len())).collect(),
            city: rng.random_bool(0.6).then(|| skewed(rng, v.cities.len())),
            geo: rng.random_bool(0.3),
            keywords,
            albums: if rng.random_bool(0.5) {
                vec![]
            } else {
                (0..rng.random_range(1..=3)).map(|_| skewed(rng, v.albums.len())).collect()
            },
            style: rng.random_bool(0.15).then(|| rng.random_range(0..v.styles.len())),
        }
    }
}

fn photos(rng: &mut SmallRng, v: &Vocab, count: usize) -> Vec<Photo> {
    let source = SourceId("bench".into());
    let start =
        chrono::NaiveDate::from_ymd_opt(2008, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis();
    let end = chrono::NaiveDate::from_ymd_opt(2026, 10, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    let mean_gap = (end - start) / count.max(1) as i64;

    let mut out = Vec::with_capacity(count);
    let mut shoot = Shoot::new(rng, v);
    let mut t = start;
    let mut asset = 0u64;
    for n in 0..count {
        // Seconds apart within a shoot; a new shoot every ~50 photos, days later.
        if rng.random_bool(0.02) {
            t += rng.random_range(0..mean_gap * 100);
            shoot = Shoot::new(rng, v);
        } else {
            t += rng.random_range(1_000..30_000);
        }
        // One photo in twenty is a second variant of the file before it.
        let variant_index = u32::from(n > 0 && rng.random_bool(0.05));
        if variant_index == 0 {
            asset += 1;
        }
        let date = chrono::DateTime::from_timestamp_millis(t).unwrap();
        let ext = *pick(rng, &["raf", "raf", "raf", "raf", "raf", "raf", "raf", "raf", "jpg", "dng"]);
        let name = format!("DSCF{:04}.{}", asset % 10_000, ext.to_uppercase());
        let (make, model) = if rng.random_bool(0.9) { CAMERAS[shoot.camera] } else { *pick(rng, &CAMERAS) };
        let (lens, focal) = &v.lenses[*pick(rng, &shoot.lenses)];
        let city = shoot.city.map(|c| &v.cities[c]);

        let mut keywords: Vec<KeywordPath> =
            shoot.keywords.iter().filter(|_| rng.random_bool(0.8)).map(|&k| v.keywords[k].clone()).collect();
        if !shoot.keywords.is_empty() && rng.random_bool(0.2) {
            keywords.push(v.keywords[skewed(rng, v.keywords.len())].clone());
        }
        keywords.sort();
        keywords.dedup();
        let description = rng.random_bool(0.2).then(|| {
            let n = rng.random_range(4..=14);
            (0..n).map(|_| v.words[skewed(rng, v.words.len())].as_str()).collect::<Vec<_>>().join(" ")
        });

        out.push(Photo {
            id: PhotoId::new(&source, n.to_string()),
            asset: AssetId(asset.to_string()),
            variant_index,
            file: FileInfo {
                path: Some(format!(r"H:\fotos\{}\{}\{name}", date.year(), date.format("%Y-%m-%d"))),
                name,
                ext: ext.into(),
                size: Some(rng.random_range(20_000_000..60_000_000)),
            },
            captured_at: Some(t),
            imported_at: Some(t + rng.random_range(0..30 * DAY)),
            exif: Exif {
                camera_make: Some(make.into()),
                camera_model: Some(model.into()),
                lens: Some(lens.clone()),
                iso: Some(*pick(rng, &ISOS)),
                aperture: Some(*pick(rng, &APERTURES)),
                shutter: Some(1.0 / f64::from(*pick(rng, &[30u32, 60, 125, 250, 500, 1000, 4000]))),
                focal: Some(*focal),
                width: Some(7728),
                height: Some(5152),
            },
            rating: match rng.random_range(0..100) {
                0..55 => None,
                55..60 => Some(1),
                60..70 => Some(2),
                70..85 => Some(3),
                85..95 => Some(4),
                _ => Some(5),
            },
            color: rng.random_bool(0.15).then(|| *pick(rng, &ColorLabel::ALL)),
            keywords,
            iptc: Iptc {
                description,
                headline: rng.random_bool(0.05).then(|| v.words[skewed(rng, v.words.len())].clone()),
                creator: Some("Bench".into()),
                ..Iptc::default()
            },
            place: city.map_or_else(Place::default, |c| Place {
                city: Some(c.name.clone()),
                state: Some(c.state.clone()),
                country: Some(c.country.clone()),
                ..Place::default()
            }),
            geo: match city {
                Some(c) if shoot.geo => Some(GeoPoint {
                    lat: c.lat + rng.random_range(-0.05..0.05),
                    lon: c.lon + rng.random_range(-0.05..0.05),
                    alt: None,
                    origin: GeoOrigin::Exif,
                }),
                _ => None,
            },
            collections: shoot.albums.iter().map(|&a| v.albums[a].id.clone()).collect(),
            edit: EditInfo {
                edited: rng.random_bool(0.4),
                styles: shoot.style.map(|s| vec![v.styles[s].clone()]).unwrap_or_default(),
                local_layers: if rng.random_bool(0.1) { rng.random_range(1..4) } else { 0 },
                retouch_layers: 0,
            },
            trashed: rng.random_bool(0.01),
        });
    }
    out
}

/// One constraint of a kind the filter bar offers, which `p` satisfies. False when `p`
/// has no value for that kind.
fn add_constraint(rng: &mut SmallRng, p: &Photo, f: &mut Filter, kind: u8) -> bool {
    let one = |rng: &mut SmallRng, n: usize| rng.random_range(0..n);
    match kind {
        0 => match p.rating {
            Some(r) => {
                f.rating =
                    Some(Presence::Values(Range { min: Some(f64::from(rng.random_range(1..=r))), ..Range::default() }))
            }
            None => return false,
        },
        1 => match p.color {
            Some(c) => f.color = Some(Presence::Values(vec![c])),
            None => return false,
        },
        // The index files years by local time, so ask in local time too.
        2 => match p.captured_at.and_then(|t| chrono::Local.timestamp_millis_opt(t).single()) {
            Some(date) => f.year = Some(date.year()),
            None => return false,
        },
        3 => match &p.exif.camera_model {
            Some(model) => f.camera = Some(model.to_lowercase()),
            None => return false,
        },
        4 => match &p.exif.lens {
            Some(lens) => f.lens = Some(lens.clone()),
            None => return false,
        },
        // A branch: the first one or two levels of one of its keywords.
        5 if !p.keywords.is_empty() => {
            let path = &p.keywords[one(rng, p.keywords.len())].0;
            let depth = rng.random_range(1..=path.len().min(2));
            f.keywords_under.push(path[..depth].join("|"));
        }
        6 if !p.keywords.is_empty() => f.keywords.push(p.keywords[one(rng, p.keywords.len())].to_pipe()),
        7 if !p.collections.is_empty() => f.collections.push(p.collections[one(rng, p.collections.len())].0.clone()),
        8 => match &p.place.country {
            Some(country) => f.country = Some(country.clone()),
            None => return false,
        },
        9 if p.geo.is_some() => f.gps = Some(true),
        10 => f.edited = Some(p.edit.edited),
        11 => match p.exif.iso {
            Some(iso) => {
                f.iso = Some(Range {
                    min: Some(f64::from(*pick(rng, &ISOS[..=ISOS.iter().position(|&i| i == iso).unwrap_or(0)]))),
                    ..Range::default()
                })
            }
            None => return false,
        },
        12 => match p.exif.aperture {
            Some(a) => f.aperture = Some(Range { max: Some(a), ..Range::default() }),
            None => return false,
        },
        13 => match p.captured_at {
            Some(t) => {
                f.captured = Some(TimeRange {
                    from: Some(t - rng.random_range(0..90) * DAY),
                    to: Some(t + rng.random_range(0..90) * DAY),
                })
            }
            None => return false,
        },
        _ => return false,
    }
    true
}

/// Up to `count` constraints that `p` satisfies: someone narrowing down towards a photo
/// they remember. Every such query has at least one hit.
fn filter_around(rng: &mut SmallRng, p: &Photo, count: usize) -> Filter {
    let mut kinds: Vec<u8> = (0..14).collect();
    let mut f = Filter::default();
    let mut added = 0;
    while added < count && !kinds.is_empty() {
        let kind = kinds.swap_remove(rng.random_range(0..kinds.len()));
        if add_constraint(rng, p, &mut f, kind) {
            added += 1;
        }
    }
    f
}

/// Words the photo's text holds (caption, keyword, city); `OR` adds an arbitrary word.
fn text_around(rng: &mut SmallRng, v: &Vocab, p: &Photo) -> String {
    let mut own: Vec<String> = p.keywords.iter().filter_map(|k| k.0.last().cloned()).collect();
    own.extend(p.iptc.description.iter().flat_map(|d| d.split(' ').map(str::to_owned)));
    own.extend(p.place.city.iter().map(|c| c.to_lowercase()));
    if own.is_empty() {
        own.push(p.file.name.to_lowercase());
    }
    let mine = |rng: &mut SmallRng| own[rng.random_range(0..own.len())].clone();
    match rng.random_range(0..10) {
        0..6 => mine(rng),
        6..9 => format!("{} {}", mine(rng), mine(rng)),
        _ => format!("{} OR {}", mine(rng), v.words[skewed(rng, v.words.len())]),
    }
}

/// Random photos that the default filter shows (not in the trash).
fn sample_photos(rng: &mut SmallRng, library: &Library, n: usize) -> anyhow::Result<Vec<Photo>> {
    let rows = library.len();
    let mut out = Vec::with_capacity(n);
    for _ in 0..n * 10 {
        if out.len() == n {
            break;
        }
        if let Some(p) = library.photo(rng.random_range(0..rows))?.filter(|p| !p.trashed) {
            out.push(p);
        }
    }
    if out.len() < n {
        bail!("could not sample {n} photos from the library");
    }
    Ok(out)
}

struct Stats {
    name: &'static str,
    times: Vec<Duration>,
    hits: Vec<usize>,
    gate: Option<f64>,
}

impl Stats {
    fn percentile(&self, p: f64) -> f64 {
        let mut ms: Vec<f64> = self.times.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        ms.sort_by(f64::total_cmp);
        let i = ((p / 100.0) * (ms.len() - 1) as f64).round() as usize;
        ms[i]
    }

    fn passed(&self) -> bool {
        self.gate.is_none_or(|max| self.percentile(95.0) < max)
    }
}

fn run(
    library: &Library,
    name: &'static str,
    filters: &[Filter],
    options: &QueryOptions,
    gate: Option<f64>,
) -> anyhow::Result<Stats> {
    for f in filters.iter().take(WARMUP) {
        library.query(f, options)?;
    }
    let mut stats =
        Stats { name, times: Vec::with_capacity(filters.len()), hits: Vec::with_capacity(filters.len()), gate };
    for f in filters {
        let started = Instant::now();
        let result = library.query(f, options)?;
        stats.times.push(started.elapsed());
        stats.hits.push(result.total);
    }
    Ok(stats)
}

fn disk_size(path: &Path) -> u64 {
    if path.is_file() {
        return path.metadata().map_or(0, |m| m.len());
    }
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => disk_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

fn default_dir(c: &Config) -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("c1studio").join("bench").join(format!("{}-{}-{}", c.photos, c.keywords, c.seed))
}

pub fn bench(c: &Config) -> anyhow::Result<()> {
    if cfg!(debug_assertions) {
        eprintln!("warning: debug build; the gate is meant for `cargo run --release -p c1cat -- bench`");
    }
    if c.keywords < 20 || c.photos == 0 {
        bail!("need at least 1 photo and 20 keywords");
    }
    let dir = c.dir.clone().unwrap_or_else(|| default_dir(c));
    let mut rng = SmallRng::seed_from_u64(c.seed);
    let vocab = Vocab::new(&mut rng, c.keywords);

    let reuse =
        !c.rebuild && dir.exists() && Library::open(&dir).map(|l| l.len() as usize == c.photos).unwrap_or(false);
    if reuse {
        println!("library   reusing {}", dir.display());
    } else {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        let started = Instant::now();
        let photos = photos(&mut rng, &vocab, c.photos);
        let generated = started.elapsed();
        let source = MemorySource::new("bench", photos, vocab.collections());
        let library = Library::open(&dir)?;
        let report = library.sync(&source)?;
        println!(
            "library   built {} photos × {} keywords in {:.1} s (generate {:.1} s, sync {:.1} s)",
            report.added,
            c.keywords,
            started.elapsed().as_secs_f64(),
            generated.as_secs_f64(),
            report.millis as f64 / 1000.0
        );
    }

    let started = Instant::now();
    let library = Library::open(&dir)?;
    let open_ms = started.elapsed().as_secs_f64() * 1000.0;
    let segments = library.searcher().segment_readers().len();
    println!(
        "          {} · store {:.0} MB · index {:.0} MB in {segments} segments",
        dir.display(),
        disk_size(&dir.join("store.redb")) as f64 / 1e6,
        disk_size(&dir.join("index")) as f64 / 1e6
    );

    let mut rng = SmallRng::seed_from_u64(c.seed ^ 0x5eed);
    let n = c.queries.max(WARMUP);
    let anchors = sample_photos(&mut rng, &library, n)?;
    let browse = vec![Filter::default(); n];
    let broad: Vec<Filter> = anchors.iter().map(|p| filter_around(&mut rng, p, 1)).collect();
    let filters: Vec<Filter> = anchors
        .iter()
        .map(|p| {
            let constraints = rng.random_range(3..=5);
            filter_around(&mut rng, p, constraints)
        })
        .collect();
    let texts: Vec<Filter> = anchors
        .iter()
        .map(|p| {
            let constraints = rng.random_range(1..=3);
            let mut f = filter_around(&mut rng, p, constraints);
            f.text = Some(text_around(&mut rng, &vocab, p));
            f
        })
        .collect();

    let options = QueryOptions { sort: Sort::DateDesc, limit: Some(PAGE), offset: 0, facets: c.dims.clone() };
    let classes = [
        run(&library, "browse", &browse, &options, None)?,
        run(&library, "1 filter", &broad, &options, None)?,
        run(&library, "filters", &filters, &options, Some(FILTERS_P95_MAX))?,
        run(&library, "text", &texts, &options, Some(TEXT_P95_MAX))?,
    ];

    let dims: Vec<String> = c.dims.iter().map(|d| format!("{d:?}")).collect();
    println!("queries   first {PAGE} by date, counts over: {}", dims.join(", "));
    println!();
    let open_ok = open_ms < OPEN_MAX;
    println!(
        "{:<9} {:>7} {:>8} {:>8} {:>8} {:>8} {:>10} {:>6}  gate",
        "", "queries", "p50", "p95", "p99", "max", "avg hits", "empty"
    );
    println!(
        "{:<9} {:>7} {:>8.1} {:>8} {:>8} {:>8} {:>10} {:>6}  p100 < {OPEN_MAX:.0} ms {}",
        "open",
        1,
        open_ms,
        "",
        "",
        "",
        "",
        "",
        verdict(open_ok)
    );
    for s in &classes {
        let avg_hits = s.hits.iter().sum::<usize>() as f64 / s.hits.len() as f64;
        let empty = s.hits.iter().filter(|&&h| h == 0).count() as f64 / s.hits.len() as f64 * 100.0;
        let gate = s.gate.map_or("—".to_owned(), |max| format!("p95 < {max:.0} ms {}", verdict(s.passed())));
        println!(
            "{:<9} {:>7} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>10.0} {:>5.0}%  {gate}",
            s.name,
            s.times.len(),
            s.percentile(50.0),
            s.percentile(95.0),
            s.percentile(99.0),
            s.percentile(100.0),
            avg_hits,
            empty
        );
    }
    println!("\ntimes in ms");

    if !open_ok || classes.iter().any(|s| !s.passed()) {
        bail!("the speed gate failed");
    }
    Ok(())
}

fn verdict(ok: bool) -> &'static str {
    if ok { "PASS" } else { "FAIL" }
}
