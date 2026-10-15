//! GPX tracks, for giving photos a position by when they were taken.

use anyhow::bail;
use chrono::DateTime;

/// A recorded position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fix {
    /// Unix epoch milliseconds (UTC).
    pub at: i64,
    pub lat: f64,
    pub lon: f64,
}

/// The attribute `name` of an opening tag's text (`trkpt lat="1" lon="2"`).
fn attribute(tag: &str, name: &str) -> Option<f64> {
    let mut rest = tag;
    while let Some(at) = rest.find(name) {
        let before = rest[..at].chars().last();
        let after = rest[at + name.len()..].trim_start();
        rest = &rest[at + name.len()..];
        if before.is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let Some(value) = after.strip_prefix('=') else { continue };
        let value = value.trim_start();
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let value = &value[1..];
        return value[..value.find(quote)?].trim().parse().ok();
    }
    None
}

/// The fixes of every track point and route point that has a time, oldest first.
pub fn parse(text: &str) -> anyhow::Result<Vec<Fix>> {
    let mut fixes = Vec::new();
    let mut at = 0;
    while let Some(found) = ["<trkpt", "<rtept"].iter().filter_map(|t| text[at..].find(t).map(|i| (i, t.len()))).min() {
        let start = at + found.0 + found.1;
        // `<trkptx>` and the like are other tags.
        if !text[start..].starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
            at = start;
            continue;
        }
        let Some(tag_end) = text[start..].find('>') else { break };
        let tag = &text[start..start + tag_end];
        let body_start = start + tag_end + 1;
        // A self-closing point has no time.
        let body_end = if tag.ends_with('/') {
            body_start
        } else {
            text[body_start..]
                .find("</trkpt>")
                .or_else(|| text[body_start..].find("</rtept>"))
                .map_or(text.len(), |i| body_start + i)
        };
        at = body_end.max(body_start);
        let body = &text[body_start..body_end];
        let (Some(lat), Some(lon)) = (attribute(tag, "lat"), attribute(tag, "lon")) else { continue };
        let Some(open) = body.find("<time>") else { continue };
        let rest = &body[open + 6..];
        let Some(close) = rest.find("</time>") else { continue };
        let Ok(time) = DateTime::parse_from_rfc3339(rest[..close].trim()) else { continue };
        if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
            fixes.push(Fix { at: time.timestamp_millis(), lat, lon });
        }
    }
    if fixes.is_empty() {
        bail!("no timed track points in this file");
    }
    fixes.sort_by_key(|f| f.at);
    Ok(fixes)
}

/// Where the track was at `at`: on the line between the fixes either side of it, if those
/// are at most `max_gap_ms` apart (a break in the recording is not walked across), or at a
/// fix exactly then.
pub fn locate(fixes: &[Fix], at: i64, max_gap_ms: i64) -> Option<(f64, f64)> {
    let after = fixes.partition_point(|f| f.at < at);
    let next = fixes.get(after)?;
    if next.at == at {
        return Some((next.lat, next.lon));
    }
    let prev = fixes.get(after.checked_sub(1)?)?;
    let gap = next.at - prev.at;
    if gap > max_gap_ms {
        return None;
    }
    let t = (at - prev.at) as f64 / gap as f64;
    // The short way round: across the date line, not the whole world.
    let dlon = (next.lon - prev.lon + 540.0).rem_euclid(360.0) - 180.0;
    let lon = (prev.lon + dlon * t + 540.0).rem_euclid(360.0) - 180.0;
    Some((prev.lat + (next.lat - prev.lat) * t, lon))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPX: &str = r#"<?xml version="1.0"?>
<gpx version="1.1"><trk><trkseg>
<trkpt lat="50.0" lon="19.0"><ele>200</ele><time>2026-05-03T10:00:00Z</time></trkpt>
<trkpt lon='19.2' lat='50.2'><time>2026-05-03T10:10:00+00:00</time></trkpt>
<trkpt lat="50.4" lon="19.4"><time>2026-05-03T12:00:00.500Z</time></trkpt>
<trkpt lat="1" lon="2"/>
<trkpt lat="1" lon="2"><ele>1</ele></trkpt>
</trkseg></trk></gpx>"#;

    fn at(h: u32, m: u32) -> i64 {
        DateTime::parse_from_rfc3339(&format!("2026-05-03T{h:02}:{m:02}:00Z")).unwrap().timestamp_millis()
    }

    #[test]
    fn reads_timed_points_in_either_attribute_order() {
        let fixes = parse(GPX).unwrap();
        assert_eq!(fixes.len(), 3);
        assert_eq!((fixes[1].lat, fixes[1].lon), (50.2, 19.2));
        assert_eq!(fixes[0].at, at(10, 0));
        assert!(parse("<gpx></gpx>").is_err());
    }

    #[test]
    fn interpolates_within_a_gap_and_not_across_a_break() {
        let fixes = parse(GPX).unwrap();
        let (lat, lon) = locate(&fixes, at(10, 5), 15 * 60_000).unwrap();
        assert!((lat - 50.1).abs() < 1e-9 && (lon - 19.1).abs() < 1e-9);
        assert_eq!(locate(&fixes, at(10, 10), 60_000), Some((50.2, 19.2)));
        // Between 10:10 and 12:00 the recording stopped.
        assert_eq!(locate(&fixes, at(11, 0), 15 * 60_000), None);
        assert_eq!(locate(&fixes, at(9, 0), 15 * 60_000), None);
        assert_eq!(locate(&fixes, at(13, 0), 15 * 60_000), None);
    }
}
