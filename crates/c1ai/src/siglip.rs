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

use crate::alike::Examples;
use crate::claude::Options;
use crate::models::SIGLIP2;
use crate::{AiError, Brief, Failed, PhotoRequest, RawAnswer, Reply, RunRecord, Step, Vocabulary, tell, validate};

/// What a keyword is wrapped in before the model reads it, as the model's authors do.
pub const TEMPLATE: &str = "This is a photo of {}.";

/// Keywords suggested for a photo, at most: the likeliest.
pub const MAX_KEYWORDS: usize = 8;

/// A keyword the model scores at least this (%) by itself is suggested, unless the
/// settings say otherwise. The model is sparing with a plain word: on a photo of the sea,
/// "sea" scores 1 to 2, where a sentence describing the photo scores over 90, and a word
/// that does not fit stays under 0.3. Under 3, what it suggests is mostly noise.
pub const DEFAULT_THRESHOLD: f32 = 3.0;

/// The least and the most a threshold can be (%).
pub const THRESHOLDS: (f32, f32) = (0.1, 99.0);

/// Bumped when the way keywords are scored changes: photos tagged by another version are
/// tagged again when asked.
const VERSION: u32 = 2;

/// The least a keyword must have to be suggested, in percent: `score` from the model by
/// itself, or `share` of the say of the photos that look alike ([`crate::alike`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Least {
    pub score: f32,
    pub share: f32,
}

impl Default for Least {
    fn default() -> Self {
        Least { score: DEFAULT_THRESHOLD, share: crate::alike::DEFAULT_SHARE }
    }
}

/// Hears of each photo's embedding as it is worked out, by the photo's key: kept, a photo
/// keyworded later teaches without being looked at again.
pub type Keep = Box<dyn Fn(&str, &[f32]) + Send + Sync>;

/// The side of the square the image tower takes, the length of an embedding, and the
/// tokens the text tower takes.
const SIZE: u32 = 224;
pub const DIM: usize = 768;
const CONTEXT: usize = 64;

/// The model's learned scale and bias, which turn a cosine into a probability: read from
/// `logit_scale` (4.7244534, before `exp`) and `logit_bias` of google/siglip2-base-patch16-224,
/// the checkpoint the ONNX export was made from; the export holds the towers only.
pub(crate) const LOGIT_SCALE: f32 = 112.669;
const LOGIT_BIAS: f32 = -16.771_725;

/// Says whether a stored suggestion was made the same way: by this version, with this
/// template and these thresholds. The photos it learned from are not part of it: they
/// grow with every keyword given, and a photo is not tagged again for that unless asked.
pub fn prompt_hash(least: &Least) -> u64 {
    let how = format!("siglip|{VERSION}|{TEMPLATE}|{:.1}|{:.0}|{MAX_KEYWORDS}", least.score, least.share);
    xxhash_rust::xxh3::xxh3_64(how.as_bytes())
}

