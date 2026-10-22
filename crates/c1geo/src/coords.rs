//! Coordinates as people type or paste them: `50.0614, 19.9366`, `33.8688S 151.2093E`,
//! `44°58′N 14°45′E`, `N44°58' E14°45'`, `44 58 N 14 45 E`, `50°3'41.0"N 19°56'12"E`, and
//! full plus codes, `8FVC9G8F+6X`. The same forms `parseGeo` in the app's `lib/format.ts`
//! reads (which takes short plus codes too, near a point it knows).

use crate::pluscode::{self, Kind};

/// What a number in typed coordinates was marked as.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Unit {
    Degrees,
    Minutes,
    Seconds,
    Bare,
}

/// A number, what it was marked as, and whether it had a minus.
type Num = (f64, Unit, bool);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Number(f64, Unit, bool),
    Letter(char),
    Separator,
}

fn tokens(text: &str) -> Option<Vec<Token>> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == ',' || c == ';' || c == '/' {
            out.push(Token::Separator);
            i += 1;
        } else if matches!(c.to_ascii_uppercase(), 'N' | 'S' | 'E' | 'W') {
            out.push(Token::Letter(c.to_ascii_uppercase()));
            i += 1;
        } else if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' {
            let negative = c == '-';
            let start = if c == '-' || c == '+' { i + 1 } else { i };
            let mut end = start;
            while end < chars.len() && (chars[end].is_ascii_digit() || chars[end] == '.') {
                end += 1;
            }
            let value: f64 = chars[start..end].iter().collect::<String>().parse().ok()?;
            i = end;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let unit = match chars.get(i) {
                Some('°' | 'º' | '˚') => Unit::Degrees,
                Some('\'' | '′' | '’' | 'ʹ' | '‘') if chars.get(i + 1) == Some(&'\'') => {
                    i += 1;
                    Unit::Seconds
                }
                Some('\'' | '′' | '’' | 'ʹ' | '‘') => Unit::Minutes,
                Some('"' | '″' | '”' | '“' | 'ʺ') => Unit::Seconds,
                _ => Unit::Bare,
            };
            if unit != Unit::Bare {
                i += 1;
            }
            out.push(Token::Number(value, unit, negative));
        } else {
            return None;
        }
    }
    Some(out)
}

/// One coordinate from its numbers: degrees, then minutes and seconds if given.
fn value(numbers: &[Num]) -> Option<f64> {
    let (degrees, minutes, seconds) = match numbers {
        [(d, _, _)] => (*d, 0.0, 0.0),
        [(d, _, _), (m, u, _)] if *u != Unit::Seconds => (*d, *m, 0.0),
        [(d, _, _), (m, _, _), (s, _, _)] => (*d, *m, *s),
        _ => return None,
    };
    if numbers.len() > 1 && (degrees.fract() != 0.0 || minutes >= 60.0 || seconds >= 60.0) {
        return None;
    }
    if numbers.len() > 2 && minutes.fract() != 0.0 {
        return None;
    }
    let v = degrees + minutes / 60.0 + seconds / 3600.0;
    Some(if numbers[0].2 { -v } else { v })
}

