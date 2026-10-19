//! IPTC IIM, as JPEG files carry it in APP13 (Photoshop's 8BIM resource 0x0404).

use c1model::KeywordPath;

use super::xmp::XmpFields;

/// Header of the APP13 segment Photoshop and its kin write.
const PHOTOSHOP: &[u8] = b"Photoshop 3.0\0";
/// The 8BIM resource that holds the IIM data.
const IPTC_RESOURCE: u16 = 0x0404;
/// Escape sequence of record 1 dataset 90 that declares UTF-8.
const UTF8_ESCAPE: &[u8] = b"\x1b%G";

/// The fields in the payload of an APP13 segment (what follows its length): the 8BIM
/// resources, one of which is the IIM. Nothing from a payload that is not Photoshop's.
pub fn parse_app13(payload: &[u8]) -> Option<XmpFields> {
    let mut rest = payload.strip_prefix(PHOTOSHOP)?;
    while let Some(tail) = rest.strip_prefix(b"8BIM") {
        let id = u16::from_be_bytes(tail.get(..2)?.try_into().ok()?);
        // a Pascal string, padded to an even length including its length byte
        let name_len = usize::from(*tail.get(2)?);
        let size_at = 2 + (1 + name_len).next_multiple_of(2);
        let size = u32::from_be_bytes(tail.get(size_at..size_at + 4)?.try_into().ok()?) as usize;
        let data = tail.get(size_at + 4..size_at + 4 + size)?;
        if id == IPTC_RESOURCE {
            return Some(parse_iim(data));
        }
        rest = tail.get(size_at + 4 + size.next_multiple_of(2)..).unwrap_or_default();
    }
    None
}

/// The fields of raw IIM data: a run of `0x1C record dataset length value` datasets.
pub fn parse_iim(data: &[u8]) -> XmpFields {
    let datasets = datasets(data);
    let utf8 = datasets.iter().any(|d| (d.record, d.number) == (1, 90) && d.value == UTF8_ESCAPE);
    let text = |record: u8, number: u8| -> Vec<String> {
        datasets
            .iter()
            .filter(|d| (d.record, d.number) == (record, number))
            .filter_map(|d| decode(d.value, utf8))
            .collect()
    };
    let first = |number: u8| text(2, number).into_iter().next();

    let mut fields = XmpFields::default();
    fields.iptc.title = first(5);
    fields.iptc.headline = first(105);
    fields.iptc.description = first(120);
    fields.iptc.creator = Some(text(2, 80).join("; ")).filter(|s| !s.is_empty());
    fields.iptc.copyright = first(116);
    fields.iptc.job_id = first(103);
    fields.iptc.instructions = first(40);
    fields.iptc.source = first(115);
    fields.place.city = first(90);
    fields.place.state = first(95);
    fields.place.country = first(101);
    fields.place.country_code = first(100);
    fields.place.location = first(92);
    fields.keywords = text(2, 25).into_iter().map(|k| KeywordPath(vec![k])).collect();
    fields
}

struct Dataset<'a> {
    record: u8,
    number: u8,
    value: &'a [u8],
}

/// Splits IIM data into datasets, stopping at the first that does not fit.
fn datasets(data: &[u8]) -> Vec<Dataset<'_>> {
    let mut out = Vec::new();
    let mut at = 0;
    while data.get(at) == Some(&0x1c) {
        let Some(&[record, number, hi, lo]) = data.get(at + 1..at + 5) else { break };
        let len = u16::from_be_bytes([hi, lo]);
        let (len, value_at) = if len & 0x8000 == 0 {
            (usize::from(len), at + 5)
        } else {
            // extended dataset: the length itself takes `len & 0x7fff` bytes
            let width = usize::from(len & 0x7fff);
            let Some(bytes) = data.get(at + 5..at + 5 + width).filter(|_| width <= 4) else { break };
            (bytes.iter().fold(0, |n, &b| n << 8 | usize::from(b)), at + 5 + width)
        };
        let Some(value) = data.get(value_at..value_at + len) else { break };
        out.push(Dataset { record, number, value });
        at = value_at + len;
    }
    out
}