/// Text as the model was trained on it: lowercase, no ASCII punctuation, single spaces.
pub fn canonical(text: &str) -> String {
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

/// A JPEG's pixels.
pub fn decode(jpeg: &[u8]) -> Result<image::RgbImage, AiError> {
    image::load_from_memory(jpeg)
        .map(|image| image.to_rgb8())
        .map_err(|e| AiError::BadReply(format!("the image could not be read: {e}")))
}

/// An image as the image tower takes it: squashed to the square, each channel in -1..1,
/// channel by channel.
fn tensor(image: &image::RgbImage) -> Vec<f32> {
    let small = image::imageops::resize(image, SIZE, SIZE, image::imageops::FilterType::CatmullRom);
    let plane = (SIZE * SIZE) as usize;
    let mut out = vec![0f32; 3 * plane];
    for (i, pixel) in small.pixels().enumerate() {
        for channel in 0..3 {
            out[channel * plane + i] = pixel.0[channel] as f32 / 127.5 - 1.0;
        }
    }
    out
}

fn normalise(v: &mut [f32]) {
    let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if length > 0.0 {
        v.iter_mut().for_each(|x| *x /= length);
    }
}

/// How well a text goes with an image, from their embeddings (each of length 1): the
/// model's logit, which [`likely`] turns into a probability.
pub fn logit(image: &[f32], text: &[f32]) -> f32 {
    let cosine: f32 = image.iter().zip(text).map(|(a, b)| a * b).sum();
    cosine * LOGIT_SCALE + LOGIT_BIAS
}

/// A logit as a probability, 0..1.
pub fn likely(logit: f32) -> f32 {
    1.0 / (1.0 + (-logit).exp())
}

/// How likely a text goes with an image, 0..1, from their embeddings (each of length 1).
fn probability(image: &[f32], text: &[f32]) -> f32 {
    likely(logit(image, text))
}

/// A keyword to suggest, by its pipe path: the share of the look-alikes' say it has and
/// how likely the model finds it by itself, each 0..1.
#[derive(Clone, Debug, PartialEq)]
struct Picked {
    path: String,
    share: f32,
    score: f32,
}

/// The keywords to suggest, at most [`MAX_KEYWORDS`]: those the look-alikes give at least
/// `least.share` of their say, the biggest share first, then those the model finds at
/// least `least.score` likely by itself, likeliest first. `shares` and `scores` hold
/// every keyword's, by path.
fn pick(shares: &HashMap<String, f32>, scores: &HashMap<String, f32>, least: &Least) -> Vec<Picked> {
    let of = |path: &String| Picked {
        path: path.clone(),
        share: shares.get(path).copied().unwrap_or(0.0),
        score: scores.get(path).copied().unwrap_or(0.0),
    };
    let mut alike: Vec<Picked> =
        shares.iter().filter(|(_, share)| **share >= least.share).map(|(p, _)| of(p)).collect();
    alike.sort_by(|a, b| b.share.total_cmp(&a.share).then_with(|| a.path.cmp(&b.path)));
    let mut own: Vec<Picked> = scores
        .iter()
        .filter(|(path, score)| **score >= least.score && !alike.iter().any(|a| &a.path == *path))
        .map(|(p, _)| of(p))
        .collect();
    own.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    alike.extend(own);
    alike.truncate(MAX_KEYWORDS);
    alike
}

fn local(e: impl std::fmt::Display) -> AiError {
    AiError::Local(e.to_string())
}

/// ONNX Runtime allows one environment per process. On Linux its library is one of the
/// model's files ([`crate::models::Runtime`]), found under `dir`, the model's folder.
fn environment(dir: &Path) -> Result<&'static Environment, AiError> {
    static ENVIRONMENT: OnceLock<Result<Environment, String>> = OnceLock::new();
    ENVIRONMENT
        .get_or_init(|| builder(dir)?.with_name("c1studio").build().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| AiError::Local(e.clone()))
}

#[cfg(not(target_os = "linux"))]
fn builder(_dir: &Path) -> Result<ort::environment::EnvironmentBuilder, String> {
    Ok(ort::init())
}

#[cfg(target_os = "linux")]
fn builder(dir: &Path) -> Result<ort::environment::EnvironmentBuilder, String> {
    let runtime = SIGLIP2.runtime.as_ref().ok_or("the local model does not run on this kind of computer")?;
    let library = dir.join(runtime.file);
    ort::init_from(&library).map_err(|e| format!("{}: {e}", library.display()))
}

