//! Writing edits into the store: plans, apply, backups and restoring them.
//!
//! Plans and writes work on the photos as the user sees them ([`crate::layer::compose`]),
//! through `c1model::change::{plan_after_with, apply_with}`; what changed goes back into the
//! store as each photo's [`crate::layer::UserLayer`]. Before a write the user's part of the
//! store (layers and albums) is saved as a backup in `<work_dir>/backups`, newest five kept.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use c1model::change::{apply_with, collection_problem, plan_after_with};
use c1model::{
    ApplyPlan, ApplyReport, BackupInfo, Change, Collection, CollectionChange, KeywordPath, KeywordRemap, Photo,
    PhotoId, Result, SourceError, SourceId,
};
use serde::{Deserialize, Serialize};

use crate::FolderSource;
use crate::definition::Definition;
use crate::layer::{UserLayer, compose, compose_file};
use crate::store::{FileState, State};

/// How many backups are kept; older ones are deleted.
const KEEP_BACKUPS: usize = 5;

/// The backups' folder in the provider's `work_dir`.
const BACKUP_DIR: &str = "backups";

/// Every shown photo (records not detached) as the user sees it.
fn shown(source: &SourceId, def: &Definition, state: &State) -> BTreeMap<PhotoId, Photo> {
    state
        .files
        .iter()
        .filter(|(_, rec)| rec.state != FileState::Detached)
        .map(|(ext, rec)| {
            let photo = compose(source, def, ext, rec, state.user.get(ext));
            (photo.id.clone(), photo)
        })
        .collect()
}

pub(crate) fn plan_after(
    src: &FolderSource,
    remaps: &[KeywordRemap],
    collections: &[CollectionChange],
    changes: &[Change],
) -> Result<ApplyPlan> {
    let def = src.definition();
    let (photos, albums) = {
        let state = src.store().read();
        (shown(&def.source_id(), &def, &state), state.albums.clone())
    };
    // Folders are not in `albums`: a change putting a photo into one is refused as a
    // conflict, as one into an album that is gone.
    Ok(plan_after_with(&photos, &albums, remaps, collections, changes))
}

pub(crate) fn check_remap(
    src: &FolderSource,
    earlier: &[KeywordRemap],
    remap: &KeywordRemap,
) -> Result<Option<String>> {
    let carried: HashSet<KeywordPath> = {
        let state = src.store().read();
        state
            .files
            .iter()
            .filter(|(_, rec)| rec.state != FileState::Detached)
            .flat_map(|(ext, rec)| {
                state.user.get(ext).and_then(|l| l.keywords.as_ref()).unwrap_or(&rec.meta.keywords).iter().cloned()
            })
            .collect()
    };
    // Each keyword after the earlier remaps, and every keyword it sits under.
    let paths: BTreeSet<KeywordPath> = carried
        .into_iter()
        .map(|k| earlier.iter().fold(k, |k, r| r.path(&k)))
        .flat_map(|k| (1..=k.0.len()).map(move |n| KeywordPath(k.0[..n].to_vec())))
        .collect();
    if remap.map.iter().any(|(from, _)| paths.contains(from)) {
        return Ok(None);
    }
    let names: Vec<String> = remap.map.iter().map(|(from, _)| from.to_pipe()).collect();
    Ok(Some(format!("no longer on any photo: {}", names.join(", "))))
}

pub(crate) fn check_collection(
    src: &FolderSource,
    earlier: &[CollectionChange],
    change: &CollectionChange,
) -> Result<Option<String>> {
    let mut albums = src.store().read().albums.clone();
    for c in earlier {
        c.apply_to(&mut albums);
    }
    Ok(collection_problem(&albums, change))
}

