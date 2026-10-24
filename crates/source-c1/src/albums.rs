//! Writing albums, smart albums and groups, ported from `legacy/c1cat-py/c1cat/collections.py`.
//!
//! A collection is a `ZCOLLECTION` row. An album's photos are linked twice: the variant
//! in `ZVARIANTINCOLLECTION` and its image in `ZIMAGEINCOLLECTION`, the image link kept
//! while any of its variants is in the album. A smart album stores its rule
//! (`ZSMARTCRITERIA`, see [`crate::smart`]), never members. The root (pk 1) holds the
//! tree and names Capture One's own collections, which are never changed here.

use std::time::{SystemTime, UNIX_EPOCH};

use c1model::change::collection_problem;
use c1model::source::Result;
use c1model::{Collection, CollectionChange, CollectionId, CollectionKind, SourceError};
use rusqlite::{Connection, OptionalExtension, params};

use crate::catalog::db_err;
use crate::{extract, smart};

const ROOT: i64 = 1;
const ENT_ALBUM: i64 = 2;
const ENT_SMART: i64 = 5;
const ENT_PROJECT: i64 = 7;
const ENT_GROUP: i64 = 8;
const ENT_IMAGE_IN_COLLECTION: i64 = 47;
const ENT_VARIANT_IN_COLLECTION: i64 = 48;

// Copied byte for byte from a collection Capture One made (CRLF line ends included).
const DEFAULT_SEARCH: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<MatchOperator Kind=\"AND\">\r\n  \
    <MatchOperator Kind=\"OR\">\r\n    <Condition Enabled=\"NO\">\r\n      <Key>ANY</Key>\r\n      \
    <Operator>6</Operator>\r\n      <Criterion></Criterion>\r\n    </Condition>\r\n  </MatchOperator>\r\n\
    </MatchOperator>";
const DEFAULT_FILTER: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<MatchOperator Kind=\"AND\" />";

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// The row of a collection: its own pk, or for one made in the app, the row written
/// with its uuid. `None` when there is no such row.
pub(crate) fn resolve(con: &Connection, id: &CollectionId) -> Result<Option<i64>> {
    let found = match id.local_uuid() {
        Some(uuid) => con.query_row("SELECT Z_PK FROM ZCOLLECTION WHERE ZUUID = ?1", [uuid], |r| r.get(0)),
        None => {
            let Ok(pk) = id.0.parse::<i64>() else { return Ok(None) };
            con.query_row("SELECT Z_PK FROM ZCOLLECTION WHERE Z_PK = ?1", [pk], |r| r.get(0))
        }
    };
    found.optional().map_err(db_err)
}

fn row(con: &Connection, id: &CollectionId) -> Result<i64> {
    resolve(con, id)?.ok_or_else(|| SourceError::Other(format!("collection {} is gone", id.0)))
}

/// The catalog's collections as they will be once `earlier` changes are written.
pub(crate) fn after(con: &Connection, earlier: &[CollectionChange]) -> Result<Vec<Collection>> {
    let mut all = extract::load_collections(con)?;
    for change in earlier {
        change.apply_to(&mut all);
    }
    Ok(all)
}

/// Why `change` cannot be written after `earlier`, or `None` if it can.
pub(crate) fn check(
    con: &Connection,
    earlier: &[CollectionChange],
    change: &CollectionChange,
) -> Result<Option<String>> {
    Ok(collection_problem(&after(con, earlier)?, change))
}