/// A session on `file` of the model kept in `dir`.
fn session(dir: &Path, file: &str, threads: usize) -> Result<Session, AiError> {
    let path = dir.join(file);
    Session::builder(environment(dir)?)
        .map_err(local)?
        .with_optimization_level(GraphOptimizationLevel::All)
        .map_err(local)?
        .with_intra_threads(threads)
        .map_err(local)?
        .commit_from_file(&path)
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
        let session = session(dir, "textual/model.onnx", threads)?;
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

/// The model's two towers, each loaded when first needed. A text embedded once is kept
/// beside the model (`text-cache-v1.bin`) for the runs after.
pub struct Embedder {
    dir: PathBuf,
    threads: usize,
    image: Option<Session>,
    cache: TextCache,
}

impl Embedder {
    /// `root` is the folder models are kept in. Loads nothing yet.
    pub fn open(root: &Path) -> Result<Self, AiError> {
        if !SIGLIP2.installed(root) {
            return Err(AiError::Local("the local model is not downloaded: get it in Settings > AI".into()));
        }
        let dir = SIGLIP2.dir(root);
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get() / 2).max(1);
        Ok(Embedder { cache: TextCache::open(dir.join("text-cache-v1.bin")), dir, threads, image: None })
    }

    /// An image's embedding: [`DIM`] numbers, of length 1.
    pub fn image(&mut self, image: &image::RgbImage) -> Result<Vec<f32>, AiError> {
        if self.image.is_none() {
            self.image = Some(session(&self.dir, "visual/model.onnx", self.threads)?);
        }
        let session = self.image.as_mut().expect("loaded above");
        let side = SIZE as usize;
        let input = Tensor::from_array(([1usize, 3, side, side], tensor(image))).map_err(local)?;
        let outputs = session.run(ort::inputs![input]).map_err(local)?;
        embedding(&outputs)
    }

    /// The embeddings of texts, [`DIM`] numbers each, in their order. A text is read as
    /// the model was trained on it ([`canonical`]). `stopped` is asked between texts
    /// not embedded before; what was done by then is kept.
    pub fn texts(&mut self, texts: &[String], stopped: &dyn Fn() -> bool) -> Result<Vec<f32>, AiError> {
        let texts: Vec<String> = texts.iter().map(|t| canonical(t)).collect();
        let mut missing: Vec<&String> =
            texts.iter().filter(|t| !self.cache.known.contains_key(&TextCache::key(t))).collect();
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            // The text tower is over a gigabyte in memory: let go once the texts are done.
            let mut tower = TextTower::open(&self.dir, self.threads)?;
            let mut new = Vec::with_capacity(missing.len());
            let embedded = missing.iter().try_for_each(|text| {
                if stopped() {
                    return Err(AiError::Stopped);
                }
                new.push((TextCache::key(text), tower.embed(text)?));
                Ok(())
            });
            self.cache.add(new)?;
            embedded?;
        }
        let mut out = Vec::with_capacity(texts.len() * DIM);
        for text in &texts {
            out.extend_from_slice(&self.cache.known[&TextCache::key(text)]);
        }
        Ok(out)
    }
}

/// The cosine of two embeddings of length 1.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The model once loaded, with every keyword's embedding.
struct Loaded {
    embedder: Embedder,
    /// [`DIM`] numbers per name, in the names' order.
    texts: Vec<f32>,
}

pub struct Siglip {
    /// The folder models are kept in.
    root: PathBuf,
    options: Options,
    /// The keywords offered, by name.
    names: Vec<String>,
    vocabulary: Vocabulary,
    /// As shares of 1.
    least: Least,
    examples: Examples,
    keep: Option<Keep>,
    prompt: u64,
    /// Loaded when the first photo comes; one photo at a time goes through it, on several
    /// threads of its own.
    loaded: Mutex<Option<Loaded>>,
}

