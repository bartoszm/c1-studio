//! Which photos are variants of one image: Capture One keeps several versions of an image
//! (each its own photo here), and the grid marks them and can show one per image.
//!
//! Kept in memory, worked out from the store whenever it changes: nothing about an image's
//! other variants goes into a photo's document, so adding or removing a variant re-indexes
//! nothing.

use std::collections::HashMap;

/// The variants of every image that has more than one.
#[derive(Debug, Default)]
pub struct Variants {
    /// Row → the rows of its image's variants, by variant number (lowest first). Only rows
    /// of images with more than one variant are here.
    of: HashMap<u64, std::sync::Arc<[u64]>>,
}

impl Variants {
    /// Groups `(row, image, variant number)` by image.
    pub fn new<K: std::hash::Hash + Eq>(photos: impl IntoIterator<Item = (u64, K, u32)>) -> Self {
        let mut images: HashMap<K, Vec<(u32, u64)>> = HashMap::new();
        for (row, image, index) in photos {
            images.entry(image).or_default().push((index, row));
        }
        let mut of = HashMap::new();
        for (_, mut rows) in images {
            if rows.len() < 2 {
                continue;
            }
            rows.sort_unstable();
            let rows: std::sync::Arc<[u64]> = rows.into_iter().map(|(_, row)| row).collect();
            for &row in rows.iter() {
                of.insert(row, rows.clone());
            }
        }
        Self { of }
    }

    /// How many variants `row`'s image has: 1 for an image with only this one.
    pub fn count(&self, row: u64) -> u32 {
        self.of.get(&row).map_or(1, |rows| rows.len() as u32)
    }

    /// The rows of `row`'s image's variants, lowest variant number first; just `row` for an
    /// image with only this one.
    pub fn siblings(&self, row: u64) -> Vec<u64> {
        self.of.get(&row).map_or_else(|| vec![row], |rows| rows.to_vec())
    }

    /// Every row of an image with more than one variant.
    pub fn multi_rows(&self) -> Vec<u64> {
        let mut rows: Vec<u64> = self.of.keys().copied().collect();
        rows.sort_unstable();
        rows
    }

    pub fn is_empty(&self) -> bool {
        self.of.is_empty()
    }

    /// Of `matching`, those to leave out to show one per image: all but the lowest-numbered
    /// variant of each image among them.
    pub fn away(&self, matching: &[u64]) -> Vec<u64> {
        // Per image (known by its lowest variant's row), the matching variants by number.
        let mut images: HashMap<u64, Vec<(usize, u64)>> = HashMap::new();
        for &row in matching {
            let Some(rows) = self.of.get(&row) else { continue };
            let at = rows.iter().position(|&r| r == row).unwrap_or(usize::MAX);
            images.entry(rows[0]).or_default().push((at, row));
        }
        let mut away = Vec::new();
        for mut rows in images.into_values() {
            rows.sort_unstable();
            rows.dedup();
            away.extend(rows.into_iter().skip(1).map(|(_, row)| row));
        }
        away.sort_unstable();
        away
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Variants {
        // Image a: rows 1 (variant 0), 5 (variant 2), 3 (variant 1). Image b: row 2 only.
        // Image c: rows 7 (variant 1) and 8 (variant 3): its variant 0 was deleted.
        Variants::new([
            (1, "s\x1fa", 0),
            (2, "s\x1fb", 0),
            (3, "s\x1fa", 1),
            (5, "s\x1fa", 2),
            (7, "s\x1fc", 1),
            (8, "s\x1fc", 3),
        ])
    }

    #[test]
    fn counts_and_siblings_go_by_image() {
        let v = sample();
        assert_eq!((v.count(1), v.count(3), v.count(2), v.count(99)), (3, 3, 1, 1));
        assert_eq!(v.siblings(5), vec![1, 3, 5]);
        assert_eq!(v.siblings(2), vec![2]);
        assert_eq!(v.multi_rows(), vec![1, 3, 5, 7, 8]);
    }

    #[test]
    fn away_keeps_the_lowest_matching_variant_of_each_image() {
        let v = sample();
        assert_eq!(v.away(&[1, 2, 3, 5, 7, 8]), vec![3, 5, 8]);
        // Variant 0 does not match: the next one stands for the image.
        assert_eq!(v.away(&[5, 3, 2]), vec![5]);
        // The lowest left after variant 0 went.
        assert_eq!(v.away(&[8, 7]), vec![8]);
        assert_eq!(v.away(&[2]), Vec::<u64>::new());
    }

    #[test]
    fn single_variant_images_are_not_kept() {
        let v = Variants::new([(1, "s\x1fa", 0), (2, "s\x1fb", 0)]);
        assert!(v.is_empty());
    }
}
