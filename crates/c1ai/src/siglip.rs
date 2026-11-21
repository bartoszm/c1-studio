//! Keywords from a model run on this computer: SigLIP 2, which puts a photo and a piece
//! of text into one space and says how well they go together. Each of the catalog's own
//! keywords is scored against the photo; those likely enough are suggested. No captions,
//! no ratings, no new keywords, and nothing leaves the computer.
//!
//! The model's files ([`crate::models::SIGLIP2`]) are loaded when the first photo comes,
//! and the keywords' embeddings are worked out then, once: they are kept beside the model
//! (`text-cache-v1.bin`) for the runs after.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::{Tensor, TensorElementType, ValueType};
use tokenizers::Tokenizer;

use crate::claude::Options;
use crate::models::SIGLIP2;
use crate::{AiError, Brief, Failed, PhotoRequest, RawAnswer, Reply, RunRecord, Step, Vocabulary, tell, validate};

/// What a keyword is wrapped in before the model reads it, as the model's authors do.
pub const TEMPLATE: &str = "This is a photo of {}.";

/// Keywords suggested for a photo, at most: the likeliest.
pub const MAX_KEYWORDS: usize = 8;

/// A keyword scoring at least this (%) is suggested, unless the settings say otherwise.
/// The model is sparing with a plain word: on a photo of the sea, "sea" scores 1 to 2,
/// where a sentence describing the photo scores over 90, and a word that does not fit
/// stays under 0.3.
pub const DEFAULT_THRESHOLD: f32 = 1.0;

/// The least and the most a threshold can be (%).
pub const THRESHOLDS: (f32, f32) = (0.1, 99.0);

/// Bumped when the way keywords are scored changes: photos tagged by another version are
/// tagged again when asked.
const VERSION: u32 = 1;

/// The side of the square the image tower takes, the length of an embedding, and the
/// tokens the text tower takes.
const SIZE: u32 = 224;
const DIM: usize = 768;
const CONTEXT: usize = 64;

/// The model's learned scale and bias, which turn a cosine into a probability: read from
/// `logit_scale` (4.7244534, before `exp`) and `logit_bias` of google/siglip2-base-patch16-224,
/// the checkpoint the ONNX export was made from; the export holds the towers only.
const LOGIT_SCALE: f32 = 112.669;
const LOGIT_BIAS: f32 = -16.771_725;

/// Says whether a stored suggestion was made the same way: by this version, with this
/// template and threshold.
pub fn prompt_hash(threshold: f32) -> u64 {
    xxhash_rust::xxh3::xxh3_64(format!("siglip|{VERSION}|{TEMPLATE}|{threshold:.1}|{MAX_KEYWORDS}").as_bytes())
}

/// Text as the model was trained on it: lowercase, no ASCII punctuation, single spaces.
fn canonical(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter_map(|c| match c {
            '_' => Some(' '),
            c if c.is_ascii_punctuation() => None,
            c => Some(c),
        })
        .collect();
    cleaned.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What the model reads for a keyword.
fn phrase(name: &str) -> String {
    canonical(&TEMPLATE.replace("{}", name))
}

/// A JPEG as the image tower takes it: squashed to the square, each channel in -1..1,
/// channel by channel.
fn pixels(jpeg: &[u8]) -> Result<Vec<f32>, AiError> {
    let image =
        image::load_from_memory(jpeg).map_err(|e| AiError::BadReply(format!("the image could not be read: {e}")))?;
    let small = image::imageops::resize(&image.to_rgb8(), SIZE, SIZE, image::imageops::FilterType::CatmullRom);
    let plane = (SIZE * SIZE) as usize;
    let mut out = vec![0f32; 3 * plane];
    for (i, pixel) in small.pixels().enumerate() {
        for channel in 0..3 {
            out[channel * plane + i] = pixel.0[channel] as f32 / 127.5 - 1.0;
        }
    }
    Ok(out)
}

fn normalise(v: &mut [f32]) {
    let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if length > 0.0 {
        v.iter_mut().for_each(|x| *x /= length);
    }
}

/// How likely a text goes with an image, 0..1, from their embeddings (each of length 1).
fn probability(image: &[f32], text: &[f32]) -> f32 {
    let cosine: f32 = image.iter().zip(text).map(|(a, b)| a * b).sum();
    1.0 / (1.0 + (-(cosine * LOGIT_SCALE + LOGIT_BIAS)).exp())
}

/// Of the scored names, those at least `threshold` likely, likeliest first, at most
/// [`MAX_KEYWORDS`].
fn pick(mut scored: Vec<(String, f32)>, threshold: f32) -> Vec<(String, f32)> {
    scored.retain(|(_, p)| *p >= threshold);
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scored.truncate(MAX_KEYWORDS);
    scored
}

fn local(e: impl std::fmt::Display) -> AiError {
    AiError::Local(e.to_string())
}

/// ONNX Runtime allows one environment per process.
fn environment() -> Result<&'static Environment, AiError> {
    static ENVIRONMENT: OnceLock<Result<Environment, String>> = OnceLock::new();
    ENVIRONMENT
        .get_or_init(|| ort::init().with_name("c1studio").build().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| AiError::Local(e.clone()))
}

