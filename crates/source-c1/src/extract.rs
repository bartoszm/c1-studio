//! Reading every variant out of a catalog into the canonical model.
//!
//! A port of `legacy/c1cat-py/c1cat/mirror.py` (`_load_variants`, `_load_collections`,
//! `_load_keywords`). The differential tests compare the two on the real catalog.

use std::collections::HashMap;
use std::path::Path;

use c1model::source::Result;
use c1model::{
    AssetId, Collection, CollectionId, CollectionKind, ColorLabel, EditInfo, Exif, FileInfo, GeoOrigin, GeoPoint, Iptc,
    KeywordNode, KeywordPath, Photo, PhotoId, Place, SourceError, SourceId,
};
use rusqlite::Connection;

use crate::catalog::db_err;
use crate::config;
use crate::smart;

const ENT_STYLE_LAYER: i64 = 19;
const ENT_LOCAL_LAYER: i64 = 23;
const ROOT_COLLECTION_PK: i64 = 1;

/// Everything a full snapshot carries.
pub struct Extract {
    pub photos: Vec<Photo>,
    pub collections: Vec<Collection>,
    pub keywords: Vec<KeywordNode>,
}

/// User metadata lives on the variant's *adjustment* layer metadata row (mirrored onto
/// the combined layer). The default layer holds what was embedded in the file; never
/// read it for ratings or keywords.
const VARIANTS: &str = "
SELECT v.Z_PK, v.ZINDEX, v.ZISMODIFIED,
       i.Z_PK, i.ZIMAGEFILENAME, i.ZISTRASHED, i.ZEXP_DATE, i.ZIMPORTDATE,
       i.ZCAMERA_MAKE, i.ZCAMERA_MODEL, i.ZCAMERA_LENS, i.ZISO,
       i.ZEXP_APERTURE, i.ZEXP_SHUTTERSPEED, i.ZEXP_FOCALLENGTH,
       i.ZWIDTH, i.ZHEIGHT, i.ZFILE_SIZE, i.ZGPSLATITUDE, i.ZGPSLONGITUDE,
       p.ZWINROOT, p.ZRELATIVEPATH, p.ZISRELATIVE,
       m.ZBASIC_RATING, m.ZCOLOR_TAG_INDEX, m.ZCONTENT_KEYWORDS,
       m.ZCONTENT_DESCRIPTION, m.ZCONTENT_HEADLINE, m.ZSTATUS_TITLE,
       m.ZCONTACT_CREATOR, m.ZSTATUS_COPYRIGHTNOTICE,
       m.ZIMAGE_CITY, m.ZIMAGE_STATE, m.ZIMAGE_COUNTRY, m.ZIMAGE_LOCATION,
       m.ZIMAGE_ISOCOUNTRYCODE
  FROM ZVARIANT v
  JOIN ZIMAGE i           ON i.Z_PK   = v.ZIMAGE
  JOIN ZPATHLOCATION p    ON p.Z_PK   = i.ZIMAGELOCATION
  JOIN ZVARIANTMETADATA m ON m.ZLAYER = v.ZADJUSTMENTLAYER
";

pub fn extract(con: &Connection, catalog_dir: &Path, source: &SourceId) -> Result<Extract> {
    let collections = load_collections(con)?;
    let membership = load_membership(con)?;
    let layers = load_layers(con)?;
    let photos = load_variants(con, catalog_dir, source, &membership, &layers)?;
    let keywords = load_keywords(con, &photos)?;
    Ok(Extract { photos, collections, keywords })
}

/// Text columns use both NULL and '' for "nothing".
fn blank(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty())
}

/// `street||0,best|best2018||1` → paths in stored order.
pub(crate) fn parse_keywords(serialised: Option<&str>) -> Result<Vec<KeywordPath>> {
    let Some(text) = serialised else { return Ok(Vec::new()) };
    text.split(',')
        .filter(|t| !t.is_empty())
        .map(|token| {
            token
                .rsplit_once("||")
                .map(|(path, _index)| KeywordPath::from_pipe(path))
                .ok_or_else(|| SourceError::Schema(format!("malformed keyword token {token:?}")))
        })
        .collect()
}

fn secs_to_ms(secs: Option<f64>) -> Option<i64> {
    secs.filter(|s| s.is_finite()).map(|s| (s * 1000.0).round() as i64)
}

#[derive(Default)]
struct Layers {
    styles: HashMap<i64, Vec<String>>,
    local: HashMap<i64, u32>,
    retouch: HashMap<i64, u32>,
}

