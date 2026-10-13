//! A colour mapping learned from two renders of the same frame: for each colour of the
//! first, the colour the second made of it. Fitted from Capture One's small finished
//! render and the large unedited one, it carries saturation, film simulations, colour
//! edits and grading over to the large one, everything that applies to the whole frame
//! alike.
//!
//! The mapping is a 3D table over 8-bit sRGB, read with trilinear interpolation. Its
//! nodes are a least-squares fit to the pixel pairs, kept smooth so that colours the
//! photo lacks follow the best single colour matrix from their neighbours, then fitted again with pixels that disagree
//! (edges, a stale thumbnail) given less say.

use crate::RgbImage;

/// Nodes per axis: 16 steps of 16 levels.
const N: usize = 17;
const NODES: usize = N * N * N;
const STEP: f32 = 255.0 / (N - 1) as f32;

/// How strongly neighbouring nodes are pulled together, relative to the pixels each node
/// holds on average.
const SMOOTHNESS: f64 = 0.02;

/// A pixel this far from the fit (out of 255) counts half in the second fit.
const ROBUST_SCALE: f32 = 12.0;

/// Fewer pixel pairs than this say too little about a photo's colours.
const MIN_SAMPLES: usize = 1_000;

const CG_ITERATIONS: usize = 400;
const CG_TOLERANCE: f64 = 1e-4;

pub struct Lut {
    /// Output colour (0–255) at each node, indexed `(r * N + g) * N + b`.
    nodes: Vec<[f32; 3]>,
    /// The median distance (0–255, over the three channels) of the fitted pairs from the
    /// table: how well one colour mapping explains them. Pairs that do not show the same
    /// spot (a perspective fix only one render has) leave it high.
    pub residual: f32,
}

impl Lut {
    pub fn identity() -> Self {
        let nodes = (0..NODES).map(|i| node_colour(i).map(|c| c as f32)).collect();
        Self { nodes, residual: 0.0 }
    }

    /// The mapping that turns `source` into `target`, two images of one size. `None` when
    /// they differ in size or have too few usable pixels.
    pub fn fit(source: &RgbImage, target: &RgbImage) -> Option<Self> {
        if source.dimensions() != target.dimensions() {
            return None;
        }
        Self::fit_pairs(source.pixels().zip(target.pixels()).map(|(s, t)| (s.0, t.0)))
    }

    /// The mapping from each pair's first colour to its second.
    pub fn fit_pairs(pairs: impl IntoIterator<Item = ([u8; 3], [u8; 3])>) -> Option<Self> {
        let samples: Vec<([u8; 3], [u8; 3])> = pairs
            .into_iter()
            // Black in the target where the source has a picture: the corners of a stale
            // thumbnail made before the photo was straightened.
            .filter(|(s, t)| !(t.iter().all(|&c| c <= 3) && s.iter().any(|&c| c > 24)))
            .collect();
        if samples.len() < MIN_SAMPLES {
            return None;
        }
        let mut weights = vec![1.0f32; samples.len()];
        let mut lut = Self::identity();
        let mut distances = vec![0.0f32; samples.len()];
        for pass in 0..2 {
            lut = lut.solve(&samples, &weights);
            for ((s, t), d) in samples.iter().zip(&mut distances) {
                let p = lut.at(*s);
                *d = (0..3).map(|c| (p[c] - f32::from(t[c])).powi(2)).sum::<f32>().sqrt();
            }
            if pass == 0 {
                for (w, d) in weights.iter_mut().zip(&distances) {
                    *w = 1.0 / (1.0 + (d / ROBUST_SCALE).powi(2));
                }
            }
        }
        let middle = distances.len() / 2;
        lut.residual = *distances.select_nth_unstable_by(middle, f32::total_cmp).1;
        Some(lut)
    }