pub(crate) fn apply(src: &FolderSource, plan: &ApplyPlan) -> Result<ApplyReport> {
    let _busy = src.busy.lock().unwrap();
    if plan.remaps.is_empty() && plan.collections.is_empty() && plan.apply.is_empty() {
        return Ok(ApplyReport::default());
    }
    let backup = make_backup(src, None, None)?;
    let def = src.definition();
    let source = def.source_id();
    let (applied, exported) = src.store().write(|w| {
        let before = shown(&source, &def, w.state());
        let mut photos = before.clone();
        let mut albums = w.state().albums.clone();
        let applied = apply_with(&mut photos, &mut albums, plan, || w.next_album_id())?;
        let mut exported = Vec::new();
        for (id, photo) in &photos {
            if before.get(id) == Some(photo) {
                continue;
            }
            let ext = id.ext.as_str();
            let Some(rec) = w.state().files.get(ext) else { continue };
            let layer = UserLayer::after(w.state().user.get(ext), &compose_file(&source, &def, ext, rec), photo);
            if rec.state == FileState::Present
                && let Some(root) = def.root(&rec.root)
            {
                exported.push((photo.clone(), rec.path_under(&root.path), rec.kind));
            }
            w.put_user(ext, Some(layer))?;
        }
        if albums != w.state().albums {
            w.set_albums(albums)?;
        }
        Ok((applied, exported))
    })?;
    let warnings =
        if def.write_xmp && !exported.is_empty() { crate::export::export(&def, &exported) } else { Vec::new() };
    Ok(ApplyReport { applied, backup: Some(backup.path), warnings })
}

pub(crate) fn backups(src: &FolderSource) -> Result<Vec<BackupInfo>> {
    Ok(list(&backup_dir(src)).into_iter().map(|b| b.info).collect())
}

pub(crate) fn restore(src: &FolderSource, id: &str) -> Result<BackupInfo> {
    let _busy = src.busy.lock().unwrap();
    let chosen = list(&backup_dir(src))
        .into_iter()
        .find(|b| b.info.id == id)
        .ok_or_else(|| SourceError::NotFound(format!("backup {id}")))?;
    let saved: BackupFile = serde_json::from_slice(&fs::read(&chosen.path)?).map_err(backup_err)?;
    let before = make_backup(src, Some(format!("before restoring {id}")), Some(id))?;
    src.store().write(|w| {
        let state = w.state();
        // Every photo with edits now or in the backup; one the store no longer has is skipped.
        let exts: BTreeSet<String> =
            state.user.keys().chain(saved.user.keys().filter(|ext| state.files.contains_key(*ext))).cloned().collect();
        for ext in exts {
            let layer = saved.user.get(&ext).cloned();
            if w.state().user.get(&ext) != layer.as_ref() {
                w.put_user(&ext, layer)?;
            }
        }
        if w.state().albums != saved.albums {
            w.set_albums(saved.albums.clone())?;
        }
        // Never lowered: an album id given once is not given again.
        w.raise_made(saved.made);
        Ok(())
    })?;
    Ok(before)
}

/// A backup file as written: the user's part of the store at one generation.
#[derive(Serialize)]
struct BackupOut<'a> {
    created_ms: u64,
    note: Option<&'a str>,
    store_uuid: &'a str,
    generation: u64,
    made: u64,
    albums: &'a [Collection],
    user: BTreeMap<&'a str, &'a UserLayer>,
}

/// A backup file read back whole, for restoring.
#[derive(Deserialize)]
struct BackupFile {
    made: u64,
    albums: Vec<Collection>,
    user: BTreeMap<String, UserLayer>,
}

/// What a listing needs from a backup file.
#[derive(Deserialize)]
struct BackupHead {
    created_ms: u64,
    #[serde(default)]
    note: Option<String>,
    store_uuid: String,
    generation: u64,
}

/// A backup found in the folder.
struct Listed {
    info: BackupInfo,
    path: PathBuf,
    store_uuid: String,
    generation: u64,
}

fn backup_err(e: impl std::fmt::Display) -> SourceError {
    SourceError::Other(format!("backup: {e}"))
}

fn backup_dir(src: &FolderSource) -> PathBuf {
    src.work_dir().join(BACKUP_DIR)
}