/// A typed position as `(lat, lon)`, or what is wrong with it. A full plus code
/// (`8FVC9G8F+6X`) is read too; a short one, which needs a place near it, is not.
pub fn parse_point(text: &str) -> Result<(f64, f64), &'static str> {
    const HOW: &str = "type latitude and longitude, e.g. 50.0614, 19.9366 or 44°58′N 14°45′E, or a full plus code";
    // A town after the code, as Google Maps gives them, says nothing a full code doesn't.
    let first = text.split(|c: char| c.is_whitespace() || c == ',').find(|w| !w.is_empty()).unwrap_or("");
    match pluscode::kind_of(first) {
        Some(Kind::Full) => return pluscode::decode(first).ok_or(HOW),
        Some(Kind::Short) => {
            return Err("a short plus code needs the place it is near: type the full code (8FVC9G8F+6X)");
        }
        None => {}
    }
    let tokens = tokens(text.trim()).ok_or(HOW)?;
    let letters: Vec<(usize, char)> = tokens
        .iter()
        .enumerate()
        .filter_map(|(i, t)| if let Token::Letter(c) = t { Some((i, *c)) } else { None })
        .collect();
    let numbers = |range: &[Token]| -> Vec<Num> {
        range.iter().filter_map(|t| if let Token::Number(v, u, n) = t { Some((*v, *u, *n)) } else { None }).collect()
    };
    // Each coordinate's numbers, and its letter if it has one.
    let groups: Vec<(Vec<Num>, Option<char>)> = match letters.as_slice() {
        [] => {
            let mut groups: Vec<Vec<Num>> = vec![Vec::new()];
            for t in &tokens {
                match t {
                    Token::Separator => groups.push(Vec::new()),
                    Token::Number(v, u, n) => {
                        let first = groups.len() == 1;
                        let current = groups.last_mut().expect("starts with one");
                        // A new degrees value starts the next coordinate: `44°58' 14°45'`.
                        if *u == Unit::Degrees && !current.is_empty() && first {
                            groups.push(vec![(*v, *u, *n)]);
                        } else {
                            current.push((*v, *u, *n));
                        }
                    }
                    Token::Letter(_) => unreachable!(),
                }
            }
            groups.retain(|g| !g.is_empty());
            if groups.len() == 1 && groups[0].iter().all(|n| n.1 == Unit::Bare) && groups[0].len().is_multiple_of(2) {
                // `44 58 14 45`: halves.
                let all = groups.remove(0);
                let half = all.len() / 2;
                groups = vec![all[..half].to_vec(), all[half..].to_vec()];
            }
            groups.into_iter().map(|g| (g, None)).collect()
        }
        [(a, la), (b, lb)] => {
            if *a == 0 || tokens[..*a].iter().all(|t| *t == Token::Separator) {
                // Letters first: `N44°58' E14°45'`.
                vec![(numbers(&tokens[a + 1..*b]), Some(*la)), (numbers(&tokens[b + 1..]), Some(*lb))]
            } else {
                if !numbers(&tokens[b + 1..]).is_empty() {
                    return Err(HOW);
                }
                vec![(numbers(&tokens[..*a]), Some(*la)), (numbers(&tokens[a + 1..*b]), Some(*lb))]
            }
        }
        _ => return Err(HOW),
    };
    let [(first, l1), (second, l2)] = <[_; 2]>::try_from(groups).map_err(|_| HOW)?;
    let (mut lat, mut lon) = (value(&first).ok_or(HOW)?, value(&second).ok_or(HOW)?);
    let lat_letter = |l: Option<char>| l.is_none_or(|c| c == 'N' || c == 'S');
    let lon_letter = |l: Option<char>| l.is_none_or(|c| c == 'E' || c == 'W');
    let (l1, l2) = if lat_letter(l1) && lon_letter(l2) {
        (l1, l2)
    } else if lon_letter(l1) && lat_letter(l2) && l1.is_some() {
        // `14°45′E 44°58′N`: longitude first.
        std::mem::swap(&mut lat, &mut lon);
        (l2, l1)
    } else {
        return Err(HOW);
    };
    // A letter says the side: `-33.8688S` is south once, not twice.
    if let Some(l) = l1 {
        lat = if l == 'S' { -lat.abs() } else { lat.abs() };
    }
    if let Some(l) = l2 {
        lon = if l == 'W' { -lon.abs() } else { lon.abs() };
    }
    if lat.abs() > 90.0 || lon.abs() > 180.0 || !lat.is_finite() || !lon.is_finite() {
        return Err("latitude is -90 to 90, longitude -180 to 180");
    }
    Ok((lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(text: &str, lat: f64, lon: f64) {
        let (a, b) = parse_point(text).unwrap_or_else(|e| panic!("{text:?}: {e}"));
        assert!((a - lat).abs() < 1e-9 && (b - lon).abs() < 1e-9, "{text:?} gave {a}, {b}");
    }

    #[test]
    fn decimal() {
        close("50.0614, 19.9366", 50.0614, 19.9366);
        close("50.0614 19.9366", 50.0614, 19.9366);
        close("-33.8688; 151.2093", -33.8688, 151.2093);
        close("33.8688S 151.2093E", -33.8688, 151.2093);
        close("-33.8688S, 151.2093E", -33.8688, 151.2093);
        close("-33.8688 S, -151.2093 W", -33.8688, -151.2093);
        close("33.8688 S, 151.2093 E", -33.8688, 151.2093);
    }

    #[test]
    fn degrees_minutes_seconds() {
        let krk = (44.0 + 58.0 / 60.0, 14.0 + 45.0 / 60.0);
        close("44°58′N 14°45′E", krk.0, krk.1);
        close("44°58'N, 14°45'E", krk.0, krk.1);
        close("N44°58′ E14°45′", krk.0, krk.1);
        close("N 44° 58′, E 14° 45′", krk.0, krk.1);
        close("44 58 N 14 45 E", krk.0, krk.1);
        close("14°45′E 44°58′N", krk.0, krk.1);
        close("44°58′ 14°45′", krk.0, krk.1);
        close("44 58, 14 45", krk.0, krk.1);
        close("50°3'41.0\"N 19°56'12\"E", 50.0 + 3.0 / 60.0 + 41.0 / 3600.0, 19.0 + 56.0 / 60.0 + 12.0 / 3600.0);
        close("50°3′41″N 19°56′12″W", 50.0 + 3.0 / 60.0 + 41.0 / 3600.0, -(19.0 + 56.0 / 60.0 + 12.0 / 3600.0));
        close("50°3'41''N 19°56'12''E", 50.0 + 3.0 / 60.0 + 41.0 / 3600.0, 19.0 + 56.0 / 60.0 + 12.0 / 3600.0);
        close("50°2.25′N 19°56.5′E", 50.0 + 2.25 / 60.0, 19.0 + 56.5 / 60.0);
    }

    #[test]
    fn full_plus_codes() {
        close("8FVC9G8F+6X", 47.3655625, 8.5249375);
        close(" 8fvc9g8f+6x Zürich, Switzerland", 47.3655625, 8.5249375);
        assert!(parse_point("9G8F+6X Zürich").unwrap_err().contains("short plus code"));
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        for bad in ["", "Kraków", "50.06", "91, 10", "10, 181", "44°58′E 14°45′E", "44°75′N 14°45′E", "1 2 3"]
        {
            assert!(parse_point(bad).is_err(), "{bad:?}");
        }
    }
}