impl Siglip {
    /// `root` is the folder models are kept in; `least` what a keyword must have to be
    /// suggested, in percent; `examples` the photos keyworded by hand to learn from, if
    /// any; `keep` hears of each photo's embedding. Loads nothing yet.
    pub fn new(
        root: &Path,
        options: Options,
        brief: &Brief,
        vocabulary: Vocabulary,
        least: Least,
        examples: Examples,
        keep: Option<Keep>,
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
            root: root.to_path_buf(),
            options,
            names: brief.vocabulary.clone(),
            vocabulary,
            prompt: prompt_hash(&least),
            least: Least {
                score: least.score.clamp(0.0, THRESHOLDS.1) / 100.0,
                share: least.share.clamp(0.0, crate::alike::SHARES.1) / 100.0,
            },
            examples,
            keep,
            loaded: Mutex::new(None),
        })
    }

    fn load(&self) -> Result<Loaded, AiError> {
        let mut embedder = Embedder::open(&self.root)?;
        let phrases: Vec<String> = self.names.iter().map(|n| phrase(n)).collect();
        let texts = embedder.texts(&phrases, &|| self.options.stopped())?;
        Ok(Loaded { embedder, texts })
    }

    /// The photo's embedding, and every keyword offered with how likely the model finds
    /// it by itself, in the names' order.
    fn scores(&self, photo: &PhotoRequest) -> Result<Scored, AiError> {
        let pixels = decode(&photo.jpeg)?;
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if self.options.stopped() {
            return Err(AiError::Stopped);
        }
        if loaded.is_none() {
            *loaded = Some(self.load()?);
        }
        let model = loaded.as_mut().expect("loaded above");
        tell(&photo.progress, Step::Sent { attempt: 1 });
        let image = model.embedder.image(&pixels)?;
        let scores = self
            .names
            .iter()
            .zip(model.texts.as_chunks::<DIM>().0)
            .map(|(n, t)| (n.clone(), probability(&image, t)))
            .collect();
        Ok((image, scores))
    }

    /// The keyword at this path is one the run offers: the look-alikes may carry keywords
    /// it leaves out.
    fn offered(&self, path: &str) -> bool {
        self.vocabulary.path(leaf(path)) == Some(path)
    }
}

/// A photo's embedding, and names with how likely the model finds each.
type Scored = (Vec<f32>, Vec<(String, f32)>);

