//! Shoots: photos taken one after another, split where two in a row are too far apart.

use crate::filter::TimeRange;

/// The shoot the photo taken at `times[at]` belongs to: the photos before and after it taken
/// one after another, no two more than `gap` ms apart, as the span from the first to the
/// last. `times` is in time order. None when `at` is not in it.
pub fn session(times: &[i64], at: usize, gap: i64) -> Option<TimeRange> {
    times.get(at)?;
    let mut lo = at;
    while lo > 0 && times[lo] - times[lo - 1] <= gap {
        lo -= 1;
    }
    let mut hi = at;
    while hi + 1 < times.len() && times[hi + 1] - times[hi] <= gap {
        hi += 1;
    }
    Some(TimeRange { from: Some(times[lo]), to: Some(times[hi]) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn span(from: i64, to: i64) -> Option<TimeRange> {
        Some(TimeRange { from: Some(from), to: Some(to) })
    }

    #[test]
    fn a_session_runs_while_photos_follow_within_the_gap() {
        let times = [0, 10 * MIN, 25 * MIN, 90 * MIN, 95 * MIN, 200 * MIN];
        assert_eq!(session(&times, 1, 30 * MIN), span(0, 25 * MIN));
        assert_eq!(session(&times, 0, 30 * MIN), span(0, 25 * MIN));
        assert_eq!(session(&times, 4, 30 * MIN), span(90 * MIN, 95 * MIN));
        // Alone, it is its own session; a wider gap joins the others.
        assert_eq!(session(&times, 5, 30 * MIN), span(200 * MIN, 200 * MIN));
        assert_eq!(session(&times, 5, 120 * MIN), span(0, 200 * MIN));
    }

    #[test]
    fn a_gap_of_exactly_the_setting_joins() {
        let times = [0, 30 * MIN, 61 * MIN];
        assert_eq!(session(&times, 0, 30 * MIN), span(0, 30 * MIN));
        assert_eq!(session(&times, 2, 30 * MIN), span(61 * MIN, 61 * MIN));
    }

    #[test]
    fn photos_taken_at_the_same_moment_are_one_shoot() {
        let times = [0, 5 * MIN, 5 * MIN, 5 * MIN];
        assert_eq!(session(&times, 2, MIN), span(5 * MIN, 5 * MIN));
    }

    #[test]
    fn a_place_past_the_end_is_none() {
        assert_eq!(session(&[0, MIN], 2, MIN), None);
        assert_eq!(session(&[], 0, MIN), None);
    }
}