    /// The colour `rgb` maps to, 0–255.
    pub fn at(&self, rgb: [u8; 3]) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for (index, weight) in corners(rgb) {
            let n = self.nodes[index];
            for c in 0..3 {
                out[c] += weight * n[c];
            }
        }
        out
    }

    pub fn apply(&self, image: &mut RgbImage) {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let raw: &mut [u8] = image;
        let chunk = raw.len().div_ceil(threads * 3).max(1) * 3;
        std::thread::scope(|scope| {
            for part in raw.chunks_mut(chunk) {
                scope.spawn(move || {
                    for px in part.as_chunks_mut::<3>().0 {
                        let mut out = [0.0f32; 3];
                        for (index, weight) in corners([px[0], px[1], px[2]]) {
                            let n = self.nodes[index];
                            for c in 0..3 {
                                out[c] += weight * n[c];
                            }
                        }
                        for c in 0..3 {
                            px[c] = (out[c] + 0.5).clamp(0.0, 255.0) as u8;
                        }
                    }
                });
            }
        });
    }

    /// Least squares over the weighted pairs plus smoothness, solved by conjugate
    /// gradients starting from `self`.
    fn solve(&self, samples: &[([u8; 3], [u8; 3])], weights: &[f32]) -> Self {
        let mut a = vec![[0.0f64; 27]; NODES];
        let mut rhs = vec![[0.0f64; 3]; NODES];
        let mut mass = 0.0f64;
        for ((s, t), &w) in samples.iter().zip(weights) {
            let w = f64::from(w);
            mass += w;
            let k = corners(*s);
            for (ci, &(i, wi)) in k.iter().enumerate() {
                let wi = f64::from(wi) * w;
                for (cj, &(_, wj)) in k.iter().enumerate() {
                    a[i][CORNER_OFFSET[ci][cj]] += wi * f64::from(wj);
                }
                for c in 0..3 {
                    rhs[i][c] += wi * f64::from(t[c]);
                }
            }
        }
        // Smoothness asks neighbouring nodes to differ as the best single colour matrix
        // would have them differ, so colours the photo lacks follow that matrix rather
        // than staying flat at the last colour seen.
        let matrix = colour_matrix(samples, weights);
        let prior: Vec<[f64; 3]> = (0..NODES).map(|i| apply_matrix(&matrix, node_colour(i))).collect();
        let lambda = SMOOTHNESS * mass / NODES as f64;
        // A faint pull towards the matrix itself keeps the system well posed.
        let anchor = lambda * 1e-3;
        for (i, row) in a.iter_mut().enumerate() {
            for j in axis_neighbours(i) {
                row[13] += lambda;
                row[offset(i, j)] -= lambda;
                for c in 0..3 {
                    rhs[i][c] += lambda * (prior[i][c] - prior[j][c]);
                }
            }
            row[13] += anchor;
            for c in 0..3 {
                rhs[i][c] += anchor * prior[i][c];
            }
        }
        // The channels share the matrix and are solved side by side.
        let channels: Vec<Vec<f64>> = std::thread::scope(|scope| {
            let solving: Vec<_> = (0..3)
                .map(|c| {
                    let (a, rhs) = (&a, &rhs);
                    scope.spawn(move || {
                        let b: Vec<f64> = rhs.iter().map(|r| r[c]).collect();
                        let start: Vec<f64> = self.nodes.iter().map(|n| f64::from(n[c])).collect();
                        conjugate_gradients(a, &b, start)
                    })
                })
                .collect();
            solving.into_iter().map(|h| h.join().expect("a solver thread does not panic")).collect()
        });
        let nodes = (0..NODES).map(|i| std::array::from_fn(|c| channels[c][i] as f32)).collect();
        Self { nodes, residual: 0.0 }
    }
}

/// The weighted least-squares affine map from source to target colours: three rows of
/// `[r, g, b, 1]` coefficients.
fn colour_matrix(samples: &[([u8; 3], [u8; 3])], weights: &[f32]) -> [[f64; 4]; 3] {
    let mut ata = [[0.0f64; 4]; 4];
    let mut atb = [[0.0f64; 3]; 4];
    for ((s, t), &w) in samples.iter().zip(weights) {
        let x = [f64::from(s[0]), f64::from(s[1]), f64::from(s[2]), 1.0];
        let w = f64::from(w);
        for i in 0..4 {
            for j in 0..4 {
                ata[i][j] += w * x[i] * x[j];
            }
            for c in 0..3 {
                atb[i][c] += w * x[i] * f64::from(t[c]);
            }
        }
    }
    // A little ridge towards nothing keeps a grey photo's matrix finite.
    let ridge = 1e-6 * (ata[0][0] + ata[1][1] + ata[2][2]).max(1.0);
    for (i, row) in ata.iter_mut().enumerate().take(3) {
        row[i] += ridge;
    }
    let solution = solve4(ata, atb);
    std::array::from_fn(|c| std::array::from_fn(|i| solution[i][c]))
}

fn apply_matrix(m: &[[f64; 4]; 3], [r, g, b]: [f64; 3]) -> [f64; 3] {
    m.map(|row| row[0] * r + row[1] * g + row[2] * b + row[3])
}

