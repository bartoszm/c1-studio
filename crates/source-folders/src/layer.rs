//! The user's edits, layered over what the file says, and the [`Photo`] the two make.
//!
//! A field the user set stays theirs, even when it equals what the file says, so a later
//! change to the file (another program's sidecar) does not undo it.

use std::collections::BTreeMap;

use c1model::{
    AssetId, Collection, CollectionId, CollectionKind, ColorLabel, EditInfo, FileInfo, GeoPoint, Iptc, KeywordPath,
    Photo, PhotoId, Place, SourceId, change::set_paths,
};
use serde::{Deserialize, Serialize};

use crate::definition::Definition;
use crate::store::{FileRec, FileState};

/// Fields the user set: `None` means "as the file says".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserLayer {
    #[serde(skip_serializing_if = "Option::is_none", with = "double")]
    pub rating: Option<Option<u8>>,
    #[serde(skip_serializing_if = "Option::is_none", with = "double")]
    pub color: Option<Option<ColorLabel>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<KeywordPath>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iptc: Option<Iptc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
    #[serde(skip_serializing_if = "Option::is_none", with = "double")]
    pub geo: Option<Option<GeoPoint>>,
    /// The albums the photo is in (never folders).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub albums: Vec<CollectionId>,
}

/// `Option<Option<T>>` in JSON: absent is `None`, `null` is `Some(None)`.
mod double {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, T: Serialize>(value: &Option<Option<T>>, s: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(inner) => inner.serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

impl UserLayer {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The layer after the user's photo went from `before` to `after`, given the photo the
    /// file alone gives (`file`). Fields set before stay set; a field set now is set.
    pub fn after(prev: Option<&UserLayer>, file: &Photo, after: &Photo) -> UserLayer {
        let prev = prev.cloned().unwrap_or_default();
        fn keep<T: Clone + PartialEq>(was: Option<T>, file: &T, now: &T) -> Option<T> {
            if was.is_some() || file != now { Some(now.clone()) } else { None }
        }
        UserLayer {
            rating: keep(prev.rating, &file.rating, &after.rating),
            color: keep(prev.color, &file.color, &after.color),
            keywords: keep(prev.keywords, &file.keywords, &after.keywords),
            iptc: keep(prev.iptc, &file.iptc, &after.iptc),
            place: keep(prev.place, &file.place, &after.place),
            geo: keep(prev.geo, &file.geo, &after.geo),
            albums: after.collections.iter().filter(|c| !is_folder(c)).cloned().collect(),
        }
    }
}

/// The collection id of a folder below a root: `folder:<root id>:<reldir>` (the root's own
/// when `reldir` is empty).
pub fn folder_id(root: &str, reldir: &str) -> CollectionId {
    if reldir.is_empty() {
        CollectionId(format!("folder:{root}:"))
    } else {
        CollectionId(format!("folder:{root}:{reldir}"))
    }
}

pub fn is_folder(id: &CollectionId) -> bool {
    id.0.starts_with("folder:")
}

/// The photo `rec` (photo id `ext`) makes with the user's `layer` on top. `def` gives the
/// root's path; a record whose root is not in it gets no path.
pub fn compose(source: &SourceId, def: &Definition, ext: &str, rec: &FileRec, layer: Option<&UserLayer>) -> Photo {
    let empty = UserLayer::default();
    let user = layer.unwrap_or(&empty);
    let m = &rec.meta;
    let path = def.root(&rec.root).map(|root| rec.path_under(&root.path).to_string_lossy().into_owned());
    let name = rec.name().to_owned();
    let file_ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let mut collections = vec![folder_id(&rec.root, rec.reldir())];
    collections.extend(user.albums.iter().cloned());
    Photo {
        id: PhotoId::new(source, ext),
        asset: AssetId(ext.to_owned()),
        variant_index: 0,
        file: FileInfo { name, ext: file_ext, path, size: Some(rec.size), offline: rec.state == FileState::Offline },
        captured_at: m.captured_at,
        imported_at: Some(rec.imported_at),
        exif: m.exif.clone(),
        rating: user.rating.unwrap_or(m.rating),
        color: user.color.unwrap_or(m.color),
        keywords: user.keywords.clone().unwrap_or_else(|| m.keywords.clone()),
        iptc: user.iptc.clone().unwrap_or_else(|| m.iptc.clone()),
        place: user.place.clone().unwrap_or_else(|| m.place.clone()),
        geo: user.geo.unwrap_or(m.geo),
        collections,
        edit: EditInfo::default(),
        trashed: false,
    }
}

/// The photo `rec` makes on its own, without the user's edits.
pub fn compose_file(source: &SourceId, def: &Definition, ext: &str, rec: &FileRec) -> Photo {
    compose(source, def, ext, rec, None)
}

/// Every shown folder (and every folder above one) as a [`CollectionKind::Folder`]
/// collection, each root the top of its tree. `system`: never renamed, moved or filled
/// from the app.
pub fn folder_collections<'a>(def: &Definition, files: impl IntoIterator<Item = &'a FileRec>) -> Vec<Collection> {
    let mut dirs: BTreeMap<(String, String), ()> = BTreeMap::new();
    for rec in files.into_iter().filter(|r| r.state != FileState::Detached) {
        let mut dir = rec.reldir();
        loop {
            if dirs.insert((rec.root.clone(), dir.to_owned()), ()).is_some() {
                break;
            }
            match dir.rsplit_once('/') {
                Some((up, _)) => dir = up,
                None if !dir.is_empty() => dir = "",
                None => break,
            }
        }
    }
    let mut out: Vec<Collection> = dirs
        .into_keys()
        .map(|(root, dir)| {
            let (name, parent) = if dir.is_empty() {
                (def.root(&root).map(|r| r.display_name()).unwrap_or_else(|| root.clone()), None)
            } else {
                let (up, name) = dir.rsplit_once('/').unwrap_or(("", dir.as_str()));
                (name.to_owned(), Some(folder_id(&root, up)))
            };
            Collection {
                id: folder_id(&root, &dir),
                name,
                kind: CollectionKind::Folder,
                parent,
                path: String::new(),
                rule: None,
                system: true,
            }
        })
        .collect();
    set_paths(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::rec;

    #[test]
    fn edits_lie_over_the_file_and_stay() {
        let def = Definition::new("t", &[std::path::PathBuf::from("/photos")]);
        let root = def.roots[0].id.clone();
        let mut r = rec(&root, "2024/a.jpg");
        r.meta.rating = Some(2);
        let src = def.source_id();
        let file = compose_file(&src, &def, "e1", &r);
        assert_eq!(file.rating, Some(2));
        assert_eq!(file.collections, [folder_id(&root, "2024")]);
        assert!(file.file.path.as_deref().unwrap().ends_with("a.jpg"));
        let mut edited = file.clone();
        edited.rating = Some(5);
        edited.collections.push(CollectionId("a1".into()));
        let layer = UserLayer::after(None, &compose_file(&src, &def, "e1", &r), &edited);
        assert_eq!(layer.rating, Some(Some(5)));
        assert_eq!(layer.keywords, None, "untouched fields follow the file");
        assert_eq!(layer.albums, [CollectionId("a1".into())]);
        // The file changes afterwards: the user's rating stays.
        r.meta.rating = Some(1);
        assert_eq!(compose(&src, &def, "e1", &r, Some(&layer)).rating, Some(5));
        // Set back to what the file says: still the user's.
        let back = UserLayer::after(Some(&layer), &compose_file(&src, &def, "e1", &r), &{
            let mut p = edited.clone();
            p.rating = Some(1);
            p
        });
        assert_eq!(back.rating, Some(Some(1)));
    }

    #[test]
    fn unrated_by_the_user_survives_json() {
        let layer = UserLayer { rating: Some(None), ..UserLayer::default() };
        let text = serde_json::to_string(&layer).unwrap();
        assert_eq!(text, r#"{"rating":null}"#);
        assert_eq!(serde_json::from_str::<UserLayer>(&text).unwrap(), layer);
        assert_eq!(serde_json::from_str::<UserLayer>("{}").unwrap(), UserLayer::default());
    }

    #[test]
    fn folders_form_a_tree_per_root() {
        let def = Definition::new("t", &[std::path::PathBuf::from("/photos/Trips")]);
        let root = def.roots[0].id.clone();
        let files = [rec(&root, "2024/rome/a.jpg"), rec(&root, "2024/b.jpg"), rec(&root, "c.jpg")];
        let folders = folder_collections(&def, &files);
        let paths: Vec<&str> = folders.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["Trips", "Trips / 2024", "Trips / 2024 / rome"]);
        assert!(folders.iter().all(|c| c.system && c.kind == CollectionKind::Folder));
    }
}
