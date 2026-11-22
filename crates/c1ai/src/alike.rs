//! Keywords from the photos that look most alike among those keyworded by hand: what the
//! photographer gave photos like this one says more of a photo than a model's own guess
//! at each word. Measured on a catalog keyworded over years, a keyword that a quarter of
//! the look-alikes carry is right six times in ten; the model scoring the catalog's
//! keywords by itself is right fewer than one time in ten.
//!
//! A photo and its look-alikes are compared by their embeddings ([`crate::siglip`]):
//! lists of numbers of length 1 whose product says how alike two photos are.

use std::collections::HashMap;

/// The photos most alike that are asked.
pub const NEIGHBOURS: usize = 30;

/// How fast a look-alike counts for less as it is less alike than the closest one: at
/// this much less of a likeness, 1/e of it.
const SHARP: f32 = 0.05;

/// The least share of the look-alikes' say a keyword must have to be suggested (%),
/// unless the settings say otherwise.
pub const DEFAULT_SHARE: f32 = 25.0;

/// The least and the most that share can be set to (%).
pub const SHARES: (f32, f32) = (5.0, 100.0);

/// Photos keyworded by hand, each with its embedding and its keywords.
pub struct Examples {
    dim: usize,
    keys: Vec<String>,
    /// `dim` numbers a photo, in the photos' order.
    embeddings: Vec<f32>,
    /// Each photo's keywords, as places in `paths`.
    keywords: Vec<Vec<u32>>,
    paths: Vec<String>,
    places: HashMap<String, u32>,
}

impl Examples {
    /// For embeddings of `dim` numbers.
    pub fn new(dim: usize) -> Self {
        Examples {
            dim,
            keys: Vec::new(),
            embeddings: Vec::new(),
            keywords: Vec::new(),
            paths: Vec::new(),
            places: HashMap::new(),
        }
    }

    /// Takes the photo `key` with its keywords (pipe paths). One with no keywords, or an
    /// embedding of another length, teaches nothing and is left out.
    pub fn add<'a>(&mut self, key: &str, embedding: &[f32], paths: impl IntoIterator<Item = &'a str>) {
        let keywords: Vec<u32> = paths
            .into_iter()
            .map(|path| match self.places.get(path) {
                Some(place) => *place,
                None => {
                    let place = self.paths.len() as u32;
                    self.paths.push(path.to_string());
                    self.places.insert(path.to_string(), place);
                    place
                }
            })
            .collect();
        if keywords.is_empty() || embedding.len() != self.dim {
            return;
        }
        self.keys.push(key.to_string());
        self.embeddings.extend_from_slice(embedding);
        self.keywords.push(keywords);
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The keywords of the [`NEIGHBOURS`] photos most like `image`, each with its share
    /// of their say (0..1), the biggest first. A look-alike's say falls with how much
    /// less alike it is than the closest. The photo `but` is not asked: the photo itself.
    pub fn votes(&self, image: &[f32], but: &str) -> Vec<(&str, f32)> {
        if image.len() != self.dim {
            return Vec::new();
        }
        let mut alike: Vec<(f32, usize)> = self
            .embeddings
            .chunks_exact(self.dim)
            .enumerate()
            .filter(|(i, _)| self.keys[*i] != but)
            .map(|(i, other)| (image.iter().zip(other).map(|(a, b)| a * b).sum(), i))
            .collect();
        if alike.len() > NEIGHBOURS {
            alike.select_nth_unstable_by(NEIGHBOURS - 1, |a, b| b.0.total_cmp(&a.0));
            alike.truncate(NEIGHBOURS);
        }
        let closest = alike.iter().map(|a| a.0).fold(f32::MIN, f32::max);
        let mut say: HashMap<u32, f32> = HashMap::new();
        let mut total = 0f32;
        for (likeness, photo) in alike {
            let weight = ((likeness - closest) / SHARP).exp();
            total += weight;
            for keyword in &self.keywords[photo] {
                *say.entry(*keyword).or_default() += weight;
            }
        }
        let mut votes: Vec<(&str, f32)> =
            say.into_iter().map(|(keyword, weight)| (self.paths[keyword as usize].as_str(), weight / total)).collect();
        votes.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        votes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn examples() -> Examples {
        let mut examples = Examples::new(2);
        examples.add("sea-1", &[1.0, 0.0], ["nature|sea", "blue"]);
        examples.add("sea-2", &[0.999, 0.045], ["nature|sea"]);
        examples.add("street-1", &[0.0, 1.0], ["street"]);
        examples.add("bare", &[1.0, 0.0], []);
        examples.add("short", &[1.0], ["odd"]);
        examples
    }

    #[test]
    fn photos_with_no_keywords_or_an_odd_embedding_are_not_examples() {
        assert_eq!(examples().len(), 3);
        assert!(Examples::new(2).is_empty());
    }

    #[test]
    fn the_look_alikes_keywords_are_shared_out_by_likeness() {
        let examples = examples();
        let votes = examples.votes(&[1.0, 0.0], "");
        assert_eq!(votes[0].0, "nature|sea");
        assert!(votes[0].1 > 0.99, "both photos of the sea have it: {votes:?}");
        assert_eq!(votes[1].0, "blue");
        assert!((0.4..0.6).contains(&votes[1].1), "one of the two alike has it: {votes:?}");
        let street = votes.iter().find(|v| v.0 == "street").unwrap().1;
        assert!(street < 1e-6, "a photo nothing like it has next to no say: {votes:?}");
        let total: f32 = examples.votes(&[0.0, 1.0], "").iter().map(|v| v.1).sum();
        assert!(total > 0.99);
    }

    #[test]
    fn a_photo_is_not_asked_about_itself() {
        let examples = examples();
        let votes = examples.votes(&[1.0, 0.0], "sea-1");
        assert!(votes.iter().all(|v| v.0 != "blue"), "only the photo itself had it: {votes:?}");
        assert!(examples.votes(&[1.0], "").is_empty());
        assert!(Examples::new(2).votes(&[1.0, 0.0], "").is_empty());
    }
}