/// Writes one change. Collections made earlier in the same write are found by uuid.
pub(crate) fn write(con: &Connection, change: &CollectionChange) -> Result<()> {
    match change {
        CollectionChange::Create { id, name, kind, parent, rule } => {
            let uuid = id.local_uuid().ok_or_else(|| SourceError::Other(format!("{} is not a new id", id.0)))?;
            let parent = match parent {
                Some(p) => row(con, p)?,
                None => ROOT,
            };
            let (ent, criteria) = match (kind, rule) {
                (CollectionKind::Album, _) => (ENT_ALBUM, None),
                (CollectionKind::Group, _) => (ENT_GROUP, None),
                (CollectionKind::Smart, Some(rule)) => {
                    (ENT_SMART, Some(smart::criteria(rule).map_err(SourceError::Other)?))
                }
                _ => return Err(SourceError::Other(format!("cannot make a {kind:?} without what it needs"))),
            };
            create(con, uuid, name, ent, parent, criteria.as_deref())?;
        }
        CollectionChange::Rename { id, name } => {
            let pk = row(con, id)?;
            con.execute("UPDATE ZCOLLECTION SET ZNAME = ?1 WHERE Z_PK = ?2", params![name, pk]).map_err(db_err)?;
            touch(con, pk)?;
        }
        CollectionChange::SetRule { id, rule } => {
            let pk = row(con, id)?;
            let criteria = smart::criteria(rule).map_err(SourceError::Other)?;
            con.execute("UPDATE ZCOLLECTION SET ZSMARTCRITERIA = ?1 WHERE Z_PK = ?2", params![criteria, pk])
                .map_err(db_err)?;
            touch(con, pk)?;
        }
        CollectionChange::Move { id, parent } => {
            let pk = row(con, id)?;
            let parent = match parent {
                Some(p) => row(con, p)?,
                None => ROOT,
            };
            con.execute(
                "UPDATE ZCOLLECTION SET ZPARENT = ?1, ZCOLLECTIONINDEX = ?2 WHERE Z_PK = ?3",
                params![parent, next_index(con, parent)?, pk],
            )
            .map_err(db_err)?;
            follow(con, pk)?;
            touch(con, pk)?;
        }
        CollectionChange::Delete { id } => {
            let pk = row(con, id)?;
            // Children first, as the Python `album delete --force-children` does.
            let mut all = vec![pk];
            let mut i = 0;
            while i < all.len() {
                let mut stmt = con.prepare_cached("SELECT Z_PK FROM ZCOLLECTION WHERE ZPARENT = ?1").map_err(db_err)?;
                let children: Vec<i64> = stmt
                    .query_map([all[i]], |r| r.get(0))
                    .map_err(db_err)?
                    .collect::<rusqlite::Result<_>>()
                    .map_err(db_err)?;
                all.extend(children);
                i += 1;
            }
            for pk in all.into_iter().rev() {
                delete(con, pk)?;
            }
        }
    }
    Ok(())
}

/// Inserts a collection row as the Python `create` does; Z_PK is a rowid alias, so
/// SQLite assigns it.
fn create(con: &Connection, uuid: &str, name: &str, ent: i64, parent: i64, smart: Option<&str>) -> Result<i64> {
    let project = project_for(con, parent)?;
    let index = next_index(con, parent)?;
    let now = unix_now();
    con.execute(
        "INSERT INTO ZCOLLECTION (Z_ENT, ZUUID, ZNAME, ZPARENT, ZPROJECT, ZDOCUMENT,
           ZCOLLECTIONINDEX, ZCOLLAPSED, ZSORTORDER, ZDATECREATED, ZDATEMODIFIED,
           ZCONTENTMODIFIEDDATE, ZSERVEDBYCAPTUREPILOT, ZCAPTURECOLLECTIONOPTIONS,
           ZPROCESSCOLLECTIONOPTIONS, ZSEARCHCRITERIA, ZFILTERCRITERIA, ZSMARTCRITERIA)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 1, 'date', ?7, ?7, ?7, 0, 0, 0, ?8, ?9, ?10)",
        params![ent, uuid, name, parent, project, index, now, DEFAULT_SEARCH, DEFAULT_FILTER, smart],
    )
    .map_err(db_err)?;
    Ok(con.last_insert_rowid())
}

