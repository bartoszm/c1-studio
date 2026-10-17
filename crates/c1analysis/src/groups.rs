//! Duplicates and series across a library, from each photo's hash and capture time.
//!
//! - **Duplicates** are one picture twice: different images (not variants of one, which
//!   are made on purpose) taken at the same instant and looking the same (hashes at most
//!   [`ClassSettings::duplicate_bits`] bits apart): imported twice, or saved again. Burst frames a moment
//!   apart look as alike, but they are a series.
//! - A **series** is a burst: [`ClassSettings::series_min`] or more photos from one camera, each taken
//!   within [`ClassSettings::series_gap_ms`] of the one before and looking much like it.

use std::collections::HashMap;

use crate::ClassSettings;

/// The most hash bits apart that [`group`] finds duplicates at: the lookup splits a hash
/// into four bands, and hashes within 3 bits share one of them exactly.
pub const DUPLICATE_MAX: u32 = 3;

pub fn distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// What grouping needs to know of a photo.
#[derive(Clone, Debug)]
pub struct Taken {
    /// The image a photo is a variant of.
    pub asset: String,
    pub camera: Option<String>,
    pub at: Option<i64>,
    pub hash: u64,
}

/// Groups of indexes into the photos given to [`group`]; each has two or more members.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Groups {
    pub duplicates: Vec<Vec<usize>>,
    pub series: Vec<Vec<usize>>,
}

/// A hash of a picture with almost no detail (a black frame): such pictures all hash
/// alike without being alike.
fn featureless(hash: u64) -> bool {
    !(6..=58).contains(&hash.count_ones())
}

pub fn group(photos: &[Taken], s: &ClassSettings) -> Groups {
    Groups { duplicates: duplicates(photos, s.duplicate_bits.min(DUPLICATE_MAX)), series: series(photos, s) }
}

fn duplicates(photos: &[Taken], bits: u32) -> Vec<Vec<usize>> {
    // Hashes within 3 bits share at least one of four 16-bit bands exactly.
    let mut buckets: HashMap<(u8, u16), Vec<usize>> = HashMap::new();
    for (i, p) in photos.iter().enumerate().filter(|(_, p)| !featureless(p.hash)) {
        for band in 0..4u8 {
            buckets.entry((band, (p.hash >> (16 * band)) as u16)).or_default().push(i);
        }
    }
    let mut parent: Vec<usize> = (0..photos.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for members in buckets.values() {
        for (n, &a) in members.iter().enumerate() {
            for &b in &members[n + 1..] {
                let (pa, pb) = (&photos[a], &photos[b]);
                if pa.asset != pb.asset && pa.at == pb.at && distance(pa.hash, pb.hash) <= bits {
                    let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                    if ra != rb {
                        parent[ra.max(rb)] = ra.min(rb);
                    }
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..photos.len() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    sorted(groups.into_values().filter(|g| g.len() > 1).collect())
}

fn series(photos: &[Taken], s: &ClassSettings) -> Vec<Vec<usize>> {
    let mut timed: Vec<usize> = (0..photos.len()).filter(|&i| photos[i].at.is_some()).collect();
    timed.sort_by_key(|&i| (photos[i].camera.clone(), photos[i].at, i));
    let mut out = Vec::new();
    let mut run: Vec<usize> = Vec::new();
    for i in timed {
        let follows = run.last().is_some_and(|&prev| {
            let (a, b) = (&photos[prev], &photos[i]);
            a.camera == b.camera
                && b.at.unwrap_or(0) - a.at.unwrap_or(0) <= s.series_gap_ms
                && a.asset != b.asset
                && distance(a.hash, b.hash) <= s.series_bits
        });
        if !follows {
            if run.len() >= s.series_min {
                out.push(std::mem::take(&mut run));
            }
            run.clear();
        }
        run.push(i);
    }
    if run.len() >= s.series_min {
        out.push(run);
    }
    sorted(out)
}

fn sorted(mut groups: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    for g in &mut groups {
        g.sort_unstable();
    }
    groups.sort();
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(asset: &str, at: i64, hash: u64) -> Taken {
        Taken { asset: asset.into(), camera: Some("X-Pro2".into()), at: Some(at), hash }
    }

    #[test]
    fn duplicates_are_alike_images_not_variants() {
        let h = 0x0f0f_3c3c_5a5a_a5a5;
        let photos = [
            p("1", 0, h),
            p("2", 0, h ^ 0b101), // two bits off, same instant: a duplicate
            p("1", 0, h),         // a variant of the first
            p("7", 300, h),       // a burst frame a moment later: not a duplicate
            p("3", 0, !h),        // nothing alike
            p("4", 0, 0),         // featureless
            p("5", 0, 0),
        ];
        let s = ClassSettings::default();
        assert_eq!(group(&photos, &s).duplicates, [vec![0, 1, 2]], "not the burst frame");
        let strict = ClassSettings { duplicate_bits: 1, ..s };
        assert!(group(&photos, &strict).duplicates.is_empty(), "two bits off is too far");
    }

    #[test]
    fn a_burst_is_a_series() {
        let h = 0x0f0f_3c3c_5a5a_a5a5u64;
        let photos = [
            p("1", 0, h),
            p("2", 1_000, h ^ 0xff),
            p("3", 2_500, h ^ 0xffff), // drifts, each step small
            p("4", 30_000, h),         // too long after
            p("5", 31_000, !h),        // too different
            p("6", 31_500, !h),
        ];
        let s = ClassSettings::default();
        assert_eq!(group(&photos, &s).series, [vec![0, 1, 2]]);
        // Asking for four photos, or a shorter wait, leaves it out.
        assert!(group(&photos, &ClassSettings { series_min: 4, ..s.clone() }).series.is_empty());
        assert!(group(&photos, &ClassSettings { series_gap_ms: 1_200, ..s }).series.is_empty());
    }
}
