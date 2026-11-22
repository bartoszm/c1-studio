//! How good a picture the local model finds a photo: its embedding held against pairs of
//! sentences, one saying a photo is good in some way and one saying it is not. The model
//! was never taught to judge photos, and a sentence alone scores near nothing; which of
//! the two a photo sits closer to still says something, most of all between frames of one
//! burst, where everything else is the same. It tells frames apart; it is no rating.

use serde::{Deserialize, Serialize};

use crate::AiError;
use crate::siglip::{Embedder, LOGIT_SCALE, cosine};

/// Bumped when the sentences or the sums change: scores of another version are worked out
/// again from the stored embeddings.
pub const VERSION: u32 = 1;

/// One way a photo can be good, said both ways.
pub struct Axis {
    pub name: &'static str,
    pub good: &'static [&'static str],
    pub bad: &'static [&'static str],
}

pub const AXES: [Axis; 4] = [
    Axis {
        name: "composition",
        good: &[
            "a well composed photograph",
            "a photograph with a balanced, pleasing composition",
            "a photograph with the subject well placed in the frame",
        ],
        bad: &[
            "a badly composed photograph",
            "a photograph with an awkward, unbalanced composition",
            "a photograph with the subject cut off by the edge of the frame",
        ],
    },
    Axis {
        name: "moment",
        good: &[
            "a photograph taken at exactly the right moment",
            "a photograph of people with lively, natural expressions",
            "a photograph that catches a gesture at its peak",
        ],
        bad: &[
            "a photograph taken a moment too early or too late",
            "a photograph of people blinking or looking away",
            "a photograph in which nothing is happening",
        ],
    },
    Axis {
        name: "mood",
        good: &[
            "an atmospheric photograph with a strong mood",
            "an evocative, emotional photograph",
            "a photograph with beautiful light",
        ],
        bad: &["a dull, lifeless photograph", "a boring snapshot", "a photograph with flat, ugly light"],
    },
    Axis {
        name: "technical",
        good: &[
            "a sharp, well exposed photograph",
            "a professional, high quality photograph",
            "a photograph with the subject in perfect focus",
        ],
        bad: &[
            "a blurry, badly exposed photograph",
            "a low quality photograph",
            "a photograph with the subject out of focus",
        ],
    },
];

/// How much each of [`AXES`] counts in [`Aesthetic::overall`], in their order: what the
/// measured sharpness cannot see counts most.
const WEIGHTS: [f32; 4] = [0.3, 0.3, 0.25, 0.15];

/// A photo's score on each of [`AXES`], 0 to 1: above a half, closer to the good sentences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Aesthetic {
    pub composition: f32,
    pub moment: f32,
    pub mood: f32,
    pub technical: f32,
}

impl Aesthetic {
    pub fn overall(&self) -> f32 {
        let axes = [self.composition, self.moment, self.mood, self.technical];
        axes.iter().zip(WEIGHTS).map(|(score, weight)| score * weight).sum()
    }
}

/// One axis' sentences as embeddings: the good ones, and the bad ones.
pub type Sides = (Vec<Vec<f32>>, Vec<Vec<f32>>);

/// The sentences' embeddings, per axis.
pub struct Prompts {
    axes: Vec<Sides>,
}

impl Prompts {
    /// Embeds the sentences (once: they are kept beside the model).
    pub fn load(embedder: &Embedder, stopped: &dyn Fn() -> bool) -> Result<Self, AiError> {
        let mut axes = Vec::with_capacity(AXES.len());
        for axis in &AXES {
            axes.push((embedder.texts(axis.good, stopped)?, embedder.texts(axis.bad, stopped)?));
        }
        Ok(Prompts { axes })
    }

    /// From embeddings worked out elsewhere, per axis in the order of [`AXES`].
    pub fn from_embeddings(axes: Vec<Sides>) -> Self {
        Prompts { axes }
    }
}

fn mean_cosine(image: &[f32], texts: &[Vec<f32>]) -> f32 {
    if texts.is_empty() {
        return 0.0;
    }
    texts.iter().map(|text| cosine(image, text)).sum::<f32>() / texts.len() as f32
}