fn session(path: &Path, threads: usize) -> Result<Session, AiError> {
    Session::builder(environment()?)
        .map_err(local)?
        .with_optimization_level(GraphOptimizationLevel::All)
        .map_err(local)?
        .with_intra_threads(threads)
        .map_err(local)?
        .commit_from_file(path)
        .map_err(|e| AiError::Local(format!("{}: {e}", path.display())))
}

/// The first output of a tower, as an embedding of length 1.
fn embedding(outputs: &ort::session::SessionOutputs) -> Result<Vec<f32>, AiError> {
    let (_, data) = outputs[0].try_extract_tensor::<f32>().map_err(local)?;
    if data.len() != DIM {
        return Err(AiError::Local(format!("the model gave {} numbers, not {DIM}", data.len())));
    }
    let mut v = data.to_vec();
    normalise(&mut v);
    Ok(v)
}

/// The embeddings of phrases worked out before, by the hash of the phrase: a file of
/// records, each a key and [`DIM`] numbers, little-endian. A file cut short loses its
/// last record only.
struct TextCache {
    path: PathBuf,
    known: HashMap<u64, Vec<f32>>,
}

const RECORD: usize = 8 + DIM * 4;

impl TextCache {
    fn open(path: PathBuf) -> Self {
        let bytes = fs::read(&path).unwrap_or_default();
        let known = bytes
            .as_chunks::<RECORD>()
            .0
            .iter()
            .map(|record| {
                let (key, numbers) = record.split_at(8);
                let key = u64::from_le_bytes(key.try_into().expect("8 bytes"));
                (key, numbers.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect())
            })
            .collect();
        TextCache { path, known }
    }

    fn key(phrase: &str) -> u64 {
        xxhash_rust::xxh3::xxh3_64(phrase.as_bytes())
    }

    /// Keeps these too. The file is only ever added to.
    fn add(&mut self, new: Vec<(u64, Vec<f32>)>) -> Result<(), AiError> {
        if new.is_empty() {
            return Ok(());
        }
        let disk = |e: std::io::Error| AiError::Local(format!("{}: {e}", self.path.display()));
        // A record cut short by a crash would shift all after it: start from whole records.
        let whole = fs::metadata(&self.path).map_or(0, |m| m.len() / RECORD as u64 * RECORD as u64);
        let file = fs::OpenOptions::new().create(true).write(true).truncate(false).open(&self.path).map_err(disk)?;
        file.set_len(whole).map_err(disk)?;
        let mut file = fs::OpenOptions::new().append(true).open(&self.path).map_err(disk)?;
        let mut bytes = Vec::with_capacity(new.len() * RECORD);
        for (key, v) in &new {
            bytes.extend_from_slice(&key.to_le_bytes());
            v.iter().for_each(|x| bytes.extend_from_slice(&x.to_le_bytes()));
        }
        file.write_all(&bytes).map_err(disk)?;
        self.known.extend(new);
        Ok(())
    }
}

/// The text tower with its tokenizer: needed only for keywords not embedded before.
struct TextTower {
    session: Session,
    tokenizer: Tokenizer,
    pad: u32,
    eos: u32,
    /// The tower takes 32-bit token ids rather than 64-bit ones.
    narrow: bool,
}

impl TextTower {
    fn open(dir: &Path, threads: usize) -> Result<Self, AiError> {
        let tokenizer = Tokenizer::from_file(dir.join("textual/tokenizer.json")).map_err(local)?;
        let id = |token: &str| {
            tokenizer.token_to_id(token).ok_or_else(|| AiError::Local(format!("the tokenizer has no {token}")))
        };
        let (pad, eos) = (id("<pad>")?, id("<eos>")?);
        let session = session(&dir.join("textual/model.onnx"), threads)?;
        let narrow = matches!(
            session.inputs().first().map(|i| i.dtype()),
            Some(ValueType::Tensor { ty: TensorElementType::Int32, .. })
        );
        Ok(TextTower { session, tokenizer, pad, eos, narrow })
    }