/// A text value: UTF-8 when declared or when valid as such, else Latin-1. Blank is `None`.
fn decode(value: &[u8], utf8: bool) -> Option<String> {
    let text = match std::str::from_utf8(value) {
        Ok(s) => s.to_owned(),
        Err(_) if utf8 => String::from_utf8_lossy(value).into_owned(),
        Err(_) => value.iter().map(|&b| char::from(b)).collect(),
    };
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn dataset(out: &mut Vec<u8>, record: u8, number: u8, value: &[u8]) {
        out.extend([0x1c, record, number]);
        out.extend((value.len() as u16).to_be_bytes());
        out.extend(value);
    }

    /// The payload of an APP13 segment holding an IIM with a few fields.
    pub(crate) fn sample_app13() -> Vec<u8> {
        let mut iim = Vec::new();
        dataset(&mut iim, 1, 90, UTF8_ESCAPE);
        dataset(&mut iim, 2, 5, "Dom nad rzeką".as_bytes());
        dataset(&mut iim, 2, 105, b"A headline");
        dataset(&mut iim, 2, 120, "Zażółć gęślą jaźń".as_bytes());
        dataset(&mut iim, 2, 80, b"Ann");
        dataset(&mut iim, 2, 80, b"Bob");
        dataset(&mut iim, 2, 116, b"(c) Ann");
        dataset(&mut iim, 2, 90, "Wrocław".as_bytes());
        dataset(&mut iim, 2, 101, b"Poland");
        dataset(&mut iim, 2, 100, b"POL");
        dataset(&mut iim, 2, 92, b"Rynek");
        dataset(&mut iim, 2, 25, b"river");
        dataset(&mut iim, 2, 25, b"house");

        let mut out = PHOTOSHOP.to_vec();
        // an unrelated resource first, with an odd-length name and data to exercise the padding
        out.extend(b"8BIM");
        out.extend(0x03edu16.to_be_bytes());
        out.extend([2, b'a', b'b', 0]);
        out.extend(3u32.to_be_bytes());
        out.extend([1, 2, 3, 0]);
        out.extend(b"8BIM");
        out.extend(IPTC_RESOURCE.to_be_bytes());
        out.extend([0, 0]);
        out.extend((iim.len() as u32).to_be_bytes());
        out.extend(&iim);
        if iim.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    #[test]
    fn reads_the_datasets() {
        let fields = parse_app13(&sample_app13()).unwrap();
        assert_eq!(fields.iptc.title.as_deref(), Some("Dom nad rzeką"));
        assert_eq!(fields.iptc.headline.as_deref(), Some("A headline"));
        assert_eq!(fields.iptc.description.as_deref(), Some("Zażółć gęślą jaźń"));
        assert_eq!(fields.iptc.creator.as_deref(), Some("Ann; Bob"));
        assert_eq!(fields.iptc.copyright.as_deref(), Some("(c) Ann"));
        assert_eq!(fields.iptc.job_id, None);
        assert_eq!(fields.place.city.as_deref(), Some("Wrocław"));
        assert_eq!(fields.place.country.as_deref(), Some("Poland"));
        assert_eq!(fields.place.country_code.as_deref(), Some("POL"));
        assert_eq!(fields.place.location.as_deref(), Some("Rynek"));
        assert_eq!(fields.keywords, [KeywordPath(vec!["river".into()]), KeywordPath(vec!["house".into()])]);
    }

    #[test]
    fn latin_1_without_a_declared_charset() {
        let mut iim = Vec::new();
        dataset(&mut iim, 2, 90, b"Z\xfcrich");
        assert_eq!(parse_iim(&iim).place.city.as_deref(), Some("Zürich"));
    }

    #[test]
    fn damage_does_not_panic() {
        let all = sample_app13();
        for cut in 0..all.len() {
            let _ = parse_app13(&all[..cut]);
        }
        assert!(parse_app13(b"Something else").is_none());
        assert_eq!(parse_iim(&[0x1c, 2, 5, 0xff, 0xff, 1]), XmpFields::default());
    }
}