/// Gaussian elimination with partial pivoting, for three right-hand sides at once.
fn solve4(mut a: [[f64; 4]; 4], mut b: [[f64; 3]; 4]) -> [[f64; 3]; 4] {
    for col in 0..4 {
        let pivot = (col..4).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs())).unwrap_or(col);
        a.swap(col, pivot);
        b.swap(col, pivot);
        let p = a[col][col];
        if p.abs() < 1e-12 {
            continue;
        }
        let (pivot_a, pivot_b) = (a[col], b[col]);
        for row in (0..4).filter(|&row| row != col) {
            let f = a[row][col] / p;
            for (v, q) in a[row].iter_mut().zip(pivot_a).skip(col) {
                *v -= f * q;
            }
            for (v, q) in b[row].iter_mut().zip(pivot_b) {
                *v -= f * q;
            }
        }
    }
    std::array::from_fn(|i| {
        let p = a[i][i];
        std::array::from_fn(|c| if p.abs() < 1e-12 { 0.0 } else { b[i][c] / p })
    })
}

/// For each 8-bit level: the lower node on that axis and how far past it the level is.
const CELLS: [(usize, f32); 256] = {
    let mut t = [(0, 0.0); 256];
    let mut v = 0;
    while v < 256 {
        let x = v as f32 / STEP;
        let i = if (x as usize) < N - 2 { x as usize } else { N - 2 };
        t[v] = (i, x - i as f32);
        v += 1;
    }
    t
};

/// The eight nodes around `rgb` and their trilinear weights.
fn corners([r, g, b]: [u8; 3]) -> [(usize, f32); 8] {
    let (ir, fr) = CELLS[usize::from(r)];
    let (ig, fg) = CELLS[usize::from(g)];
    let (ib, fb) = CELLS[usize::from(b)];
    let base = (ir * N + ig) * N + ib;
    std::array::from_fn(|k| {
        let (dr, dg, db) = (k >> 2 & 1, k >> 1 & 1, k & 1);
        let w = if dr == 1 { fr } else { 1.0 - fr }
            * if dg == 1 { fg } else { 1.0 - fg }
            * if db == 1 { fb } else { 1.0 - fb };
        (base + dr * N * N + dg * N + db, w)
    })
}

fn node_coords(i: usize) -> [usize; 3] {
    [i / (N * N), i / N % N, i % N]
}

fn node_colour(i: usize) -> [f64; 3] {
    node_coords(i).map(|k| k as f64 * f64::from(STEP))
}

/// For trilinear corners `a` and `b` of one cell (bits r, g, b), where `b` sits in `a`'s
/// row of 27.
const CORNER_OFFSET: [[usize; 8]; 8] = {
    let mut t = [[0; 8]; 8];
    let mut a = 0;
    while a < 8 {
        let mut b = 0;
        while b < 8 {
            let (dr, dg, db) =
                ((b >> 2 & 1) + 1 - (a >> 2 & 1), (b >> 1 & 1) + 1 - (a >> 1 & 1), (b & 1) + 1 - (a & 1));
            t[a][b] = dr * 9 + dg * 3 + db;
            b += 1;
        }
        a += 1;
    }
    t
};

/// The index step to each of the 27 neighbours, in row order.
const NEIGHBOUR_STEP: [isize; 27] = {
    let mut t = [0; 27];
    let mut o = 0;
    while o < 27 {
        let n = N as isize;
        t[o] = (o as isize / 9 - 1) * n * n + (o as isize / 3 % 3 - 1) * n + (o as isize % 3 - 1);
        o += 1;
    }
    t
};

/// Where node `j`, one step at most from node `i` on each axis, sits in `i`'s row of 27.
fn offset(i: usize, j: usize) -> usize {
    let (a, b) = (node_coords(i), node_coords(j));
    let d = |k: usize| b[k] + 1 - a[k];
    d(0) * 9 + d(1) * 3 + d(2)
}

/// Nodes one step away along a single axis.
fn axis_neighbours(i: usize) -> impl Iterator<Item = usize> {
    let p = node_coords(i);
    let strides = [N * N, N, 1];
    (0..3).flat_map(move |axis| {
        let below = (p[axis] > 0).then(|| i - strides[axis]);
        let above = (p[axis] + 1 < N).then(|| i + strides[axis]);
        below.into_iter().chain(above)
    })
}

/// `a · x` for the banded matrix: each row holds its node's 27 neighbours.
/// Entries are non-zero only where the neighbour exists, so the step never leaves the grid.
fn multiply(a: &[[f64; 27]], x: &[f64], out: &mut [f64]) {
    for (i, (row, out)) in a.iter().zip(out).enumerate() {
        let mut sum = 0.0;
        for (&v, &step) in row.iter().zip(&NEIGHBOUR_STEP) {
            if v != 0.0 {
                sum += v * x[i.wrapping_add_signed(step)];
            }
        }
        *out = sum;
    }
}