    /// The phrase's tokens as the tower takes them: ended, and padded to [`CONTEXT`].
    fn tokens(&self, phrase: &str) -> Result<Vec<u32>, AiError> {
        let encoding = self.tokenizer.encode(phrase, false).map_err(local)?;
        Ok(padded(encoding.get_ids(), self.eos, self.pad))
    }

    fn embed(&mut self, phrase: &str) -> Result<Vec<f32>, AiError> {
        let tokens = self.tokens(phrase)?;
        let shape = [1usize, CONTEXT];
        let outputs = if self.narrow {
            let ids: Vec<i32> = tokens.iter().map(|&t| t as i32).collect();
            self.session.run(ort::inputs![Tensor::from_array((shape, ids)).map_err(local)?])
        } else {
            let ids: Vec<i64> = tokens.iter().map(|&t| t as i64).collect();
            self.session.run(ort::inputs![Tensor::from_array((shape, ids)).map_err(local)?])
        }
        .map_err(local)?;
        embedding(&outputs)
    }
}

/// Token ids cut to leave room for the end token, ended, then padded to [`CONTEXT`]: the
/// tower reads the phrase's meaning off the last place.
fn padded(ids: &[u32], eos: u32, pad: u32) -> Vec<u32> {
    let mut tokens: Vec<u32> = ids.iter().copied().filter(|&t| t != eos).take(CONTEXT - 1).collect();
    tokens.push(eos);
    tokens.resize(CONTEXT, pad);
    tokens
}

/// The model once loaded: the image tower and every keyword's embedding.
struct Loaded {
    image: Session,
    /// [`DIM`] numbers per name, in the names' order.
    texts: Vec<f32>,
}

pub struct Siglip {
    dir: PathBuf,
    options: Options,
    /// The keywords offered, by name.
    names: Vec<String>,
    vocabulary: Vocabulary,
    threshold: f32,
    prompt: u64,
    /// Loaded when the first photo comes; one photo at a time goes through it, on several
    /// threads of its own.
    loaded: Mutex<Option<Loaded>>,
}

impl Siglip {
    /// `root` is the folder models are kept in; `threshold` the least a keyword must
    /// score to be suggested, in percent. Loads nothing yet.
    pub fn new(
        root: &Path,
        options: Options,
        brief: &Brief,
        vocabulary: Vocabulary,
        threshold: f32,
    ) -> Result<Self, AiError> {
        if !SIGLIP2.installed(root) {
            return Err(AiError::Local("the local model is not downloaded: get it in Settings > AI".into()));
        }
        if brief.task != crate::Task::Describe {
            return Err(AiError::Local("the local model suggests keywords only: it cannot locate photos".into()));
        }
        if brief.vocabulary.is_empty() {
            return Err(AiError::Local(
                "the local model picks from the catalog's keywords, and none are offered".into(),
            ));
        }
        Ok(Siglip {
            dir: SIGLIP2.dir(root),
            options,
            names: brief.vocabulary.clone(),
            vocabulary,
            threshold: threshold.clamp(0.0, THRESHOLDS.1) / 100.0,
            prompt: prompt_hash(threshold),
            loaded: Mutex::new(None),
        })
    }

    fn threads() -> usize {
        std::thread::available_parallelism().map_or(1, |n| n.get() / 2).max(1)
    }