fn load_layers(con: &Connection) -> Result<Layers> {
    let mut layers = Layers::default();
    let mut stmt =
        con.prepare("SELECT ZVARIANT, Z_ENT, ZNAME FROM ZVARIANTLAYER WHERE Z_ENT IN (?1, ?2)").map_err(db_err)?;
    let mut rows = stmt.query([ENT_STYLE_LAYER, ENT_LOCAL_LAYER]).map_err(db_err)?;
    while let Some(r) = rows.next().map_err(db_err)? {
        let variant: i64 = r.get(0).map_err(db_err)?;
        if r.get::<_, i64>(1).map_err(db_err)? == ENT_STYLE_LAYER {
            if let Some(name) = blank(r.get(2).map_err(db_err)?) {
                layers.styles.entry(variant).or_default().push(name);
            }
        } else {
            *layers.local.entry(variant).or_default() += 1;
        }
    }
    let mut stmt = con.prepare("SELECT ZVARIANT FROM ZRETOUCHINGLAYER").map_err(db_err)?;
    let mut rows = stmt.query([]).map_err(db_err)?;
    while let Some(r) = rows.next().map_err(db_err)? {
        *layers.retouch.entry(r.get(0).map_err(db_err)?).or_default() += 1;
    }
    Ok(layers)
}

fn load_membership(con: &Connection) -> Result<HashMap<i64, Vec<CollectionId>>> {
    let mut members: HashMap<i64, Vec<CollectionId>> = HashMap::new();
    let mut stmt = con.prepare("SELECT ZVARIANT, ZCOLLECTION FROM ZVARIANTINCOLLECTION").map_err(db_err)?;
    let mut rows = stmt.query([]).map_err(db_err)?;
    while let Some(r) = rows.next().map_err(db_err)? {
        let variant: i64 = r.get(0).map_err(db_err)?;
        let coll: i64 = r.get(1).map_err(db_err)?;
        let list = members.entry(variant).or_default();
        let id = CollectionId(coll.to_string());
        if !list.contains(&id) {
            list.push(id);
        }
    }
    Ok(members)
}

fn load_variants(
    con: &Connection,
    catalog_dir: &Path,
    source: &SourceId,
    membership: &HashMap<i64, Vec<CollectionId>>,
    layers: &Layers,
) -> Result<Vec<Photo>> {
    let mut stmt = con.prepare(VARIANTS).map_err(db_err)?;
    let mut rows = stmt.query([]).map_err(db_err)?;
    let mut photos = Vec::new();
    while let Some(r) = rows.next().map_err(db_err)? {
        let text = |i: usize| -> Result<Option<String>> { r.get::<_, Option<String>>(i).map(blank).map_err(db_err) };
        let int = |i: usize| -> Result<Option<i64>> { r.get::<_, Option<i64>>(i).map_err(db_err) };
        let float = |i: usize| -> Result<Option<f64>> { r.get::<_, Option<f64>>(i).map_err(db_err) };

        let vpk: i64 = r.get(0).map_err(db_err)?;
        let filename = text(4)?.unwrap_or_default();
        let ext = filename.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        let path = config::original_path(
            &text(20)?.unwrap_or_default(),
            &text(21)?.unwrap_or_default(),
            &filename,
            int(22)?.unwrap_or(0) != 0,
            catalog_dir,
        );

        // Unrated is NULL; a legacy 0 means the same. Colour 0 is "no tag" too.
        let rating = int(23)?.filter(|&v| v > 0).map(|v| v.clamp(1, 5) as u8);
        let color = int(24)?.and_then(|v| u8::try_from(v).ok()).and_then(ColorLabel::from_index);

        let geo = match (config::parse_gps(text(18)?.as_deref()), config::parse_gps(text(19)?.as_deref())) {
            (Some(lat), Some(lon)) => Some(GeoPoint { lat, lon, alt: None, origin: GeoOrigin::Exif }),
            _ => None,
        };

        photos.push(Photo {
            id: PhotoId::new(source, vpk.to_string()),
            asset: AssetId(int(3)?.unwrap_or_default().to_string()),
            variant_index: int(1)?.unwrap_or(0).max(0) as u32,
            file: FileInfo {
                name: filename,
                ext,
                path: Some(path),
                size: int(17)?.and_then(|v| u64::try_from(v).ok()),
            },
            captured_at: secs_to_ms(float(6)?),
            imported_at: secs_to_ms(float(7)?),
            exif: Exif {
                camera_make: text(8)?,
                camera_model: text(9)?,
                lens: text(10)?,
                iso: int(11)?.and_then(|v| u32::try_from(v).ok()),
                aperture: float(12)?,
                shutter: float(13)?,
                focal: float(14)?,
                width: int(15)?.and_then(|v| u32::try_from(v).ok()),
                height: int(16)?.and_then(|v| u32::try_from(v).ok()),
            },
            rating,
            color,
            keywords: parse_keywords(text(25)?.as_deref())?,
            iptc: Iptc {
                description: text(26)?,
                headline: text(27)?,
                title: text(28)?,
                creator: text(29)?,
                copyright: text(30)?,
            },
            place: Place {
                city: text(31)?,
                state: text(32)?,
                country: text(33)?,
                location: text(34)?,
                country_code: text(35)?,
            },
            geo,
            collections: membership.get(&vpk).cloned().unwrap_or_default(),
            edit: EditInfo {
                edited: int(2)?.unwrap_or(0) != 0,
                styles: layers.styles.get(&vpk).cloned().unwrap_or_default(),
                local_layers: layers.local.get(&vpk).copied().unwrap_or(0),
                retouch_layers: layers.retouch.get(&vpk).copied().unwrap_or(0),
            },
            trashed: int(5)?.unwrap_or(0) != 0,
        });
    }
    Ok(photos)
}