/// The backups in `dir`, newest first. Files that cannot be read are left out.
fn list(dir: &Path) -> Vec<Listed> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Listed> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("user.") && name.ends_with(".json")) {
                return None;
            }
            let path = e.path();
            let bytes = fs::read(&path).ok()?;
            let head: BackupHead = serde_json::from_slice(&bytes).ok()?;
            let info = BackupInfo {
                id: name,
                path: path.display().to_string(),
                created_ms: head.created_ms,
                size: bytes.len() as u64,
                note: head.note,
            };
            Some(Listed { info, path, store_uuid: head.store_uuid, generation: head.generation })
        })
        .collect();
    out.sort_by(|a, b| b.info.created_ms.cmp(&a.info.created_ms).then_with(|| b.info.id.cmp(&a.info.id)));
    out
}

/// A name for a new backup in `dir`, from the local time, and the time it stands for:
/// a millisecond later while one by that name exists.
fn fresh_name(dir: &Path) -> (u64, String) {
    let mut at = chrono::Local::now();
    loop {
        let name = format!("user.{}.json", at.format("%Y%m%dT%H%M%S%.3f"));
        if !dir.join(&name).exists() {
            return (at.timestamp_millis().max(0) as u64, name);
        }
        at += chrono::Duration::milliseconds(1);
    }
}

/// Saves the user's part of the store (layers, albums, the album counter) as a backup,
/// unless the newest backup was made at the store's current generation: nothing changed
/// since, so that one is returned. Keeps the newest [`KEEP_BACKUPS`], never deleting
/// `protect` (a backup about to be restored).
fn make_backup(src: &FolderSource, note: Option<String>, protect: Option<&str>) -> Result<BackupInfo> {
    let dir = backup_dir(src);
    let all = list(&dir);
    let state = src.store().read();
    if let Some(newest) = all.first()
        && newest.store_uuid == state.uuid
        && newest.generation == state.generation
    {
        return Ok(newest.info.clone());
    }
    fs::create_dir_all(&dir)?;
    let (created_ms, name) = fresh_name(&dir);
    let body = BackupOut {
        created_ms,
        note: note.as_deref(),
        store_uuid: &state.uuid,
        generation: state.generation,
        made: state.made,
        albums: &state.albums,
        user: state.user.iter().map(|(ext, layer)| (ext.as_str(), layer)).collect(),
    };
    let json = serde_json::to_vec(&body).map_err(backup_err)?;
    drop(state);
    let path = dir.join(&name);
    let tmp = dir.join(format!("{name}.tmp"));
    fs::write(&tmp, &json)?;
    fs::rename(&tmp, &path)?;
    for stale in list(&dir).into_iter().skip(KEEP_BACKUPS).filter(|b| Some(b.info.id.as_str()) != protect) {
        let _ = fs::remove_file(&stale.path);
    }
    Ok(BackupInfo { id: name, path: path.display().to_string(), created_ms, size: json.len() as u64, note })
}

#[cfg(test)]
mod tests {
    use c1model::{
        CollectionId, CollectionKind, ColorLabel, GeoOrigin, GeoPoint, Iptc, Op, PhotoSource, Place, SmartRule,
    };

    use super::*;
    use crate::layer::folder_id;
    use crate::store::{FileRec, tests::rec};

    struct Fixture {
        _dir: tempfile::TempDir,
        src: FolderSource,
        root: String,
    }

    fn ext(i: usize) -> String {
        format!("{i:016x}")
    }

    fn k(path: &str) -> KeywordPath {
        KeywordPath::from_pipe(path)
    }

