//! Keywords on photos and in the keyword tree, ported from `legacy/c1cat-py/c1cat/keywords.py`
//! and `cmd_keywords.py`.
//!
//! `ZVARIANTMETADATA.ZCONTENT_KEYWORDS` is one comma-separated string of
//! `<path>||<index>` tokens, the path's levels joined by `|`. A variant stores the same
//! set twice, in two orders: the adjustment layer sorted by path (bytes), the combined
//! layer by index. `index` numbers the keywords in the order they were added, densely.
//!
//! `ZKEYWORD` is a nested set: `ZLEFT` and `ZRIGHT` are each UNIQUE, a row CHECK wants
//! `ZLEFT < ZRIGHT`, and in a healthy catalog together they are exactly 1..2n.

use c1model::SourceError;
use c1model::source::Result;
use rusqlite::{Connection, OptionalExtension, params};

use crate::catalog::db_err;

/// `ZKEYWORD.Z_ENT` in the 16.8 schema.
const ENT_KEYWORD: i64 = 44;
/// `ZVARIANTLAYER.Z_ENT` of the combined layer, the one stored in index order.
const ENT_COMBINED: i64 = 20;

/// Paths with their indices, as stored.
pub(crate) type Pairs = Vec<(String, i64)>;

pub(crate) fn parse(serialised: Option<&str>) -> Result<Pairs> {
    let Some(text) = serialised else { return Ok(Vec::new()) };
    text.split(',')
        .filter(|t| !t.is_empty())
        .map(|token| {
            let (path, index) = token
                .rsplit_once("||")
                .ok_or_else(|| SourceError::Schema(format!("malformed keyword token {token:?}")))?;
            let index = index.parse().map_err(|_| SourceError::Schema(format!("malformed keyword token {token:?}")))?;
            Ok((path.to_owned(), index))
        })
        .collect()
}

/// Indices made dense, 0..n-1, keeping their order.
fn renumber(pairs: &[(String, i64)]) -> Pairs {
    let mut ordered = pairs.to_vec();
    ordered.sort_by_key(|(_, i)| *i);
    ordered.into_iter().enumerate().map(|(i, (path, _))| (path, i as i64)).collect()
}

/// The string for one layer: by path (bytes, as Capture One sorts: `b&w` before `best`)
/// or by index. `None` for no keywords.
pub(crate) fn serialise(pairs: &[(String, i64)], by_path: bool) -> Option<String> {
    if pairs.is_empty() {
        return None;
    }
    let mut pairs = renumber(pairs);
    if by_path {
        pairs.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    }
    Some(pairs.iter().map(|(path, i)| format!("{path}||{i}")).collect::<Vec<_>>().join(","))
}

/// Appends the paths not there yet, numbered after the last.
pub(crate) fn add_paths(pairs: &[(String, i64)], paths: &[String]) -> Pairs {
    let mut out = pairs.to_vec();
    let mut next = pairs.iter().map(|(_, i)| *i).max().map_or(0, |m| m + 1);
    for path in paths {
        if !out.iter().any(|(p, _)| p == path) {
            out.push((path.clone(), next));
            next += 1;
        }
    }
    renumber(&out)
}

pub(crate) fn remove_paths(pairs: &[(String, i64)], paths: &[String]) -> Pairs {
    let kept: Pairs = pairs.iter().filter(|(p, _)| !paths.contains(p)).cloned().collect();
    renumber(&kept)
}

/// A name with `,` or `|` would corrupt every keyword string it ends up in.
pub(crate) fn validate(path: &str) -> Result<()> {
    for name in path.split('|') {
        if name.trim().is_empty() {
            return Err(SourceError::Other(format!("keyword {path:?} has an empty level")));
        }
        if name.contains(',') {
            return Err(SourceError::Other(format!("keyword names may not contain ',': {path:?}")));
        }
    }
    Ok(())
}

/// The keyword string of the adjustment layer, as `extract` reads it.
pub(crate) fn stored(con: &Connection, variant_pk: i64) -> Result<Pairs> {
    let text: Option<String> = con
        .prepare_cached(
            "SELECT m.ZCONTENT_KEYWORDS FROM ZVARIANT v
               JOIN ZVARIANTMETADATA m ON m.ZLAYER = v.ZADJUSTMENTLAYER WHERE v.Z_PK = ?1",
        )
        .and_then(|mut s| s.query_row([variant_pk], |r| r.get(0)).optional())
        .map_err(db_err)?
        .flatten();
    parse(text.as_deref())
}