    fn load(&self) -> Result<Loaded, AiError> {
        let threads = Self::threads();
        let mut cache = TextCache::open(self.dir.join("text-cache-v1.bin"));
        let phrases: Vec<String> = self.names.iter().map(|n| phrase(n)).collect();
        let mut missing: Vec<&String> =
            phrases.iter().filter(|p| !cache.known.contains_key(&TextCache::key(p))).collect();
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            // The text tower is over a gigabyte in memory: let go once the keywords are done.
            let mut tower = TextTower::open(&self.dir, threads)?;
            let mut new = Vec::with_capacity(missing.len());
            let embedded = missing.iter().try_for_each(|phrase| {
                if self.options.stopped() {
                    return Err(AiError::Stopped);
                }
                new.push((TextCache::key(phrase), tower.embed(phrase)?));
                Ok(())
            });
            // What was embedded before a stop is kept: the next run starts from there.
            cache.add(new)?;
            embedded?;
        }
        let mut texts = Vec::with_capacity(phrases.len() * DIM);
        for phrase in &phrases {
            texts.extend_from_slice(&cache.known[&TextCache::key(phrase)]);
        }
        Ok(Loaded { image: session(&self.dir.join("visual/model.onnx"), threads)?, texts })
    }

    /// Every keyword offered with how likely it goes with the photo, in the names' order.
    fn scores(&self, photo: &PhotoRequest) -> Result<Vec<(String, f32)>, AiError> {
        let input = pixels(&photo.jpeg)?;
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if self.options.stopped() {
            return Err(AiError::Stopped);
        }
        if loaded.is_none() {
            *loaded = Some(self.load()?);
        }
        let model = loaded.as_mut().expect("loaded above");
        tell(&photo.progress, Step::Sent { attempt: 1 });
        let side = SIZE as usize;
        let tensor = Tensor::from_array(([1usize, 3, side, side], input)).map_err(local)?;
        let outputs = model.image.run(ort::inputs![tensor]).map_err(local)?;
        let image = embedding(&outputs)?;
        drop(outputs);
        Ok(self
            .names
            .iter()
            .zip(model.texts.as_chunks::<DIM>().0)
            .map(|(n, t)| (n.clone(), probability(&image, t)))
            .collect())
    }
}

impl crate::VisionProvider for Siglip {
    fn id(&self) -> &str {
        "local"
    }

    fn model(&self) -> &str {
        SIGLIP2.id
    }

    fn prompt_hash(&self) -> u64 {
        self.prompt
    }

    fn describe(&self, photo: &PhotoRequest) -> Result<Reply, Failed> {
        let started = Instant::now();
        let picked = pick(self.scores(photo)?, self.threshold);
        let raw = RawAnswer {
            keywords: picked.iter().map(|(name, _)| name.clone()).collect(),
            confidence: picked.first().map_or(0.0, |(_, p)| *p as f64),
            ..RawAnswer::default()
        };
        let (mut suggestion, warnings) = validate(&raw, &self.vocabulary);
        // Names sharing a path keep the likeliest one's score.
        suggestion.keyword_scores = suggestion
            .keywords
            .iter()
            .map(|path| {
                let of_path = picked.iter().filter(|(name, _)| self.vocabulary.path(name) == Some(path.as_str()));
                of_path.map(|(_, p)| *p).fold(0.0, f32::max)
            })
            .collect();
        let said: Vec<String> = picked.iter().map(|(name, p)| format!("{name} {:.1}%", p * 100.0)).collect();
        let record = RunRecord {
            provider: "local".into(),
            model: SIGLIP2.id.into(),
            prompt: self.prompt,
            millis: started.elapsed().as_millis() as u64,
            raw: said.join(", "),
            ..RunRecord::default()
        };
        Ok(Reply { suggestion, warnings, record })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VisionProvider;

    #[test]
    fn text_is_read_as_the_model_was_trained_on_it() {
        assert_eq!(canonical("  Hong_Kong,  (night)! "), "hong kong night");
        assert_eq!(phrase("Kraków"), "this is a photo of kraków");
        assert_eq!(phrase("b&w"), "this is a photo of bw");
    }

    #[test]
    fn tokens_end_and_fill_the_context() {
        let tokens = padded(&[5, 6, 1], 1, 0);
        assert_eq!(tokens.len(), CONTEXT);
        assert_eq!(&tokens[..4], &[5, 6, 1, 0], "one end token, then padding");
        let long: Vec<u32> = (10..200).collect();
        let tokens = padded(&long, 1, 0);
        assert_eq!((tokens.len(), tokens[CONTEXT - 1]), (CONTEXT, 1), "cut, the end token kept");
    }

    #[test]
    fn a_photo_is_squashed_to_the_square_in_the_models_range() {
        let white = image::RgbImage::from_pixel(60, 30, image::Rgb([255, 255, 255]));
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(white)
            .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        let input = pixels(&jpeg).unwrap();
        assert_eq!(input.len(), 3 * 224 * 224);
        assert!(input.iter().all(|x| (x - 1.0).abs() < 0.05), "white is 1");
        assert!(matches!(pixels(b"not a jpeg"), Err(AiError::BadReply(_))), "this photo's failure, not the run's");
    }

    #[test]
    fn a_cosine_becomes_a_probability() {
        let unit = |x: f32, y: f32| {
            let mut v = vec![x, y];
            normalise(&mut v);
            v
        };
        let image = unit(1.0, 0.0);
        let same = probability(&image, &unit(1.0, 0.0));
        // A cosine of 0.14: a little under where the model draws its line.
        let near = probability(&image, &unit(0.14, 0.99));
        let apart = probability(&image, &unit(0.0, 1.0));
        assert!(same > 0.99 && same > near && near > apart, "{same} {near} {apart}");
        assert!(near > 0.05 && near < 0.5, "{near}");
        assert!(apart < 1e-6);
        // Where the model draws its line: a cosine of bias over scale is even odds.
        let even = -LOGIT_BIAS / LOGIT_SCALE;
        let at = probability(&[1.0], &[even]);
        assert!((at - 0.5).abs() < 1e-3, "{at}");
    }

    #[test]
    fn the_likeliest_keywords_over_the_threshold_are_picked() {
        let mut scored: Vec<(String, f32)> = (0..12).map(|i| (format!("k{i:02}"), 0.2 + i as f32 * 0.05)).collect();
        scored.push(("low".into(), 0.05));
        scored.push(("tie".into(), 0.2 + 10.0 * 0.05));
        let picked = pick(scored, 0.1);
        assert_eq!(picked.len(), MAX_KEYWORDS);
        assert_eq!(picked[0].0, "k11");
        assert_eq!((picked[1].0.as_str(), picked[2].0.as_str()), ("k10", "tie"), "equal ones by name");
        assert!(picked.iter().all(|(name, _)| name != "low"));
        assert!(pick(vec![("a".into(), 0.09)], 0.1).is_empty());
    }

    #[test]
    fn embeddings_worked_out_before_are_kept_and_a_cut_record_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.bin");
        let mut cache = TextCache::open(path.clone());
        assert!(cache.known.is_empty());
        cache.add(vec![(1, vec![0.5; DIM])]).unwrap();
        // A crash mid-write leaves part of a record behind.
        fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(&[7; 10]).unwrap();
        let mut cache = TextCache::open(path.clone());
        assert_eq!(cache.known.len(), 1);
        cache.add(vec![(2, vec![0.25; DIM])]).unwrap();
        let cache = TextCache::open(path);
        assert_eq!(cache.known.len(), 2);
        assert_eq!((cache.known[&1][0], cache.known[&2][DIM - 1]), (0.5, 0.25));
    }