/// The project a collection in `parent` belongs to: the parent if it is one, else the
/// parent's own (the root, a project, for the top level).
fn project_for(con: &Connection, parent: i64) -> Result<i64> {
    let (parent_project, parent_ent): (Option<i64>, i64) = con
        .query_row("SELECT ZPROJECT, Z_ENT FROM ZCOLLECTION WHERE Z_PK = ?1", [parent], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(db_err)?
        .ok_or_else(|| SourceError::Other(format!("parent collection {parent} does not exist")))?;
    Ok(if parent_ent == ENT_PROJECT { parent } else { parent_project.filter(|p| *p != -1).unwrap_or(ROOT) })
}

/// After `parent` and after the last of its collections. Capture One numbers a
/// collection past its parent (the parent's number + 1 + its place among its siblings)
/// and leaves one numbered at or before its parent out of the tree.
fn next_index(con: &Connection, parent: i64) -> Result<i64> {
    con.query_row(
        "SELECT MAX(COALESCE((SELECT ZCOLLECTIONINDEX FROM ZCOLLECTION WHERE Z_PK = ?1), -1),
                    COALESCE((SELECT MAX(ZCOLLECTIONINDEX) FROM ZCOLLECTION WHERE ZPARENT = ?1), -1)) + 1",
        [parent],
        |r| r.get(0),
    )
    .map_err(db_err)
}

