//! How well a photo matches the colours, light and focus picked in a filter, so the grid can
//! show the best (or worst) matches first.
//!
//! Each picked dimension has its own measure (a colour's share of the picture, how dark it
//! is, how sharp), scaled to 0..1; with several dimensions picked the scores are averaged.
//! Within one dimension a photo is as good as its best pick, as the filter itself means "any
//! of these". Pure functions: the index reads the measures from fast fields.

use c1analysis::{ClassSettings, ColourMode, HUES};

/// The measures a score is made of (what the analysis stored for the photo).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measures {
    pub hue_shares: [f32; HUES.len()],
    /// Each colour's biggest patch (share of the picture).
    pub hue_patches: [f32; HUES.len()],
    pub chroma: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub sharpness: f32,
}

/// The sharpness at which the focus scale tops out (about the sharpest photos measure).
const SHARPEST: f32 = 20_000.0;
/// A clipped or blocked share counts fully at this many times the line that makes the class.
const OVER_THE_LINE: f32 = 4.0;

#[derive(Clone, Debug, PartialEq)]
pub struct MatchSpec {
    /// Indexes into [`HUES`] of the picked colours.
    colours: Vec<usize>,
    black_and_white: bool,
    light: Vec<String>,
    focus: Vec<String>,
    lines: ClassSettings,
    /// All of the colours are asked for, not any.
    every_colour: bool,
}

fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

impl MatchSpec {
    /// What there is to rank by in the picks of a filter (values with no measure, such as
    /// `muted` or a shape, are left out); None when nothing is left.
    pub fn new(hues: &[String], light: &[String], focus: &[String], lines: &ClassSettings) -> Option<Self> {
        let colours: Vec<usize> = hues.iter().filter_map(|h| HUES.iter().position(|n| n == h)).collect();
        let black_and_white = hues.iter().any(|h| h == "black & white");
        const LIGHT: [&str; 6] = ["dark", "medium", "bright", "clipped highlights", "blocked shadows", "low contrast"];
        let light: Vec<String> = light.iter().filter(|l| LIGHT.contains(&l.as_str())).cloned().collect();
        let focus: Vec<String> =
            focus.iter().filter(|f| ["sharp", "soft", "blurry"].contains(&f.as_str())).cloned().collect();
        let any = !colours.is_empty() || black_and_white || !light.is_empty() || !focus.is_empty();
        any.then(|| Self { colours, black_and_white, light, focus, lines: lines.clone(), every_colour: false })
    }

    /// The colours were asked for all at once ("red and blue"), not any of them.
    pub fn every_colour(self, every: bool) -> Self {
        Self { every_colour: every && self.colours.len() > 1, ..self }
    }

    /// 0..1: how well the measures match the picks (1 is a perfect match).
    pub fn score(&self, m: &Measures) -> f32 {
        let mut parts: Vec<f32> = Vec::new();
        if !self.colours.is_empty() || self.black_and_white {
            let line = (self.lines.patch_share * OVER_THE_LINE).max(f32::EPSILON);
            // One picked colour's measure: its share, or its biggest patch with full marks at
            // four times the line.
            let one = |i: usize| match self.lines.colour_mode {
                ColourMode::Covers => unit(m.hue_shares[i]),
                ColourMode::Patch => unit(m.hue_patches[i] / line),
            };
            let colour = if self.every_colour {
                // Asked for all of them (red and blue): as good as the weakest.
                self.colours.iter().map(|&i| one(i)).fold(1.0, f32::min)
            } else {
                match self.lines.colour_mode {
                    // How much of the picture is in any of the picked colours.
                    ColourMode::Covers => unit(self.colours.iter().map(|&i| m.hue_shares[i]).sum()),
                    // The biggest patch of any of them.
                    ColourMode::Patch => self.colours.iter().map(|&i| one(i)).fold(0.0, f32::max),
                }
            };
            let mono = if self.black_and_white && self.lines.mono_chroma > 0.0 {
                unit(1.0 - m.chroma / self.lines.mono_chroma)
            } else {
                0.0
            };
            parts.push(colour.max(mono));
        }
        if !self.light.is_empty() {
            parts.push(self.light.iter().map(|l| self.light_score(l, m)).fold(0.0, f32::max));
        }
        if !self.focus.is_empty() {
            parts.push(self.focus.iter().map(|f| self.focus_score(f, m.sharpness)).fold(0.0, f32::max));
        }
        parts.iter().sum::<f32>() / parts.len().max(1) as f32
    }