    #[test]
    fn the_threshold_is_part_of_how_a_photo_was_tagged() {
        assert_ne!(prompt_hash(1.0), prompt_hash(1.5));
        assert_eq!(prompt_hash(1.0), prompt_hash(1.04), "told apart by the tenth");
    }

    #[test]
    fn it_needs_the_model_keywords_and_a_describing_run() {
        let root = tempfile::tempdir().unwrap();
        let brief = Brief { vocabulary: vec!["street".into()], ..Default::default() };
        let missing = Siglip::new(root.path(), Options::default(), &brief, Vocabulary::default(), 1.0);
        assert!(matches!(missing, Err(AiError::Local(m)) if m.contains("not downloaded")));
    }

    /// With the model downloaded (`C1_MODELS` names the folder models are kept in):
    /// `cargo test -p c1ai --release -- --ignored siglip`.
    #[test]
    #[ignore = "needs the downloaded model"]
    fn the_model_scores_keywords_against_a_photo() {
        let root = PathBuf::from(std::env::var("C1_MODELS").expect("C1_MODELS"));
        let names = ["street", "dog", "ulica", "mountains", "plain white"];
        let brief = Brief { vocabulary: names.iter().map(|n| n.to_string()).collect(), ..Default::default() };
        let vocabulary = Vocabulary::new(names.iter().map(|n| (*n, 1)));
        let siglip = Siglip::new(&root, Options::default(), &brief, vocabulary, 0.0).unwrap();
        let white = image::RgbImage::from_pixel(224, 224, image::Rgb([255, 255, 255]));
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(white)
            .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        let photo = PhotoRequest { jpeg, context: String::new(), progress: None };
        let scores = siglip.scores(&photo).unwrap();
        println!("{scores:?}");
        let best = scores.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert_eq!(best.0, "plain white");
        let reply = siglip.describe(&photo).unwrap();
        assert_eq!(reply.suggestion.keywords.len(), reply.suggestion.keyword_scores.len());
    }
}