/// Writes one keyword set to the variant's adjustment and combined metadata rows, each in
/// its own order. Returns the rows touched.
pub(crate) fn write_variant(con: &Connection, variant_pk: i64, pairs: &[(String, i64)]) -> Result<usize> {
    let mut stmt = con
        .prepare_cached(
            "SELECT l.ZMETADATA, l.Z_ENT FROM ZVARIANTLAYER l
               JOIN ZVARIANT v ON l.Z_PK IN (v.ZADJUSTMENTLAYER, v.ZCOMBINEDSETTINGS)
              WHERE v.Z_PK = ?1",
        )
        .map_err(db_err)?;
    let layers: Vec<(Option<i64>, i64)> = stmt
        .query_map([variant_pk], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(db_err)?
        .collect::<rusqlite::Result<_>>()
        .map_err(db_err)?;
    let mut update =
        con.prepare_cached("UPDATE ZVARIANTMETADATA SET ZCONTENT_KEYWORDS = ?1 WHERE Z_PK = ?2").map_err(db_err)?;
    let mut touched = 0;
    for (metadata, ent) in layers {
        let Some(metadata) = metadata else { continue };
        update.execute(params![serialise(pairs, ent != ENT_COMBINED), metadata]).map_err(db_err)?;
        touched += 1;
    }
    Ok(touched)
}

/// The tree node for a path, made (with any missing levels above it) if needed.
/// Matches names exactly. Returns the node's `Z_PK`.
pub(crate) fn ensure_path(con: &Connection, path: &str) -> Result<i64> {
    validate(path)?;
    let mut parent: Option<i64> = None;
    for name in path.split('|') {
        let found: Option<i64> = match parent {
            None => {
                con.query_row("SELECT Z_PK FROM ZKEYWORD WHERE ZPARENT IS NULL AND ZNAME = ?1", [name], |r| r.get(0))
            }
            Some(p) => {
                con.query_row("SELECT Z_PK FROM ZKEYWORD WHERE ZPARENT = ?1 AND ZNAME = ?2", params![p, name], |r| {
                    r.get(0)
                })
            }
        }
        .optional()
        .map_err(db_err)?;
        parent = Some(match found {
            Some(pk) => pk,
            None => insert_node(con, name, parent)?,
        });
    }
    parent.ok_or_else(|| SourceError::Other(format!("empty keyword path {path:?}")))
}

/// Inserts one node and keeps the nested set whole.
fn insert_node(con: &Connection, name: &str, parent: Option<i64>) -> Result<i64> {
    let Some(parent) = parent else {
        // A new root goes past every existing interval: nothing moves.
        con.execute(
            "INSERT INTO ZKEYWORD (Z_ENT, ZNAME, ZPARENT, ZLEFT, ZRIGHT, ZISEXPORTABLE, ZDESCRIPTION)
             SELECT ?1, ?2, NULL, COALESCE(MAX(ZRIGHT), 0) + 1, COALESCE(MAX(ZRIGHT), 0) + 2, 1, NULL FROM ZKEYWORD",
            params![ENT_KEYWORD, name],
        )
        .map_err(db_err)?;
        return Ok(con.last_insert_rowid());
    };
    let lo: i64 = con
        .query_row("SELECT ZRIGHT FROM ZKEYWORD WHERE Z_PK = ?1", [parent], |r| r.get(0))
        .optional()
        .map_err(db_err)?
        .ok_or_else(|| SourceError::Other(format!("parent keyword {parent} does not exist")))?;
    let top: i64 = con.query_row("SELECT COALESCE(MAX(ZRIGHT), 0) FROM ZKEYWORD", [], |r| r.get(0)).map_err(db_err)?;
    let off = top + 1000;
    // Both ends of a row move in one statement, or CHECK(ZLEFT < ZRIGHT) fires midway;
    // and the values in flight must not meet untouched ones, or the UNIQUE indexes fire.
    // Lifting everything at or after the gap into a high, disjoint range, then bringing it
    // back two higher, satisfies both.
    con.execute(
        "UPDATE ZKEYWORD SET
           ZLEFT  = CASE WHEN ZLEFT  >= ?1 THEN ZLEFT  + ?2 ELSE ZLEFT  END,
           ZRIGHT = CASE WHEN ZRIGHT >= ?1 THEN ZRIGHT + ?2 ELSE ZRIGHT END
         WHERE ZRIGHT >= ?1",
        params![lo, off],
    )
    .map_err(db_err)?;
    con.execute(
        "UPDATE ZKEYWORD SET
           ZLEFT  = CASE WHEN ZLEFT  > ?1 THEN ZLEFT  - ?2 + 2 ELSE ZLEFT  END,
           ZRIGHT = CASE WHEN ZRIGHT > ?1 THEN ZRIGHT - ?2 + 2 ELSE ZRIGHT END
         WHERE ZRIGHT > ?1",
        params![top, off],
    )
    .map_err(db_err)?;
    con.execute(
        "INSERT INTO ZKEYWORD (Z_ENT, ZNAME, ZPARENT, ZLEFT, ZRIGHT, ZISEXPORTABLE, ZDESCRIPTION)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, NULL)",
        params![ENT_KEYWORD, name, parent, lo, lo + 1],
    )
    .map_err(db_err)?;
    Ok(con.last_insert_rowid())
}

/// Nested-set violations; empty means healthy.
pub(crate) fn check_tree(con: &Connection) -> Result<Vec<String>> {
    let mut stmt = con.prepare("SELECT Z_PK, ZNAME, ZPARENT, ZLEFT, ZRIGHT FROM ZKEYWORD").map_err(db_err)?;
    let rows: Vec<(i64, String, Option<i64>, i64, i64)> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .map_err(db_err)?
        .collect::<rusqlite::Result<_>>()
        .map_err(db_err)?;
    let mut problems = Vec::new();
    let mut ends: Vec<i64> = rows.iter().flat_map(|r| [r.3, r.4]).collect();
    ends.sort_unstable();
    if !ends.iter().enumerate().all(|(i, v)| *v == i as i64 + 1) {
        problems.push(format!("ZLEFT/ZRIGHT are not a contiguous 1..{}", 2 * rows.len()));
    }
    let spans: std::collections::HashMap<i64, (i64, i64)> = rows.iter().map(|r| (r.0, (r.3, r.4))).collect();
    for (pk, name, parent, left, right) in &rows {
        if left >= right {
            problems.push(format!("keyword {name:?} ({pk}) has ZLEFT >= ZRIGHT"));
        }
        if let Some(parent) = parent {
            match spans.get(parent) {
                None => problems.push(format!("keyword {name:?} ({pk}) points at missing parent {parent}")),
                Some((pl, pr)) if !(pl < left && right < pr) => {
                    problems.push(format!("keyword {name:?} ({pk}) sits outside its parent's interval"))
                }
                _ => {}
            }
        }
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, i64)]) -> Pairs {
        list.iter().map(|(p, i)| ((*p).to_owned(), *i)).collect()
    }

    #[test]
    fn each_layer_has_its_order() {
        let set = add_paths(&[], &["street".into(), "b&w".into(), "best|best2018".into()]);
        assert_eq!(serialise(&set, true).unwrap(), "b&w||1,best|best2018||2,street||0");
        assert_eq!(serialise(&set, false).unwrap(), "street||0,b&w||1,best|best2018||2");
        assert_eq!(serialise(&[], true), None);
    }

    #[test]
    fn adding_appends_and_removing_renumbers() {
        let stored = parse(Some("b&w||1,street||0,zoo||4")).unwrap();
        let added = add_paths(&stored, &["street".into(), "new".into()]);
        assert_eq!(added, pairs(&[("street", 0), ("b&w", 1), ("zoo", 2), ("new", 3)]));
        let removed = remove_paths(&added, &["b&w".into()]);
        assert_eq!(removed, pairs(&[("street", 0), ("zoo", 1), ("new", 2)]));
    }

    #[test]
    fn names_that_would_corrupt_strings_are_refused() {
        assert!(validate("a|b").is_ok());
        assert!(validate("a,b").is_err());
        assert!(validate("a||b").is_err());
        assert!(validate(" ").is_err());
    }
}
