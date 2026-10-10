//! Smart albums: Capture One stores their rules (`ZCOLLECTION.ZSMARTCRITERIA`), not their
//! members. The rules are XML in a fixed layout:
//!
//! ```xml
//! <MatchOperator Kind="AND">
//!   <MatchOperator Kind="OR">
//!     <Condition Enabled="YES">
//!       <Key>IB_S_CONTENT_KEYWORDS</Key><Operator>6</Operator><Criterion>street</Criterion>
//!     </Condition>
//!   </MatchOperator>
//!   …
//! </MatchOperator>
//! ```
//!
//! The outer kind joins the conditions; each inner block holds one. Every album starts
//! with a disabled `ANY` placeholder. Operator codes were read off albums in a real
//! catalog whose names state their intent (see `legacy/c1cat-py/c1cat/collections.py`).

use c1model::{Condition, SmartRule};

const KEY_RATING: &str = "IB_I_BASIC_RATING";
const KEY_KEYWORDS: &str = "IB_S_CONTENT_KEYWORDS";
const KEY_DATE: &str = "IB_S_EXP_DATE";

const OP_EQUALS: u32 = 0;
const OP_AT_LEAST: u32 = 2;
const OP_GREATER: u32 = 5;
const OP_CONTAINS: u32 = 6;

pub fn parse(xml: &str) -> SmartRule {
    let match_all = attr(xml, "MatchOperator", "Kind").is_none_or(|k| k != "OR");
    let enabled = |block: &'_ str| -> Vec<String> {
        block
            .split("<Condition")
            .skip(1)
            .filter(|c| attr_in(c, "Enabled").as_deref() == Some("YES"))
            .map(str::to_owned)
            .collect()
    };
    // Split on opening tags: the first piece is the XML header, the second the outer block.
    let blocks: Vec<&str> = xml.split("<MatchOperator").skip(1).collect();
    let mut conditions = Vec::new();
    if let [outer] = blocks.as_slice() {
        // No inner blocks: the conditions sit directly in the outer one.
        conditions.extend(enabled(outer).iter().map(|c| condition(c)));
    }
    for block in blocks.iter().skip(1) {
        match enabled(block).as_slice() {
            [] => {}
            [one] => conditions.push(condition(one)),
            _ => conditions.push(Condition::Unsupported("a group of several conditions".into())),
        }
    }
    SmartRule { match_all, conditions }
}

fn condition(xml: &str) -> Condition {
    let key = text(xml, "Key").unwrap_or_default();
    let op = text(xml, "Operator").and_then(|o| o.trim().parse::<u32>().ok());
    let criterion = text(xml, "Criterion").unwrap_or_default();
    let unsupported = || Condition::Unsupported(format!("{key} operator {op:?} {criterion:?}"));
    match (key.as_str(), op) {
        (KEY_KEYWORDS, Some(OP_CONTAINS)) if !criterion.is_empty() => Condition::KeywordContains(criterion),
        (KEY_RATING, Some(op)) => {
            let Ok(n) = criterion.trim().parse::<u8>() else { return unsupported() };
            match op {
                OP_EQUALS => Condition::Rating { min: n, max: n },
                OP_AT_LEAST => Condition::Rating { min: n, max: 5 },
                OP_GREATER => Condition::Rating { min: n.saturating_add(1), max: 5 },
                _ => unsupported(),
            }
        }
        (KEY_DATE, Some(OP_EQUALS)) => match criterion.trim().parse::<i32>() {
            Ok(year) if (1800..=9999).contains(&year) => Condition::Year(year),
            _ => unsupported(),
        },
        _ => unsupported(),
    }
}

/// The value of `name="…"` on the first `<tag …>`.
fn attr(xml: &str, tag: &str, name: &str) -> Option<String> {
    let start = xml.find(&format!("<{tag}"))?;
    attr_in(&xml[start + tag.len() + 1..], name)
}

/// The value of `name="…"` in a tag whose name has already been consumed.
fn attr_in(rest: &str, name: &str) -> Option<String> {
    let head = &rest[..rest.find('>')?];
    let at = head.find(&format!("{name}=\""))? + name.len() + 2;
    let len = head[at..].find('"')?;
    Some(unescape(&head[at..at + len]))
}

fn text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let len = xml[start..].find(&format!("</{tag}>"))?;
    Some(unescape(&xml[start..start + len]))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album(kind: &str, conditions: &[(&str, &str, u32, &str)]) -> String {
        let mut body = String::new();
        for (enabled, key, op, criterion) in conditions {
            body += &format!(
                "  <MatchOperator Kind=\"OR\">\r\n    <Condition Enabled=\"{enabled}\">\r\n      <Key>{key}</Key>\r\n      \
                 <Operator>{op}</Operator>\r\n      <Criterion>{criterion}</Criterion>\r\n    </Condition>\r\n  \
                 </MatchOperator>\r\n"
            );
        }
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<MatchOperator Kind=\"{kind}\">\r\n{body}</MatchOperator>"
        )
    }

    #[test]
    fn rules_from_a_real_catalog() {
        // "street4+ 2025": keyword street, rating greater than 3, captured in 2025.
        let xml = album(
            "AND",
            &[
                ("NO", "ANY", 6, ""),
                ("YES", KEY_KEYWORDS, 6, "street"),
                ("YES", KEY_RATING, 5, "3"),
                ("YES", KEY_DATE, 0, "2025"),
            ],
        );
        let rule = parse(&xml);
        assert!(rule.match_all);
        assert_eq!(
            rule.conditions,
            [Condition::KeywordContains("street".into()), Condition::Rating { min: 4, max: 5 }, Condition::Year(2025)]
        );
        assert!(rule.supported());
    }

    #[test]
    fn any_of_and_escapes() {
        let xml = album(
            "OR",
            &[("NO", "ANY", 6, ""), ("YES", KEY_KEYWORDS, 6, "cola"), ("YES", KEY_KEYWORDS, 6, "a &amp; b")],
        );
        let rule = parse(&xml);
        assert!(!rule.match_all);
        assert_eq!(
            rule.conditions,
            [Condition::KeywordContains("cola".into()), Condition::KeywordContains("a & b".into())]
        );
    }

    #[test]
    fn rating_operators() {
        let one = |op, n| parse(&album("AND", &[("YES", KEY_RATING, op, n)])).conditions;
        assert_eq!(one(0, "5"), [Condition::Rating { min: 5, max: 5 }]);
        assert_eq!(one(2, "4"), [Condition::Rating { min: 4, max: 5 }]);
        assert_eq!(one(5, "4"), [Condition::Rating { min: 5, max: 5 }]);
    }

    #[test]
    fn unknown_conditions_are_kept_as_unsupported() {
        let rule = parse(&album("AND", &[("YES", "IB_S_FILE_NAME", 6, "DSCF")]));
        assert!(!rule.supported());
        let rule = parse(&album("AND", &[("YES", KEY_RATING, 9, "4")]));
        assert!(!rule.supported());
    }
}