/// The last level of a pipe path: the keyword's name.
fn leaf(path: &str) -> &str {
    path.rsplit('|').next().unwrap_or(path)
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
        let (image, scored) = self.scores(photo)?;
        if let Some(keep) = self.keep.as_ref().filter(|_| !photo.key.is_empty()) {
            keep(&photo.key, &image);
        }
        // Names sharing a path keep the likeliest one's score.
        let mut scores: HashMap<String, f32> = HashMap::new();
        for (name, score) in scored {
            if let Some(path) = self.vocabulary.path(&name) {
                let best = scores.entry(path.to_string()).or_default();
                *best = best.max(score);
            }
        }
        let shares: HashMap<String, f32> = self
            .examples
            .votes(&image, &photo.key)
            .into_iter()
            .filter(|(path, _)| self.offered(path))
            .map(|(path, share)| (path.to_string(), share))
            .collect();
        let picked = pick(&shares, &scores, &self.least);
        let raw = RawAnswer {
            keywords: picked.iter().map(|p| leaf(&p.path).to_string()).collect(),
            confidence: picked.first().map_or(0.0, |p| p.share.max(p.score) as f64),
            ..RawAnswer::default()
        };
        let (mut suggestion, warnings) = validate(&raw, &self.vocabulary);
        let of = |path: &String| picked.iter().find(|p| &p.path == path);
        suggestion.keyword_scores = suggestion.keywords.iter().map(|k| of(k).map_or(0.0, |p| p.score)).collect();
        suggestion.keyword_shares = suggestion.keywords.iter().map(|k| of(k).map_or(0.0, |p| p.share)).collect();
        let said: Vec<String> = picked
            .iter()
            .map(|p| format!("{} {:.0}% alike, {:.1}% model", leaf(&p.path), p.share * 100.0, p.score * 100.0))
            .collect();
        let record = RunRecord {
            provider: "local".into(),
            model: SIGLIP2.id.into(),
            prompt: self.prompt,
            millis: started.elapsed().as_millis() as u64,
            raw: said.join("; "),
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
        let input = tensor(&decode(&jpeg).unwrap());
        assert_eq!(input.len(), 3 * 224 * 224);
        assert!(input.iter().all(|x| (x - 1.0).abs() < 0.05), "white is 1");
        assert!(matches!(decode(b"not a jpeg"), Err(AiError::BadReply(_))), "this photo's failure, not the run's");
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
    fn look_alikes_keywords_come_first_then_the_models_own() {
        let map = |pairs: &[(&str, f32)]| -> HashMap<String, f32> {
            pairs.iter().map(|(path, n)| (path.to_string(), *n)).collect()
        };
        let shares = map(&[("nature|sea", 0.8), ("blue", 0.3), ("boat", 0.1)]);
        let scores = map(&[("nature|sea", 0.02), ("boat", 0.2), ("gull", 0.05), ("street", 0.01)]);
        let least = Least { score: 0.03, share: 0.25 };
        let picked = pick(&shares, &scores, &least);
        let paths: Vec<&str> = picked.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, ["nature|sea", "blue", "boat", "gull"]);
        assert_eq!(picked[0], Picked { path: "nature|sea".into(), share: 0.8, score: 0.02 });
        assert_eq!((picked[2].share, picked[2].score), (0.1, 0.2), "by the model, its small share shown too");
        assert_eq!(picked[1].score, 0.0, "a keyword the model was not asked about");

        let many: HashMap<String, f32> = (0..12).map(|i| (format!("k{i:02}"), 0.5 + i as f32 * 0.01)).collect();
        let picked = pick(&many, &HashMap::new(), &least);
        assert_eq!((picked.len(), picked[0].path.as_str()), (MAX_KEYWORDS, "k11"));
        assert!(pick(&map(&[("a", 0.2)]), &map(&[("a", 0.02)]), &least).is_empty());
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
        let least = |score: f32, share: f32| Least { score, share };
        assert_ne!(prompt_hash(&least(1.0, 25.0)), prompt_hash(&least(1.5, 25.0)));
        assert_eq!(prompt_hash(&least(1.0, 25.0)), prompt_hash(&least(1.04, 25.0)), "by the tenth");
        assert_ne!(prompt_hash(&least(1.0, 25.0)), prompt_hash(&least(1.0, 30.0)));
    }

    #[test]
    fn it_needs_the_model_keywords_and_a_describing_run() {
        let root = tempfile::tempdir().unwrap();
        let brief = Brief { vocabulary: vec!["street".into()], ..Default::default() };
        let missing = Siglip::new(
            root.path(),
            Options::default(),
            &brief,
            Vocabulary::default(),
            Least::default(),
            Examples::new(DIM),
            None,
        );
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
        let least = Least { score: 0.0, share: 25.0 };
        let mut examples = Examples::new(DIM);
        let mut alike = vec![0.0; DIM];
        alike[0] = 1.0;
        examples.add("other", &alike, ["dog"]);
        let kept = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = kept.clone();
        let keep: Keep = Box::new(move |key, embedding| sink.lock().unwrap().push((key.to_string(), embedding.len())));
        let siglip = Siglip::new(&root, Options::default(), &brief, vocabulary, least, examples, Some(keep)).unwrap();
        let white = image::RgbImage::from_pixel(224, 224, image::Rgb([255, 255, 255]));
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(white)
            .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        let photo = PhotoRequest { key: "white".into(), jpeg, context: String::new(), progress: None };
        let (_, scores) = siglip.scores(&photo).unwrap();
        println!("{scores:?}");
        let best = scores.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert_eq!(best.0, "plain white");
        let reply = siglip.describe(&photo).unwrap();
        assert_eq!(reply.suggestion.keywords.len(), reply.suggestion.keyword_scores.len());
        assert_eq!(reply.suggestion.keywords.len(), reply.suggestion.keyword_shares.len());
        assert_eq!(reply.suggestion.keywords[0], "dog", "the one look-alike has all the say: {:?}", reply.record.raw);
        assert_eq!(*kept.lock().unwrap(), [("white".to_string(), DIM)]);
    }
}