/// A picture's scores from its embedding. Per axis, how much closer it is to the good
/// sentences than to the bad, through the model's own scale: its bias falls out of the
/// difference, so the score is a half where the two are as near.
pub fn score(image: &[f32], prompts: &Prompts) -> Aesthetic {
    let axis = |i: usize| {
        let Some((good, bad)) = prompts.axes.get(i) else { return 0.5 };
        let apart = mean_cosine(image, good) - mean_cosine(image, bad);
        1.0 / (1.0 + (-apart * LOGIT_SCALE).exp())
    };
    Aesthetic { composition: axis(0), moment: axis(1), mood: axis(2), technical: axis(3) }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn unit(x: f32, y: f32) -> Vec<f32> {
        let length = (x * x + y * y).sqrt();
        vec![x / length, y / length]
    }

    #[test]
    fn a_picture_nearer_the_good_sentences_scores_over_a_half() {
        // Good along x, bad along y, on every axis.
        let pair = || (vec![unit(1.0, 0.0)], vec![unit(0.0, 1.0)]);
        let prompts = Prompts::from_embeddings(vec![pair(), pair(), pair(), pair()]);
        let good = score(&unit(1.0, 0.98), &prompts);
        let even = score(&unit(1.0, 1.0), &prompts);
        let bad = score(&unit(0.98, 1.0), &prompts);
        assert!((even.composition - 0.5).abs() < 1e-4, "{even:?}");
        assert!(good.mood > 0.6 && bad.mood < 0.4, "{good:?} {bad:?}");
        assert!(good.overall() > even.overall() && even.overall() > bad.overall());
        assert!((even.overall() - 0.5).abs() < 1e-4, "the weights add up to one");
    }

    #[test]
    fn an_axis_with_no_sentences_says_nothing() {
        let prompts = Prompts::from_embeddings(vec![(vec![unit(1.0, 0.0)], vec![unit(0.0, 1.0)])]);
        let s = score(&unit(1.0, 0.0), &prompts);
        assert!(s.composition > 0.99);
        assert_eq!((s.moment, s.mood, s.technical), (0.5, 0.5, 0.5));
    }

    /// With the model downloaded (`C1_MODELS` names the folder models are kept in) and
    /// `C1_BURSTS` naming a folder of folders, each the JPEGs of one group:
    /// `cargo test -p c1ai --release -- --ignored --nocapture bursts`. Prints each frame's
    /// scores and how alike the frames of a group are, for judging the sentences by eye.
    #[test]
    #[ignore = "needs the downloaded model and a folder of bursts"]
    fn the_model_scores_the_frames_of_bursts() {
        let root = PathBuf::from(std::env::var("C1_MODELS").expect("C1_MODELS"));
        let bursts = PathBuf::from(std::env::var("C1_BURSTS").expect("C1_BURSTS"));
        let embedder = Embedder::open(&root).unwrap();
        let prompts = Prompts::load(&embedder, &|| false).unwrap();
        let mut groups: Vec<PathBuf> = std::fs::read_dir(&bursts).unwrap().map(|e| e.unwrap().path()).collect();
        groups.sort();
        for group in groups.iter().filter(|g| g.is_dir()) {
            let mut files: Vec<PathBuf> = std::fs::read_dir(group).unwrap().map(|e| e.unwrap().path()).collect();
            files.retain(|f| f.extension().is_some_and(|e| e == "jpg"));
            files.sort();
            let started = std::time::Instant::now();
            let frames: Vec<(String, Vec<f32>)> = files
                .iter()
                .map(|f| {
                    let picture = image::open(f).unwrap().to_rgb8();
                    (f.file_stem().unwrap().to_string_lossy().into_owned(), embedder.image(&picture).unwrap())
                })
                .collect();
            let each = started.elapsed().as_millis() / frames.len().max(1) as u128;
            println!("{} ({each} ms a frame)", group.file_name().unwrap().to_string_lossy());
            for (name, e) in &frames {
                let s = score(e, &prompts);
                let alike: Vec<String> = frames.iter().map(|(_, other)| format!("{:.3}", cosine(e, other))).collect();
                println!(
                    "  {name:>10}  overall {:.3}  comp {:.3}  moment {:.3}  mood {:.3}  tech {:.3}  | {}",
                    s.overall(),
                    s.composition,
                    s.moment,
                    s.mood,
                    s.technical,
                    alike.join(" ")
                );
            }
        }
    }
}
