//! Google's Plus Codes (Open Location Code): `8FVC9G8F+6X` is a spot of about 14 × 14 m.
//! Only full codes stand alone; a short one (`9G8F+6X`) needs a place near it, which the
//! app has and the command line does not. As `app/src/lib/pluscode.ts`.

const ALPHABET: &[u8] = b"23456789CFGHJMPQRVWX";
const SEPARATOR_AT: usize = 8;
const PAIR_LENGTH: usize = 10;
const PAIR_PRECISION: f64 = 8000.0;
const GRID_ROWS: f64 = 5.0;
const GRID_COLUMNS: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Full,
    Short,
}

fn digit(c: u8) -> Option<usize> {
    ALPHABET.iter().position(|&a| a == c.to_ascii_uppercase())
}

/// Whether `code` is a plus code, and which kind.
pub fn kind_of(code: &str) -> Option<Kind> {
    let c = code.as_bytes();
    let sep = code.find('+')?;
    if code.rfind('+') != Some(sep) || sep > SEPARATOR_AT || sep % 2 == 1 || sep < 2 {
        return None;
    }
    let (head, tail) = (&c[..sep], &c[sep + 1..]);
    if tail.len() == 1 || tail.iter().any(|&ch| digit(ch).is_none()) {
        return None;
    }
    let digits = match head.iter().position(|&ch| ch == b'0') {
        Some(pad) => {
            // Padding: only in a full code, from an even place to the separator, nothing after.
            if sep != SEPARATOR_AT
                || pad == 0
                || pad % 2 == 1
                || head[pad..].iter().any(|&ch| ch != b'0')
                || !tail.is_empty()
            {
                return None;
            }
            &head[..pad]
        }
        None => head,
    };
    if digits.iter().any(|&ch| digit(ch).is_none()) {
        return None;
    }
    if sep < SEPARATOR_AT {
        return Some(Kind::Short);
    }
    // The first pair is 20° steps: at most 180° of latitude and 360° of longitude.
    (digit(c[0])? * 20 < 180 && digit(c[1])? * 20 < 360).then_some(Kind::Full)
}

/// The middle of a full code's area.
pub fn decode(code: &str) -> Option<(f64, f64)> {
    if kind_of(code)? != Kind::Full {
        return None;
    }
    let c: Vec<usize> = code.bytes().filter(|&b| b != b'+' && b != b'0').map(|b| digit(b).expect("checked")).collect();
    let (mut lat, mut lon) = (-90.0 * PAIR_PRECISION, -180.0 * PAIR_PRECISION);
    let mut place = 160_000.0;
    let pairs = c.len().min(PAIR_LENGTH);
    for i in (0..pairs).step_by(2) {
        lat += c[i] as f64 * place;
        lon += c[i + 1] as f64 * place;
        if i + 2 < pairs {
            place /= 20.0;
        }
    }
    let (mut lat_size, mut lon_size) = (place / PAIR_PRECISION, place / PAIR_PRECISION);
    let (final_lat, final_lon) = (PAIR_PRECISION * GRID_ROWS.powi(5), PAIR_PRECISION * GRID_COLUMNS.powi(5));
    let (mut grid_lat, mut grid_lon) = (0.0, 0.0);
    if c.len() > PAIR_LENGTH {
        let (mut row, mut column) = (GRID_ROWS.powi(4), GRID_COLUMNS.powi(4));
        let end = c.len().min(15);
        for (i, &d) in c.iter().enumerate().take(end).skip(PAIR_LENGTH) {
            grid_lat += (d / 4) as f64 * row;
            grid_lon += (d % 4) as f64 * column;
            if i + 1 < end {
                row /= GRID_ROWS;
                column /= GRID_COLUMNS;
            }
        }
        lat_size = row / final_lat;
        lon_size = column / final_lon;
    }
    let south = lat / PAIR_PRECISION + grid_lat / final_lat;
    let west = lon / PAIR_PRECISION + grid_lon / final_lon;
    Some(((south + lat_size / 2.0).min(90.0), (west + lon_size / 2.0).min(180.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_codes_decode_to_their_middle() {
        let (lat, lon) = decode("8FVC9G8F+6X").unwrap();
        assert!((lat - 47.3655625).abs() < 1e-9 && (lon - 8.5249375).abs() < 1e-9, "{lat}, {lon}");
        assert_eq!(decode("8fvc0000+"), Some((47.5, 8.5)));
        let (lat, lon) = decode("7FG49Q00+").unwrap();
        assert!((lat - 20.375).abs() < 1e-9 && (lon - 2.775).abs() < 1e-9);
        assert!(decode("8FVC9G8F+6XQ").is_some());
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of("9G8F+6X"), Some(Kind::Short));
        assert_eq!(decode("9G8F+6X"), None);
        for bad in
            ["8FVC9G8F+6", "8FVC9G8F6X", "8F+VC+6X", "+6X", "8FVC9G8F+6A", "WC2345G6+", "8FV00000+", "8FVC0000+6X"]
        {
            assert_eq!(kind_of(bad), None, "{bad}");
        }
    }
}