    fn light_score(&self, class: &str, m: &Measures) -> f32 {
        let s = &self.lines;
        match class {
            "dark" => unit(1.0 - m.brightness / s.dark_below.max(f32::EPSILON)),
            "bright" => unit((m.brightness - s.bright_above) / (1.0 - s.bright_above).max(f32::EPSILON)),
            "medium" => {
                let (mid, half) = ((s.dark_below + s.bright_above) / 2.0, (s.bright_above - s.dark_below) / 2.0);
                unit(1.0 - (m.brightness - mid).abs() / half.max(f32::EPSILON))
            }
            "clipped highlights" => unit(m.highlights / (s.clipped_share * OVER_THE_LINE).max(f32::EPSILON)),
            "blocked shadows" => unit(m.shadows / (s.blocked_share * OVER_THE_LINE).max(f32::EPSILON)),
            "low contrast" => unit(1.0 - m.contrast / s.flat_below.max(f32::EPSILON)),
            _ => 0.0,
        }
    }

    fn focus_score(&self, class: &str, sharpness: f32) -> f32 {
        // On a log scale: sharpness spans orders of magnitude, and so do the lines.
        let scale = (1.0 + SHARPEST).ln();
        let t = unit((1.0 + sharpness.max(0.0)).ln() / scale);
        match class {
            "sharp" => t,
            "blurry" => 1.0 - t,
            "soft" => {
                let mid = ((1.0 + self.lines.blurry_below).ln() + (1.0 + self.lines.soft_below).ln()) / 2.0 / scale;
                unit(1.0 - (t - mid).abs() * 2.0)
            }
            _ => 0.0,
        }
    }
}