/// Brings everything under `pk` in line with where `pk` now is: each collection takes
/// its parent's project, and the children of each are numbered after it, in the order
/// they had.
fn follow(con: &Connection, pk: i64) -> Result<()> {
    let mut todo = vec![pk];
    while let Some(pk) = todo.pop() {
        let (parent, index): (i64, i64) = con
            .query_row("SELECT ZPARENT, COALESCE(ZCOLLECTIONINDEX, -1) FROM ZCOLLECTION WHERE Z_PK = ?1", [pk], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(db_err)?;
        con.execute("UPDATE ZCOLLECTION SET ZPROJECT = ?1 WHERE Z_PK = ?2", params![project_for(con, parent)?, pk])
            .map_err(db_err)?;
        let children: Vec<i64> = con
            .prepare_cached("SELECT Z_PK FROM ZCOLLECTION WHERE ZPARENT = ?1 ORDER BY ZCOLLECTIONINDEX, Z_PK")
            .and_then(|mut s| s.query_map([pk], |r| r.get(0))?.collect())
            .map_err(db_err)?;
        for (n, &child) in children.iter().enumerate() {
            con.execute(
                "UPDATE ZCOLLECTION SET ZCOLLECTIONINDEX = ?1 WHERE Z_PK = ?2",
                params![index + 1 + n as i64, child],
            )
            .map_err(db_err)?;
        }
        todo.extend(children);
    }
    Ok(())
}

/// Puts back after their parent the user's collections numbered at or before it, as
/// builds before this one wrote them; Capture One does not show them. They go after
/// their last sibling, in the order they had, and what is under them follows. Their
/// siblings keep their numbers. Returns how many were out of place.
pub(crate) fn repair_order(con: &Connection) -> Result<usize> {
    let mut fixed = 0;
    // A parent at a time: placing its collections again may mend others under them.
    while let Some(parent) = con
        .query_row(
            &format!("SELECT p.Z_PK {OUT_OF_ORDER} ORDER BY p.Z_PK LIMIT 1"),
            params![ENT_ALBUM, ENT_SMART, ENT_PROJECT, ENT_GROUP],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(db_err)?
    {
        let out: Vec<i64> = con
            .prepare(&format!("SELECT c.Z_PK {OUT_OF_ORDER} AND p.Z_PK = ?5 ORDER BY c.ZCOLLECTIONINDEX, c.Z_PK"))
            .and_then(|mut s| {
                s.query_map(params![ENT_ALBUM, ENT_SMART, ENT_PROJECT, ENT_GROUP, parent], |r| r.get(0))?.collect()
            })
            .map_err(db_err)?;
        let first = next_index(con, parent)?;
        for (n, &pk) in out.iter().enumerate() {
            con.execute("UPDATE ZCOLLECTION SET ZCOLLECTIONINDEX = ?1 WHERE Z_PK = ?2", params![first + n as i64, pk])
                .map_err(db_err)?;
            follow(con, pk)?;
        }
        fixed += out.len();
    }
    Ok(fixed)
}

/// The user's collections numbered at or before their parent (`c`, under `p`).
const OUT_OF_ORDER: &str = "FROM ZCOLLECTION c JOIN ZCOLLECTION p ON p.Z_PK = c.ZPARENT
     WHERE c.Z_ENT IN (?1, ?2, ?3, ?4) AND COALESCE(c.ZCOLLECTIONINDEX, -1) <= COALESCE(p.ZCOLLECTIONINDEX, -1)";

fn touch(con: &Connection, pk: i64) -> Result<()> {
    con.execute(
        "UPDATE ZCOLLECTION SET ZDATEMODIFIED = ?1, ZCONTENTMODIFIEDDATE = ?1 WHERE Z_PK = ?2",
        params![unix_now(), pk],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Removes one collection and everything that points at it: its photo links, the
/// selection Capture One remembers for it, and the document's "selected collection"
/// (which moves to All Images). The photos themselves are not touched.
fn delete(con: &Connection, pk: i64) -> Result<()> {
    for sql in [
        "DELETE FROM ZIMAGEINCOLLECTION WHERE ZCOLLECTION = ?1",
        "DELETE FROM ZVARIANTINCOLLECTION WHERE ZCOLLECTION = ?1",
        "DELETE FROM ZIMAGEINCOLLECTIONPROPERTIES WHERE ZCOLLECTION = ?1",
        "DELETE FROM ZSELECTEDVARIANTS WHERE ZSELECTEDINCOLLECTIONS = ?1",
        "DELETE FROM ZCOMPAREVARIANTS WHERE ZCOMPAREINCOLLECTIONS = ?1",
        "UPDATE ZDOCUMENTCONTENT SET ZSELECTEDCOLLECTION =
           (SELECT ZALLIMAGESCOLLECTION FROM ZCOLLECTION WHERE Z_PK = 1) WHERE ZSELECTEDCOLLECTION = ?1",
        "DELETE FROM ZCOLLECTION WHERE Z_PK = ?1",
    ] {
        con.execute(sql, [pk]).map_err(db_err)?;
    }
    Ok(())
}

pub(crate) fn is_member(con: &Connection, collection: i64, variant: i64) -> Result<bool> {
    con.prepare_cached("SELECT 1 FROM ZVARIANTINCOLLECTION WHERE ZCOLLECTION = ?1 AND ZVARIANT = ?2")
        .and_then(|mut s| s.exists(params![collection, variant]))
        .map_err(db_err)
}

/// Links variants (and their images) into an album.
pub(crate) fn add_members(con: &Connection, collection: i64, variants: &[i64]) -> Result<()> {
    for &variant in variants {
        let Some(image) = image_of(con, variant)? else { continue };
        con.prepare_cached("INSERT OR IGNORE INTO ZIMAGEINCOLLECTION (Z_ENT, ZCOLLECTION, ZIMAGE) VALUES (?1, ?2, ?3)")
            .and_then(|mut s| s.execute(params![ENT_IMAGE_IN_COLLECTION, collection, image]))
            .map_err(db_err)?;
        con.prepare_cached(
            "INSERT OR IGNORE INTO ZVARIANTINCOLLECTION (Z_ENT, ZCOLLECTION, ZVARIANT) VALUES (?1, ?2, ?3)",
        )
        .and_then(|mut s| s.execute(params![ENT_VARIANT_IN_COLLECTION, collection, variant]))
        .map_err(db_err)?;
    }
    touch(con, collection)
}

/// Unlinks variants. An image link (and its properties row) goes only when the image's
/// last variant leaves the album.
pub(crate) fn remove_members(con: &Connection, collection: i64, variants: &[i64]) -> Result<()> {
    for &variant in variants {
        let Some(image) = image_of(con, variant)? else { continue };
        con.prepare_cached("DELETE FROM ZVARIANTINCOLLECTION WHERE ZCOLLECTION = ?1 AND ZVARIANT = ?2")
            .and_then(|mut s| s.execute(params![collection, variant]))
            .map_err(db_err)?;
        let remaining: i64 = con
            .prepare_cached(
                "SELECT COUNT(*) FROM ZVARIANTINCOLLECTION vc JOIN ZVARIANT v ON v.Z_PK = vc.ZVARIANT
                  WHERE vc.ZCOLLECTION = ?1 AND v.ZIMAGE = ?2",
            )
            .and_then(|mut s| s.query_row(params![collection, image], |r| r.get(0)))
            .map_err(db_err)?;
        if remaining == 0 {
            for sql in [
                "DELETE FROM ZIMAGEINCOLLECTION WHERE ZCOLLECTION = ?1 AND ZIMAGE = ?2",
                "DELETE FROM ZIMAGEINCOLLECTIONPROPERTIES WHERE ZCOLLECTION = ?1 AND ZIMAGE = ?2",
            ] {
                con.prepare_cached(sql).and_then(|mut s| s.execute(params![collection, image])).map_err(db_err)?;
            }
        }
    }
    touch(con, collection)
}

fn image_of(con: &Connection, variant: i64) -> Result<Option<i64>> {
    con.prepare_cached("SELECT ZIMAGE FROM ZVARIANT WHERE Z_PK = ?1")
        .and_then(|mut s| s.query_row([variant], |r| r.get(0)).optional())
        .map_err(db_err)
}

/// Variant links with no image link, and image links with no variant link, in one
/// collection. Capture One's own catalogs carry both, so this is a measurement: a write
/// must not raise either.
pub(crate) fn dangling(con: &Connection, collection: i64) -> Result<(i64, i64)> {
    let variants = con
        .query_row(
            "SELECT COUNT(*) FROM ZVARIANTINCOLLECTION vc JOIN ZVARIANT v ON v.Z_PK = vc.ZVARIANT
              WHERE vc.ZCOLLECTION = ?1 AND NOT EXISTS (SELECT 1 FROM ZIMAGEINCOLLECTION ic
                    WHERE ic.ZCOLLECTION = vc.ZCOLLECTION AND ic.ZIMAGE = v.ZIMAGE)",
            [collection],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    let images = con
        .query_row(
            "SELECT COUNT(*) FROM ZIMAGEINCOLLECTION ic
              WHERE ic.ZCOLLECTION = ?1 AND NOT EXISTS (SELECT 1 FROM ZVARIANTINCOLLECTION vc
                    JOIN ZVARIANT v ON v.Z_PK = vc.ZVARIANT
                    WHERE vc.ZCOLLECTION = ic.ZCOLLECTION AND v.ZIMAGE = ic.ZIMAGE)",
            [collection],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    Ok((variants, images))
}

#[cfg(test)]
mod tests {
    use c1model::{Change, Condition, Op, PhotoId, SmartRule, SourceId};

    use super::*;
    use crate::write::tests::fixture;
    use crate::write::{plan, write as write_plan};

    fn pk(c: &Change) -> Result<i64> {
        Ok(c.photo.ext.parse().unwrap())
    }

    fn member(variant: i64, c: &CollectionId, add: bool) -> Change {
        let collection = c.clone();
        let op = if add { Op::AddToCollection { collection } } else { Op::RemoveFromCollection { collection } };
        Change { photo: PhotoId::new(&SourceId("t".into()), variant.to_string()), op }
    }

    fn create(id: &CollectionId, name: &str, kind: CollectionKind, parent: Option<&CollectionId>) -> CollectionChange {
        let rule = (kind == CollectionKind::Smart)
            .then(|| SmartRule { match_all: true, conditions: vec![Condition::KeywordContains("street".into())] });
        CollectionChange::Create { id: id.clone(), name: name.into(), kind, parent: parent.cloned(), rule }
    }

    fn links(con: &Connection, collection: i64) -> (Vec<i64>, Vec<i64>) {
        let list = |sql: &str| -> Vec<i64> {
            let mut stmt = con.prepare(sql).unwrap();
            stmt.query_map([collection], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
        };
        (
            list("SELECT ZVARIANT FROM ZVARIANTINCOLLECTION WHERE ZCOLLECTION = ?1 ORDER BY 1"),
            list("SELECT ZIMAGE FROM ZIMAGEINCOLLECTION WHERE ZCOLLECTION = ?1 ORDER BY 1"),
        )
    }

    /// Collections Capture One would leave out: numbered at or before their parent.
    fn out_of_order(con: &Connection) -> Vec<i64> {
        let mut stmt = con
            .prepare(
                "SELECT c.Z_PK FROM ZCOLLECTION c JOIN ZCOLLECTION p ON p.Z_PK = c.ZPARENT
                  WHERE COALESCE(c.ZCOLLECTIONINDEX, -1) <= COALESCE(p.ZCOLLECTIONINDEX, -1) ORDER BY 1",
            )
            .unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    }

    #[test]
    fn put_back_after_their_parent_as_capture_one_numbers_them() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        // As earlier builds left a catalog: a group made at the top level with an album
        // made in it and one moved into it, both numbered before it, and a group of
        // Capture One's own that is fine.
        con.execute_batch(
            "INSERT INTO ZCOLLECTION (Z_ENT, Z_PK, ZNAME, ZPARENT, ZPROJECT, ZDOCUMENT, ZCOLLECTIONINDEX) VALUES
               (8, 40, 'fuji', 1, 1, 1, 8), (2, 41, 'kept', 40, 1, 1, 9),
               (8, 913, 'konkursy', 1, 1, 1, 23), (2, 914, 'made', 913, 1, 1, 5), (8, 892, 'moved', 913, 1, 1, 7),
               (2, 893, 'inside moved', 892, 1, 1, 8);",
        )
        .unwrap();
        assert_eq!(out_of_order(&con), vec![892, 914]);
        let index = |pk: i64| -> i64 {
            con.query_row("SELECT ZCOLLECTIONINDEX FROM ZCOLLECTION WHERE Z_PK = ?1", [pk], |r| r.get(0)).unwrap()
        };
        assert_eq!(repair_order(&con).unwrap(), 2);
        assert_eq!(repair_order(&con).unwrap(), 0);
        assert_eq!(out_of_order(&con), Vec::<i64>::new());
        // In the order they had, after the group, with what is inside following.
        assert_eq!([index(913), index(914), index(892), index(893)], [23, 24, 25, 26]);
        assert_eq!([index(40), index(41)], [8, 9], "Capture One's own are not touched");

        // Any write mends a catalog left like that.
        con.execute("UPDATE ZCOLLECTION SET ZCOLLECTIONINDEX = 0 WHERE Z_PK = 914", []).unwrap();
        let p = plan(&con, &[], &[], &[member(1, &CollectionId("41".into()), true)], pk).unwrap();
        write_plan(&mut con, &p, pk).unwrap();
        assert_eq!(out_of_order(&con), Vec::<i64>::new());
    }

    #[test]
    fn made_filled_and_deleted_as_python_does() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        let (group, album, smart) =
            (CollectionId::new_local("g-1"), CollectionId::new_local("a-1"), CollectionId::new_local("s-1"));
        let made = [
            create(&group, "trips", CollectionKind::Group, None),
            create(&album, "japan", CollectionKind::Album, Some(&group)),
            create(&smart, "street", CollectionKind::Smart, None),
        ];
        let mut p =
            plan(&con, &[], &made, &[member(1, &album, true), member(2, &album, true), member(3, &smart, true)], pk)
                .unwrap();
        assert_eq!((p.apply.len(), p.conflicts.len()), (2, 1));
        p.collections = made.to_vec();
        assert_eq!(write_plan(&mut con, &p, pk).unwrap(), 5);

        let g = resolve(&con, &group).unwrap().unwrap();
        let a = resolve(&con, &album).unwrap().unwrap();
        let s = resolve(&con, &smart).unwrap().unwrap();
        let row = |pk: i64| -> (i64, String, i64, i64, i64, String, Option<String>) {
            con.query_row(
                "SELECT Z_ENT, ZNAME, ZPARENT, ZPROJECT, ZCOLLECTIONINDEX, ZSORTORDER, ZSMARTCRITERIA FROM ZCOLLECTION
                  WHERE Z_PK = ?1",
                [pk],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )
            .unwrap()
        };
        // Top level is under the root (a project), so its project is the root. Each is
        // numbered after its parent and its earlier siblings.
        assert_eq!(row(g), (ENT_GROUP, "trips".into(), ROOT, ROOT, 1, "date".into(), None));
        assert_eq!(row(a), (ENT_ALBUM, "japan".into(), g, ROOT, 2, "date".into(), None));
        let (ent, _, parent, _, index, _, xml) = row(s);
        assert_eq!((ent, parent, index), (ENT_SMART, ROOT, 2));
        assert_eq!(out_of_order(&con), Vec::<i64>::new());
        assert!(xml.unwrap().contains("<Criterion>street</Criterion>"));
        assert_eq!(links(&con, a), (vec![1, 2], vec![1, 2]));

        // Out again: the image link goes with its last variant.
        let a_id = CollectionId(a.to_string());
        let p = plan(&con, &[], &[], &[member(2, &a_id, false), member(3, &a_id, false)], pk).unwrap();
        assert_eq!((p.apply.len(), p.no_ops.len()), (1, 1));
        write_plan(&mut con, &p, pk).unwrap();
        assert_eq!(links(&con, a), (vec![1], vec![1]));

        // Deleting the group takes the album, its links, and what pointed at it.
        con.execute("UPDATE ZDOCUMENTCONTENT SET ZSELECTEDCOLLECTION = ?1", [a]).unwrap();
        let delete = [CollectionChange::Delete { id: CollectionId(g.to_string()) }];
        let mut p = plan(&con, &[], &delete, &[], pk).unwrap();
        p.collections = delete.to_vec();
        write_plan(&mut con, &p, pk).unwrap();
        assert_eq!(resolve(&con, &album).unwrap(), None);
        assert_eq!(resolve(&con, &group).unwrap(), None);
        assert_eq!(links(&con, a), (vec![], vec![]));
        let selected: Option<i64> =
            con.query_row("SELECT ZSELECTEDCOLLECTION FROM ZDOCUMENTCONTENT", [], |r| r.get(0)).unwrap();
        assert_eq!(selected, None, "the fixture's root names no All Images");
    }

    #[test]
    fn moved_into_a_project_and_out_with_what_is_inside() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        con.execute_batch(
            "INSERT INTO ZCOLLECTION (Z_ENT, Z_PK, ZNAME, ZPARENT, ZPROJECT, ZDOCUMENT, ZCOLLECTIONINDEX)
               VALUES (7, 30, 'proj', 1, 1, 1, 1);
             INSERT INTO ZCOLLECTION (Z_ENT, Z_PK, ZNAME, ZPARENT, ZPROJECT, ZDOCUMENT, ZCOLLECTIONINDEX)
               VALUES (2, 31, 'already in it', 30, 30, 1, 2);",
        )
        .unwrap();
        let (group, album) = (CollectionId::new_local("g-1"), CollectionId::new_local("a-1"));
        let made = [
            create(&group, "trips", CollectionKind::Group, None),
            create(&album, "japan", CollectionKind::Album, Some(&group)),
        ];
        let mut p = plan(&con, &[], &made, &[], pk).unwrap();
        p.collections = made.to_vec();
        write_plan(&mut con, &p, pk).unwrap();
        let (g, a) = (resolve(&con, &group).unwrap().unwrap(), resolve(&con, &album).unwrap().unwrap());
        let place = |con: &Connection, pk: i64| -> (i64, i64, i64) {
            con.query_row("SELECT ZPARENT, ZPROJECT, ZCOLLECTIONINDEX FROM ZCOLLECTION WHERE Z_PK = ?1", [pk], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap()
        };
        assert_eq!(place(&con, g), (ROOT, ROOT, 2));
        assert_eq!(place(&con, a), (g, ROOT, 3));

        let moves = |to: Option<&str>| {
            [CollectionChange::Move { id: CollectionId(g.to_string()), parent: to.map(|t| CollectionId(t.into())) }]
        };
        for (to, project, index) in [(Some("30"), 30, 3), (None, ROOT, 2)] {
            let changes = moves(to);
            assert_eq!(check(&con, &[], &changes[0]).unwrap(), None);
            let mut p = plan(&con, &[], &changes, &[], pk).unwrap();
            p.collections = changes.to_vec();
            write_plan(&mut con, &p, pk).unwrap();
            let parent = to.map_or(ROOT, |t| t.parse().unwrap());
            // After what was there, and the album inside follows: to the project, and
            // numbered after the group.
            assert_eq!(place(&con, g), (parent, project, index));
            assert_eq!(place(&con, a), (g, project, index + 1));
            assert_eq!(out_of_order(&con), Vec::<i64>::new());
        }
        // Into an album, or into itself, is refused.
        let bad = CollectionChange::Move { id: CollectionId(g.to_string()), parent: Some(CollectionId(a.to_string())) };
        assert!(check(&con, &[], &bad).unwrap().is_some());
    }

    #[test]
    fn capture_ones_own_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let Some(con) = fixture(&dir.path().join("c.cocatalogdb")) else { return };
        con.execute_batch(
            "INSERT INTO ZCOLLECTION (Z_ENT, Z_PK, ZNAME, ZPARENT, ZPROJECT, ZDOCUMENT) VALUES (8, 16, 'Recent Imports', 1, 1, 1);
             INSERT INTO ZCOLLECTION (Z_ENT, Z_PK, ZNAME, ZPARENT, ZPROJECT, ZDOCUMENT) VALUES (2, 20, 'import 1', 16, 1, 1);
             UPDATE ZCOLLECTION SET ZLASTIMPORTSCOLLECTION = 16 WHERE Z_PK = 1;",
        )
        .unwrap();
        let all = extract::load_collections(&con).unwrap();
        assert!(all.iter().find(|c| c.id.0 == "16").unwrap().system);
        assert!(!all.iter().find(|c| c.id.0 == "20").unwrap().system);
        let id = |s: &str| CollectionId(s.into());
        assert!(check(&con, &[], &CollectionChange::Delete { id: id("16") }).unwrap().is_some());
        assert!(check(&con, &[], &CollectionChange::Rename { id: id("16"), name: "x".into() }).unwrap().is_some());
        let into = create(&CollectionId::new_local("x"), "x", CollectionKind::Album, Some(&id("16")));
        assert!(check(&con, &[], &into).unwrap().is_some());
        // Albums Capture One made in it are the user's.
        assert!(check(&con, &[], &CollectionChange::Rename { id: id("20"), name: "trip".into() }).unwrap().is_none());
    }
}