/// Solves `a · x = b` for a symmetric positive definite `a`, with Jacobi preconditioning.
fn conjugate_gradients(a: &[[f64; 27]], b: &[f64], mut x: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    let dot = |u: &[f64], v: &[f64]| u.iter().zip(v).map(|(p, q)| p * q).sum::<f64>();
    let diagonal: Vec<f64> = a.iter().map(|row| row[13]).collect();
    let mut ax = vec![0.0; n];
    multiply(a, &x, &mut ax);
    let mut r: Vec<f64> = b.iter().zip(&ax).map(|(b, ax)| b - ax).collect();
    let mut z: Vec<f64> = r.iter().zip(&diagonal).map(|(r, d)| r / d).collect();
    let mut p = z.clone();
    let mut rz = dot(&r, &z);
    let goal = CG_TOLERANCE * dot(b, b).sqrt();
    let mut ap = vec![0.0; n];
    for _ in 0..CG_ITERATIONS {
        if dot(&r, &r).sqrt() <= goal {
            break;
        }
        multiply(a, &p, &mut ap);
        let alpha = rz / dot(&p, &ap);
        for k in 0..n {
            x[k] += alpha * p[k];
            r[k] -= alpha * ap[k];
            z[k] = r[k] / diagonal[k];
        }
        let next = dot(&r, &z);
        let beta = next / rz;
        rz = next;
        for k in 0..n {
            p[k] = z[k] + beta * p[k];
        }
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every colour in a 32-level cube, as a 1-pixel-high image.
    fn cube() -> RgbImage {
        let levels: Vec<u8> = (0..32).map(|k| (k * 255 / 31) as u8).collect();
        let mut pixels = Vec::new();
        for &r in &levels {
            for &g in &levels {
                for &b in &levels {
                    pixels.extend([r, g, b]);
                }
            }
        }
        RgbImage::from_raw(32 * 32 * 32, 1, pixels).unwrap()
    }

    fn mapped(image: &RgbImage, f: impl Fn([u8; 3]) -> [u8; 3]) -> RgbImage {
        let mut out = image.clone();
        for p in out.pixels_mut() {
            p.0 = f(p.0);
        }
        out
    }

    fn worst(a: &RgbImage, b: &RgbImage) -> u8 {
        a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| x.abs_diff(*y)).max().unwrap()
    }

    #[test]
    fn identity_leaves_colours() {
        let source = cube();
        let mut image = source.clone();
        Lut::identity().apply(&mut image);
        assert!(worst(&image, &source) <= 1);
    }

    #[test]
    fn learns_a_channel_swap_and_a_curve() {
        let source = cube();
        let f = |[r, g, b]: [u8; 3]| [g, ((f32::from(b) / 255.0).powf(0.7) * 255.0) as u8, r];
        let target = mapped(&source, f);
        let lut = Lut::fit(&source, &target).unwrap();
        let mut image = source.clone();
        lut.apply(&mut image);
        let w = worst(&image, &target);
        assert!(w <= 6, "{w}");
    }

    #[test]
    fn colours_it_never_saw_follow_their_neighbours() {
        // Fitted on dark colours only; a bright one still comes out roughly right.
        let source = cube();
        let dark: Vec<u8> =
            source.as_raw().chunks(3).filter(|p| p.iter().all(|&c| c < 160)).flatten().copied().collect();
        let dark = RgbImage::from_raw(dark.len() as u32 / 3, 1, dark).unwrap();
        let brighten = |p: [u8; 3]| p.map(|c| (u16::from(c) * 9 / 10 + 20) as u8);
        let lut = Lut::fit(&dark, &mapped(&dark, brighten)).unwrap();
        let out = lut.at([200, 200, 200]);
        assert!(out.iter().all(|&c| (c - 200.0).abs() < 4.0), "{out:?}");
    }

    #[test]
    fn pairs_from_different_spots_leave_a_large_residual() {
        let source = cube();
        let same = Lut::fit(&source, &mapped(&source, |p| p.map(|c| c / 2 + 60))).unwrap();
        assert!(same.residual < 3.0, "{}", same.residual);
        // Each colour paired with a random other: no colour mapping explains that.
        let mut pixels: Vec<[u8; 3]> = source.pixels().map(|p| p.0).collect();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for i in (1..pixels.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            pixels.swap(i, (seed % (i as u64 + 1)) as usize);
        }
        let shuffled = RgbImage::from_raw(source.width(), 1, pixels.concat()).unwrap();
        let other = Lut::fit(&source, &shuffled).unwrap();
        assert!(other.residual > 50.0, "{}", other.residual);
    }

    #[test]
    fn mismatched_sizes_do_not_fit() {
        assert!(Lut::fit(&RgbImage::new(40, 40), &RgbImage::new(40, 41)).is_none());
    }
}