    /// A source with `n` photos in `2024/` of its one root, each record as `edit` leaves it.
    fn fixture_with(n: usize, edit: impl Fn(usize, &mut FileRec)) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let def = Definition::new("t", &[dir.path().join("photos")]);
        let location = dir.path().join("t.c1folders");
        def.save(&location).unwrap();
        let src = FolderSource::open(&location, &dir.path().join("work")).unwrap();
        let root = def.roots[0].id.clone();
        src.store()
            .write(|w| {
                for i in 0..n {
                    let mut r = rec(&root, &format!("2024/{i}.jpg"));
                    edit(i, &mut r);
                    w.put_file(&ext(i), r)?;
                }
                Ok(())
            })
            .unwrap();
        Fixture { _dir: dir, src, root }
    }

    fn fixture(n: usize) -> Fixture {
        fixture_with(n, |_, _| {})
    }

    impl Fixture {
        fn id(&self, i: usize) -> PhotoId {
            PhotoId::new(&self.src.definition().source_id(), ext(i))
        }

        /// Photo `i` as the user sees it.
        fn photo(&self, i: usize) -> Photo {
            let def = self.src.definition();
            let state = self.src.store().read();
            compose(&def.source_id(), &def, &ext(i), &state.files[&ext(i)], state.user.get(&ext(i)))
        }

        fn layer(&self, i: usize) -> Option<UserLayer> {
            self.src.store().read().user.get(&ext(i)).cloned()
        }

        fn change(&self, i: usize, op: Op) -> Change {
            Change { photo: self.id(i), op }
        }

        /// Plans and applies, expecting every change to apply.
        fn write_after(
            &self,
            remaps: &[KeywordRemap],
            collections: &[CollectionChange],
            changes: &[Change],
        ) -> ApplyReport {
            let plan = self.src.plan_after(remaps, collections, changes).unwrap();
            assert!(plan.conflicts.is_empty() && plan.no_ops.is_empty(), "{plan:?}");
            self.src.apply(&plan).unwrap()
        }

        fn write(&self, changes: &[Change]) -> ApplyReport {
            self.write_after(&[], &[], changes)
        }

        fn albums(&self) -> Vec<Collection> {
            self.src.store().read().albums.clone()
        }
    }

    fn rating(base: Option<u8>, value: Option<u8>) -> Op {
        Op::SetRating { base, value }
    }

    fn new_album(
        id: &CollectionId,
        name: &str,
        kind: CollectionKind,
        parent: Option<&CollectionId>,
    ) -> CollectionChange {
        CollectionChange::Create { id: id.clone(), name: name.into(), kind, parent: parent.cloned(), rule: None }
    }

    #[test]
    fn a_rating_lands_replans_as_a_no_op_and_stale_bases_conflict() {
        let f = fixture_with(3, |i, r| {
            if i == 2 {
                r.state = FileState::Detached;
            }
        });
        let report = f.write(&[f.change(0, rating(None, Some(5)))]);
        assert_eq!(report.applied, 1);
        assert!(report.backup.is_some());
        assert_eq!(f.photo(0).rating, Some(5));
        assert_eq!(f.layer(0).unwrap().rating, Some(Some(5)));
        assert_eq!(f.layer(1), None, "an untouched photo gets no layer");
        let state = f.src.store().read();
        assert_eq!(state.changed[&ext(0)], state.generation, "the photo is sent again");
        drop(state);

        let plan = f.src.plan(&[f.change(0, rating(None, Some(5)))]).unwrap();
        assert_eq!((plan.no_ops.len(), plan.apply.len()), (1, 0));
        let plan = f.src.plan(&[f.change(0, rating(Some(1), Some(2)))]).unwrap();
        assert_eq!(plan.conflicts.len(), 1);
        let plan = f.src.plan(&[f.change(2, rating(None, Some(2)))]).unwrap();
        assert_eq!(plan.conflicts[0].current, "deleted", "a detached record is not shown, so not editable");

        // Changed outside the plan's view after it was made: the plan is refused whole.
        let plan = f.src.plan(&[f.change(0, rating(Some(5), Some(4))), f.change(1, rating(None, Some(1)))]).unwrap();
        assert_eq!(plan.apply.len(), 2);
        f.src
            .store()
            .write(|w| w.put_user(&ext(0), Some(UserLayer { rating: Some(Some(2)), ..UserLayer::default() })))
            .unwrap();
        assert!(matches!(f.src.apply(&plan), Err(SourceError::StalePlan)));
        assert_eq!(f.photo(0).rating, Some(2));
        assert_eq!(f.photo(1).rating, None, "nothing of a refused plan lands");

        let backups = f.src.backups().unwrap().len();
        assert_eq!(f.src.apply(&ApplyPlan::default()).unwrap(), ApplyReport::default());
        assert_eq!(f.src.backups().unwrap().len(), backups, "an empty plan makes no backup");
    }

    #[test]
    fn keywords_are_added_removed_and_remapped() {
        let f = fixture_with(3, |i, r| {
            r.meta.keywords = match i {
                0 => vec![k("street")],
                1 => vec![k("streets|night")],
                _ => vec![],
            }
        });
        f.write(&[
            f.change(0, Op::AddKeywords { paths: vec![k("b&w")] }),
            f.change(0, Op::RemoveKeywords { paths: vec![k("street")] }),
            f.change(2, Op::AddKeywords { paths: vec![k("b&w")] }),
        ]);
        assert_eq!(f.photo(0).keywords, [k("b&w")]);
        assert_eq!(f.photo(2).keywords, [k("b&w")]);
        assert_eq!(f.layer(0).unwrap().keywords, Some(vec![k("b&w")]));
        assert_eq!(f.layer(1), None);

        let join = KeywordRemap::join(vec![k("streets")], k("street"));
        assert_eq!(f.src.check_remap(&[], &join).unwrap(), None);
        let report = f.write_after(std::slice::from_ref(&join), &[], &[]);
        assert_eq!(report.applied, 1);
        assert_eq!(f.photo(1).keywords, [k("street|night")]);
        assert_eq!(f.photo(0).keywords, [k("b&w")]);

        let why = f.src.check_remap(&[], &join).unwrap().expect("nothing carries “streets” any more");
        assert!(why.contains("streets"), "{why}");
        // A keyword that only sits above a photo's keyword is still there to rename.
        let rename = KeywordRemap::rename(k("street"), "road");
        assert_eq!(f.src.check_remap(&[], &rename).unwrap(), None);
        assert!(f.src.check_remap(std::slice::from_ref(&rename), &rename).unwrap().is_some());
        f.write_after(&[KeywordRemap::rename(k("b&w"), "mono")], &[], &[]);
        assert_eq!(f.photo(0).keywords, [k("mono")]);
        assert_eq!(f.photo(2).keywords, [k("mono")]);
    }

    #[test]
    fn albums_are_made_filled_renamed_and_deleted() {
        let f = fixture(2);
        let group = CollectionId::new_local("g");
        let album = CollectionId::new_local("a");
        let report = f.write_after(
            &[],
            &[
                new_album(&group, "Trips", CollectionKind::Group, None),
                new_album(&album, "Rome", CollectionKind::Album, Some(&group)),
            ],
            &[f.change(0, Op::AddToCollection { collection: album.clone() })],
        );
        assert_eq!(report.applied, 3);
        let (a1, a2) = (CollectionId("a1".into()), CollectionId("a2".into()));
        let albums = f.albums();
        let paths: Vec<(&CollectionId, &str)> = albums.iter().map(|c| (&c.id, c.path.as_str())).collect();
        assert_eq!(paths, [(&a1, "Trips"), (&a2, "Trips / Rome")]);
        assert_eq!(f.photo(0).collections, [folder_id(&f.root, "2024"), a2.clone()]);
        assert_eq!(f.layer(0).unwrap().albums, std::slice::from_ref(&a2));
        assert_eq!(f.src.store().read().made, 2);

        let rename = CollectionChange::Rename { id: a2.clone(), name: "Roma".into() };
        assert_eq!(f.src.check_collection(&[], &rename).unwrap(), None);
        let earlier = [CollectionChange::Delete { id: a1.clone() }];
        assert!(f.src.check_collection(&earlier, &rename).unwrap().is_some(), "deleted by an earlier change");
        f.write_after(&[], &[rename], &[]);
        assert_eq!(f.albums()[1].path, "Trips / Roma");

        // Only albums take photos: not a folder, a group or a smart album.
        let smart = CollectionChange::Create {
            id: CollectionId::new_local("s"),
            name: "Best".into(),
            kind: CollectionKind::Smart,
            parent: None,
            rule: Some(SmartRule { match_all: true, conditions: vec![] }),
        };
        f.write_after(&[], &[smart], &[]);
        let a3 = CollectionId("a3".into());
        for to in [folder_id(&f.root, "2024"), folder_id(&f.root, ""), a1.clone(), a3.clone()] {
            let plan = f.src.plan(&[f.change(1, Op::AddToCollection { collection: to.clone() })]).unwrap();
            assert_eq!(plan.conflicts.len(), 1, "{to:?}: {plan:?}");
        }
        let plan = f.src.plan(&[f.change(1, Op::AddToCollection { collection: a3 })]).unwrap();
        assert!(plan.conflicts[0].current.contains("smart album"), "{plan:?}");

        // Deleting the group takes the album in it, and the photo out of it.
        f.write_after(&[], &[CollectionChange::Delete { id: a1 }], &[]);
        assert_eq!(f.albums().len(), 1);
        assert_eq!(f.photo(0).collections, [folder_id(&f.root, "2024")]);
        assert_eq!(f.layer(0), None, "an emptied layer is dropped");
        f.write_after(&[], &[new_album(&CollectionId::new_local("n"), "Next", CollectionKind::Album, None)], &[]);
        assert_eq!(f.albums()[1].id.0, "a4", "ids are never given twice");
    }

    #[test]
    fn geo_place_iptc_and_color_are_set() {
        let f = fixture(1);
        let geo = GeoPoint { lat: 51.11, lon: 17.03, alt: None, origin: GeoOrigin::Manual };
        let place = Place { city: Some("Wrocław".into()), country: Some("Poland".into()), ..Place::default() };
        let iptc = Iptc { title: Some("Market".into()), creator: Some("B".into()), ..Iptc::default() };
        f.write(&[
            f.change(0, Op::SetGeo { base: None, value: Some(geo) }),
            f.change(0, Op::SetPlace { base: Place::default(), value: place.clone() }),
            f.change(0, Op::SetIptc { base: Iptc::default(), value: iptc.clone() }),
            f.change(0, Op::SetColor { base: None, value: Some(ColorLabel::Red) }),
        ]);
        let photo = f.photo(0);
        assert_eq!(photo.geo, Some(geo));
        assert_eq!(photo.place, place);
        assert_eq!(photo.iptc, iptc);
        assert_eq!(photo.color, Some(ColorLabel::Red));
        let layer = f.layer(0).unwrap();
        assert_eq!(layer.geo, Some(Some(geo)));
        assert_eq!(layer.place, Some(place));
        assert_eq!(layer.iptc, Some(iptc));
        assert_eq!(layer.rating, None, "untouched fields follow the file");
        f.write(&[f.change(0, Op::SetGeo { base: Some(geo), value: None })]);
        assert_eq!(f.photo(0).geo, None);
        assert_eq!(f.layer(0).unwrap().geo, Some(None), "cleared by the user, not following the file");
    }

    #[test]
    fn the_users_edits_win_over_the_file() {
        let f = fixture(2);
        f.write(&[f.change(0, rating(None, Some(4)))]);
        f.src
            .store()
            .write(|w| {
                for i in 0..2 {
                    let mut r = w.state().files[&ext(i)].clone();
                    r.meta.rating = Some(2);
                    r.meta.keywords = vec![k("new")];
                    w.put_file(&ext(i), r)?;
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(f.photo(0).rating, Some(4));
        assert_eq!(f.photo(1).rating, Some(2));
        assert_eq!(f.photo(0).keywords, [k("new")], "what the user did not touch follows the file");
    }

    /// The user layers a backup file holds.
    fn saved_layers(path: &str) -> BTreeMap<String, UserLayer> {
        serde_json::from_slice::<BackupFile>(&fs::read(path).unwrap()).unwrap().user
    }

    #[test]
    fn backups_are_made_reused_and_pruned() {
        let f = fixture(1);
        assert!(f.src.backups().unwrap().is_empty());
        let report = f.write(&[f.change(0, rating(None, Some(5)))]);
        let list = f.src.backups().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(report.backup.as_deref(), Some(list[0].path.as_str()));
        assert!(list[0].id.starts_with("user.") && list[0].id.ends_with(".json"), "{}", list[0].id);
        assert!(saved_layers(&list[0].path).is_empty(), "made before the write");

        let now = make_backup(&f.src, None, None).unwrap();
        assert_ne!(now.id, list[0].id, "the store changed since");
        assert_eq!(saved_layers(&now.path)[&ext(0)].rating, Some(Some(5)));
        assert_eq!(make_backup(&f.src, None, None).unwrap().id, now.id, "nothing changed: reused");
        let report = f.write(&[f.change(0, rating(Some(5), Some(3)))]);
        assert_eq!(report.backup.as_deref(), Some(now.path.as_str()), "the write reuses it too");
        assert_eq!(f.src.backups().unwrap().len(), 2);

        for n in 0..6u8 {
            let from = f.photo(0).rating;
            f.write(&[f.change(0, rating(from, Some(1 + n % 2)))]);
        }
        let list = f.src.backups().unwrap();
        assert_eq!(list.len(), KEEP_BACKUPS);
        assert!(list.windows(2).all(|w| w[0].created_ms > w[1].created_ms), "newest first");
        let files = fs::read_dir(backup_dir(&f.src)).unwrap().count();
        assert_eq!(files, KEEP_BACKUPS, "older files are deleted, no temporary ones left");
    }

    #[test]
    fn restore_brings_old_values_back_and_can_be_undone() {
        let f = fixture(2);
        let report = f.write(&[f.change(0, rating(None, Some(5)))]);
        let first = f.src.backups().unwrap()[0].clone();
        assert_eq!(report.backup.as_deref(), Some(first.path.as_str()));
        let album = CollectionId::new_local("a");
        f.write_after(
            &[],
            &[new_album(&album, "Rome", CollectionKind::Album, None)],
            &[f.change(1, Op::AddToCollection { collection: album.clone() })],
        );
        let a1 = CollectionId("a1".into());
        assert_eq!(f.photo(1).collections[1], a1);

        let undo = f.src.restore(&first.id).unwrap();
        assert_eq!(undo.note, Some(format!("before restoring {}", first.id)));
        assert!(f.src.backups().unwrap().iter().any(|b| b.id == undo.id), "the backup made first is listed");
        assert_eq!(f.photo(0).rating, None);
        assert_eq!(f.photo(1).collections, [folder_id(&f.root, "2024")]);
        assert!(f.albums().is_empty());
        assert_eq!(f.src.store().read().made, 1, "the album counter is never lowered");
        assert!(matches!(f.src.restore("user.nothing.json"), Err(SourceError::NotFound(_))));

        f.src.restore(&undo.id).unwrap();
        assert_eq!(f.photo(0).rating, Some(5));
        assert_eq!(f.photo(1).collections[1], a1);
        assert_eq!(f.albums()[0].name, "Rome");

        // A photo the store no longer has is skipped.
        let before_delete = make_backup(&f.src, None, None).unwrap();
        f.src.store().write(|w| w.remove_file(&ext(0))).unwrap();
        f.src.restore(&before_delete.id).unwrap();
        assert!(!f.src.store().read().user.contains_key(&ext(0)));
        assert_eq!(f.photo(1).collections[1], a1);
    }

    #[test]
    fn passes_the_write_conformance_check() {
        let f = fixture(3);
        assert_eq!(c1model::conformance::check_write(&f.src, &f.id(1)), Vec::<String>::new());
    }

    #[test]
    fn passes_the_backup_conformance_check() {
        let f = fixture(3);
        assert_eq!(c1model::conformance::check_backup(&f.src, &f.id(1)), Vec::<String>::new());
        assert_eq!(f.photo(1).rating, None, "probe restored");
    }
}
