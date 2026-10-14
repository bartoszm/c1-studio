//! Names compared the way people type them: `Kraków`, `krakow` and `KRAKOW ` are one.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Lower case, without diacritics, every run of anything but letters and digits one space.
pub fn fold(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut space = false;
    for c in name.nfkd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase) {
        let c = match c {
            'ł' => "l",
            'ø' => "o",
            'đ' | 'ð' => "d",
            'ı' => "i",
            'æ' => "ae",
            'œ' => "oe",
            'ß' => "ss",
            'þ' => "th",
            c if c.is_alphanumeric() => {
                if space && !out.is_empty() {
                    out.push(' ');
                }
                space = false;
                out.push(c);
                continue;
            }
            // An apostrophe joins (`L'Aquila` and `LAquila` differ, but `Pol'šu` and
            // `Pol’šu` do not).
            '\'' | '’' | 'ʼ' | '`' => continue,
            _ => {
                space = true;
                continue;
            }
        };
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push_str(c);
    }
    out
}

/// Whether two folded names are one typo apart: a letter missing, extra, swapped with
/// its neighbour or different (`Istambul`, `Istanbul`). Short names must match exactly.
pub fn one_typo(a: &str, b: &str) -> bool {
    // Most names differ in length by more than a letter: no need to look closer. (Bytes
    // first: a letter is at most 4 of them.)
    if a.len().abs_diff(b.len()) > 4 {
        return false;
    }
    if a.is_ascii() && b.is_ascii() {
        return one_typo_in(a.as_bytes(), b.as_bytes());
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    one_typo_in(&a, &b)
}

fn one_typo_in<T: PartialEq>(a: &[T], b: &[T]) -> bool {
    if a.len().min(b.len()) < 5 || a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    if start == a.len() && start == b.len() {
        return true;
    }
    let (ra, rb) = (&a[start..], &b[start..]);
    match ra.len().cmp(&rb.len()) {
        std::cmp::Ordering::Equal => {
            ra[1..] == rb[1..] || (ra.len() >= 2 && ra[0] == rb[1] && ra[1] == rb[0] && ra[2..] == rb[2..])
        }
        std::cmp::Ordering::Greater => ra[1..] == *rb,
        std::cmp::Ordering::Less => *ra == rb[1..],
    }
}

/// Whether one folded name starts the other and they share most of their letters:
/// `malopolska`, `malopolskie`; `lower silesian`, `lower silesia`.
pub fn alike_start(a: &str, b: &str) -> bool {
    let common = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let shorter = a.chars().count().min(b.chars().count());
    common >= 5 && common * 4 >= shorter * 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_diacritics_and_spacing() {
        assert_eq!(fold("Kraków"), "krakow");
        assert_eq!(fold("  United  Kingdom "), "united kingdom");
        assert_eq!(fold("Łódź"), "lodz");
        assert_eq!(fold("Wałbrzych"), "walbrzych");
        assert_eq!(fold("Saint-Étienne"), "saint etienne");
        assert_eq!(fold("Pol’šu"), "polsu");
    }

    #[test]
    fn typos() {
        assert!(one_typo("istambul", "istanbul"));
        assert!(one_typo("krakow", "krakwo"));
        assert!(one_typo("poznan", "poznann"));
        assert!(!one_typo("rome", "rime"), "too short to guess");
        assert!(!one_typo("berlin", "dublin"));
    }

    #[test]
    fn alike_starts() {
        assert!(alike_start("malopolska", "malopolskie"));
        assert!(alike_start("lower silesian", "lower silesia"));
        assert!(alike_start("lower silesian", "lower silesian voivodeship"));
        assert!(!alike_start("silesian", "lower silesia"));
    }
}