fn kind_of(ent: i64) -> CollectionKind {
    match ent {
        2 => CollectionKind::Album,
        5 => CollectionKind::Smart,
        7 => CollectionKind::Project,
        8 => CollectionKind::Group,
        36 => CollectionKind::Folder,
        _ => CollectionKind::Other,
    }
}

fn load_collections(con: &Connection) -> Result<Vec<Collection>> {
    struct Row {
        pk: i64,
        ent: i64,
        name: String,
        parent: Option<i64>,
        smart: Option<String>,
    }
    let mut stmt =
        con.prepare("SELECT Z_PK, Z_ENT, ZNAME, ZPARENT, ZSMARTCRITERIA FROM ZCOLLECTION").map_err(db_err)?;
    let rows: Vec<Row> = stmt
        .query_map([], |r| {
            Ok(Row {
                pk: r.get(0)?,
                ent: r.get(1)?,
                name: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                parent: r.get(3)?,
                smart: r.get(4)?,
            })
        })
        .map_err(db_err)?
        .collect::<rusqlite::Result<_>>()
        .map_err(db_err)?;
    let by_pk: HashMap<i64, &Row> = rows.iter().map(|r| (r.pk, r)).collect();

    // The root collection (pk 1) and parent -1 both mean "top level".
    let real_parent =
        |parent: Option<i64>| parent.filter(|p| *p != -1 && *p != ROOT_COLLECTION_PK && by_pk.contains_key(p));
    let path_of = |row: &Row| {
        let mut names = vec![row.name.clone()];
        let mut seen = vec![row.pk];
        let mut parent = real_parent(row.parent);
        while let Some(pk) = parent {
            if seen.contains(&pk) {
                break;
            }
            seen.push(pk);
            let p = by_pk[&pk];
            names.push(p.name.clone());
            parent = real_parent(p.parent);
        }
        names.reverse();
        names.join(" / ")
    };

    // The root (pk 1, "[Collection]") holds the catalog's tree; it is not a collection.
    Ok(rows
        .iter()
        .filter(|row| row.pk != ROOT_COLLECTION_PK)
        .map(|row| Collection {
            id: CollectionId(row.pk.to_string()),
            name: row.name.clone(),
            kind: kind_of(row.ent),
            parent: real_parent(row.parent).map(|p| CollectionId(p.to_string())),
            path: path_of(row),
            rule: (kind_of(row.ent) == CollectionKind::Smart).then(|| smart::parse(row.smart.as_deref().unwrap_or(""))),
        })
        .collect())
}

/// The keyword library (`ZKEYWORD`, a nested set) with usage counts, including
/// keywords no photo uses.
fn load_keywords(con: &Connection, photos: &[Photo]) -> Result<Vec<KeywordNode>> {
    let mut stmt = con.prepare("SELECT Z_PK, ZNAME, ZPARENT FROM ZKEYWORD").map_err(db_err)?;
    let raw: HashMap<i64, (String, Option<i64>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, (r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get(2)?))))
        .map_err(db_err)?
        .collect::<rusqlite::Result<_>>()
        .map_err(db_err)?;

    let mut uses: HashMap<&KeywordPath, u32> = HashMap::new();
    for photo in photos {
        for kw in &photo.keywords {
            *uses.entry(kw).or_default() += 1;
        }
    }

    let mut nodes: Vec<KeywordNode> = raw
        .keys()
        .map(|&pk| {
            let mut segments = Vec::new();
            let mut cursor = Some(pk);
            while let Some(current) = cursor {
                let Some((name, parent)) = raw.get(&current) else { break };
                segments.push(name.clone());
                cursor = *parent;
                if segments.len() > 32 {
                    break; // a cycle; the tree is at most a few levels deep
                }
            }
            segments.reverse();
            let path = KeywordPath(segments);
            let n = uses.get(&path).copied().unwrap_or(0);
            KeywordNode { path, uses: n }
        })
        .collect();
    nodes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_parse_in_stored_order() {
        let kws = parse_keywords(Some("35mm||0,b&w||3,best|best2018||4")).unwrap();
        let paths: Vec<String> = kws.iter().map(KeywordPath::to_pipe).collect();
        assert_eq!(paths, ["35mm", "b&w", "best|best2018"]);
        assert!(parse_keywords(None).unwrap().is_empty());
        assert!(parse_keywords(Some("broken")).is_err());
    }
}