/// The sort key of a photo: higher sorts first. A photo not measured (`None`) has the lowest
/// key, so it comes last either way; `worst` turns the scale round. The row in the low bits
/// keeps photos with the same score in their order.
pub fn key(score: Option<f32>, row: u64, worst: bool) -> u64 {
    let low = u64::from(u32::MAX) - (row & 0xffff_ffff);
    match score {
        Some(s) => {
            let permille = (unit(s) * 1000.0).round() as u64;
            ((1 + if worst { 1000 - permille } else { permille }) << 32) | low
        }
        None => low,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines() -> ClassSettings {
        ClassSettings::default()
    }
    fn spec(hues: &[&str], light: &[&str], focus: &[&str]) -> MatchSpec {
        let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        MatchSpec::new(&own(hues), &own(light), &own(focus), &lines()).unwrap()
    }
    fn with_hue(name: &str, share: f32) -> Measures {
        let mut m = Measures::default();
        m.hue_shares[HUES.iter().position(|h| *h == name).unwrap()] = share;
        m
    }

    #[test]
    fn nothing_to_rank_by() {
        let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(MatchSpec::new(&[], &[], &[], &lines()).is_none());
        assert!(MatchSpec::new(&own(&["muted"]), &[], &[], &lines()).is_none());
    }

    #[test]
    fn colours_add_up_across_the_picks() {
        let s = spec(&["red", "green"], &[], &[]);
        let mut both = with_hue("red", 0.3);
        both.hue_shares[HUES.iter().position(|h| *h == "green").unwrap()] = 0.2;
        assert!((s.score(&both) - 0.5).abs() < 1e-6);
        assert!(s.score(&with_hue("red", 0.4)) > s.score(&both) - 0.2);
        assert_eq!(s.score(&with_hue("blue", 0.9)), 0.0);
        assert_eq!(spec(&["red"], &[], &[]).score(&with_hue("red", 1.5)), 1.0);
    }

    #[test]
    fn black_and_white_is_the_lack_of_chroma() {
        let s = spec(&["black & white"], &[], &[]);
        assert_eq!(s.score(&Measures { chroma: 0.0, ..Measures::default() }), 1.0);
        assert!(s.score(&Measures { chroma: 2.0, ..Measures::default() }) > 0.4);
        assert_eq!(s.score(&Measures { chroma: 40.0, ..Measures::default() }), 0.0);
        // Next to a colour pick, whichever fits the photo better counts.
        let either = spec(&["black & white", "red"], &[], &[]);
        assert_eq!(either.score(&Measures { chroma: 0.0, ..with_hue("red", 0.1) }), 1.0);
    }

    #[test]
    fn light_follows_its_own_measure() {
        let m = |brightness, highlights, shadows, contrast| Measures {
            brightness,
            highlights,
            shadows,
            contrast,
            ..Measures::default()
        };
        assert!(
            spec(&[], &["dark"], &[]).score(&m(0.05, 0.0, 0.0, 0.3))
                > spec(&[], &["dark"], &[]).score(&m(0.15, 0.0, 0.0, 0.3))
        );
        assert_eq!(spec(&[], &["dark"], &[]).score(&m(0.5, 0.0, 0.0, 0.3)), 0.0);
        assert_eq!(spec(&[], &["bright"], &[]).score(&m(1.0, 0.0, 0.0, 0.3)), 1.0);
        assert_eq!(spec(&[], &["medium"], &[]).score(&m(0.375, 0.0, 0.0, 0.3)), 1.0);
        assert_eq!(spec(&[], &["clipped highlights"], &[]).score(&m(0.5, 0.5, 0.0, 0.3)), 1.0);
        assert_eq!(spec(&[], &["blocked shadows"], &[]).score(&m(0.5, 0.0, 0.0, 0.3)), 0.0);
        assert_eq!(spec(&[], &["low contrast"], &[]).score(&m(0.5, 0.0, 0.0, 0.0)), 1.0);
        // Any of the picks: the better one counts.
        assert_eq!(spec(&[], &["dark", "bright"], &[]).score(&m(1.0, 0.0, 0.0, 0.3)), 1.0);
    }

    #[test]
    fn focus_is_ranked_on_a_log_scale() {
        let at = |sharpness| Measures { sharpness, ..Measures::default() };
        let sharp = spec(&[], &[], &["sharp"]);
        let blurry = spec(&[], &[], &["blurry"]);
        let soft = spec(&[], &[], &["soft"]);
        assert!(sharp.score(&at(5000.0)) > sharp.score(&at(500.0)));
        assert!(blurry.score(&at(10.0)) > blurry.score(&at(500.0)));
        // A soft photo (between the lines) beats both a sharp and a blurry one for "soft".
        assert!(soft.score(&at(450.0)) > soft.score(&at(15_000.0)));
        assert!(soft.score(&at(450.0)) > soft.score(&at(5.0)));
    }

    #[test]
    fn in_patch_mode_the_biggest_patch_of_a_picked_colour_counts() {
        let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let lines = ClassSettings { colour_mode: ColourMode::Patch, patch_share: 0.02, ..lines() };
        let s = MatchSpec::new(&own(&["red", "green"]), &[], &[], &lines).unwrap();
        let with_patch = |name: &str, patch: f32| {
            let mut m = Measures::default();
            m.hue_patches[HUES.iter().position(|h| *h == name).unwrap()] = patch;
            m
        };
        // Full marks at four times the line (8%), half at 4%; the share covered does not count.
        assert_eq!(s.score(&with_patch("red", 0.08)), 1.0);
        assert!((s.score(&with_patch("green", 0.04)) - 0.5).abs() < 1e-6);
        assert_eq!(s.score(&Measures { ..with_hue("red", 0.9) }), 0.0);
    }

    #[test]
    fn asking_for_every_colour_ranks_by_the_weakest() {
        let mut big_red_small_blue = with_hue("red", 0.55);
        big_red_small_blue.hue_shares[HUES.iter().position(|h| *h == "blue").unwrap()] = 0.05;
        let mut both = with_hue("red", 0.3);
        both.hue_shares[HUES.iter().position(|h| *h == "blue").unwrap()] = 0.3;
        let any = spec(&["red", "blue"], &[], &[]);
        let every = spec(&["red", "blue"], &[], &[]).every_colour(true);
        // Any of them: both cover 60%. Every one: the photo with plenty of each wins.
        assert!((any.score(&big_red_small_blue) - any.score(&both)).abs() < 1e-6);
        assert!(every.score(&both) > every.score(&big_red_small_blue));
        assert!((every.score(&big_red_small_blue) - 0.05).abs() < 1e-6);
    }

    #[test]
    fn dimensions_are_averaged() {
        let s = spec(&["red"], &[], &["sharp"]);
        let m = Measures { sharpness: 20_000.0, ..with_hue("red", 0.5) };
        assert!((s.score(&m) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn keys_order_best_first_and_unmeasured_last() {
        let best = key(Some(0.9), 5, false);
        let worse = key(Some(0.2), 1, false);
        assert!(best > worse);
        assert!(worse > key(None, 1, false));
        // The other way round: worst first, still not measured last.
        assert!(key(Some(0.2), 1, true) > key(Some(0.9), 5, true));
        assert!(key(Some(0.9), 5, true) > key(None, 5, true));
        // Equal scores keep row order.
        assert!(key(Some(0.5), 3, false) > key(Some(0.5), 4, false));
    }
}
