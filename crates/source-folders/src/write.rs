//! Writing edits into the store: plans, apply, backups and restoring them.
//!
//! Plans and writes work on the photos as the user sees them ([`crate::layer::compose`]),
//! through `c1model::change::{plan_after_with, apply_with}`; what changed goes back into the
//! store as each photo's [`crate::layer::UserLayer`]. Before a write the user's part of the
//! store (layers and albums) is saved as a backup in `<work_dir>/backups`, newest five kept.
//! After it, the XMP export (if the definition asks for one) writes the files, and their
//! records are brought up to date with what it wrote.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use c1model::change::{apply_with, collection_problem, plan_after_with};
use c1model::{
    ApplyPlan, ApplyReport, BackupInfo, Change, Collection, CollectionChange, ImageTier, KeywordPath, KeywordRemap,
    Photo, PhotoId, Result, SourceError, SourceId,
};
use serde::{Deserialize, Serialize};

use crate::FolderSource;
use crate::definition::Definition;
use crate::export::{Written, export};
use crate::ids::{self, ContentKey};
use crate::layer::{UserLayer, compose, compose_file};
use crate::scan::mtime_ns;
use crate::store::{FileState, ImageRec, State};

/// How many backups are kept; older ones are deleted.
const KEEP_BACKUPS: usize = 5;

/// The backups' folder in the provider's `work_dir`.
const BACKUP_DIR: &str = "backups";

/// The shown photos (records not detached), as the user sees them, that `remaps`,
/// `collections` and `changes` can alter: those the changes name, those carrying a keyword
/// a remap moves and, when an album is deleted, those in any album. Only these are
/// composed, so a write costs what it touches, not the whole library.
fn touched(
    source: &SourceId,
    def: &Definition,
    state: &State,
    remaps: &[KeywordRemap],
    collections: &[CollectionChange],
    changes: &[Change],
) -> BTreeMap<PhotoId, Photo> {
    let named: BTreeSet<&str> =
        changes.iter().filter(|c| &c.photo.source == source).map(|c| c.photo.ext.as_str()).collect();
    let deletes = collections.iter().any(|c| matches!(c, CollectionChange::Delete { .. }));
    let compose_one = |ext: &str, rec| {
        let photo = compose(source, def, ext, rec, state.user.get(ext));
        (photo.id.clone(), photo)
    };
    if remaps.is_empty() && !deletes {
        return named
            .into_iter()
            .filter_map(|ext| state.files.get_key_value(ext))
            .filter(|(_, rec)| rec.state != FileState::Detached)
            .map(|(ext, rec)| compose_one(ext, rec))
            .collect();
    }
    state
        .files
        .iter()
        .filter(|(_, rec)| rec.state != FileState::Detached)
        .filter(|(ext, rec)| {
            let layer = state.user.get(ext.as_str());
            // A keyword a remap leaves alone is the same for the next one: checking the
            // keyword as it is now against every remap is enough.
            let keywords = layer.and_then(|l| l.keywords.as_ref()).unwrap_or(&rec.meta.keywords);
            named.contains(ext.as_str())
                || keywords.iter().any(|k| remaps.iter().any(|r| r.moves(k)))
                || (deletes && layer.is_some_and(|l| !l.albums.is_empty()))
        })
        .map(|(ext, rec)| compose_one(ext, rec))
        .collect()
}

pub(crate) fn plan_after(
    src: &FolderSource,
    remaps: &[KeywordRemap],
    collections: &[CollectionChange],
    changes: &[Change],
) -> Result<ApplyPlan> {
    let def = src.definition();
    // Only the photos the changes name are planned against.
    let (photos, albums) = {
        let state = src.store().read();
        (touched(&def.source_id(), &def, &state, &[], &[], changes), state.albums.clone())
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
        let before = touched(&source, &def, w.state(), &plan.remaps, &plan.collections, &plan.apply);
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
                exported.push((ext.to_owned(), (photo.clone(), rec.path_under(&root.path), rec.kind)));
            }
            w.put_user(ext, Some(layer))?;
        }
        if albums != w.state().albums {
            w.set_albums(albums)?;
        }
        Ok((applied, exported))
    })?;
    let mut warnings = Vec::new();
    if def.write_xmp && !exported.is_empty() {
        let (exts, files): (Vec<String>, Vec<_>) = exported.into_iter().unzip();
        let done = export(&def, &files);
        warnings = done.problems;
        let written: Vec<(&str, &Path, &Written)> =
            done.written.iter().map(|(i, w)| (exts[*i].as_str(), files[*i].1.as_path(), w)).collect();
        warnings.extend(refresh(src, &written));
    }
    Ok(ApplyReport { applied, backup: Some(backup.path), warnings })
}

/// What the export changed of a photo's files, as its record holds it.
enum Refresh {
    Original { size: u64, mtime_ns: i64, content: ContentKey },
    Sidecar { name: String, mtime_ns: i64 },
}

/// Brings the records of the files the export wrote, each as (photo id, the photo's file,
/// what was written), up to date, so the next walk does not take them for changed files
/// (or, an original moved meanwhile, for new ones that leave the user's edits behind): an
/// original's size, time and content key, its images kept current (their pixels are the
/// same); a sidecar's name and time. Returns what could not be brought up to date.
fn refresh(src: &FolderSource, written: &[(&str, &Path, &Written)]) -> Vec<String> {
    let mut problems = Vec::new();
    // Read before the store's lock is taken.
    let mut updates: Vec<(&str, Refresh)> = Vec::new();
    for (ext, path, what) in written {
        let update = match what {
            Written::Original => fs::metadata(path).and_then(|m| {
                let content = ids::content_key(path)?;
                Ok(Refresh::Original { size: m.len(), mtime_ns: mtime_ns(&m), content })
            }),
            Written::Sidecar(sidecar) => fs::metadata(sidecar).map(|m| Refresh::Sidecar {
                name: sidecar.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                mtime_ns: mtime_ns(&m),
            }),
        };
        match update {
            Ok(update) => updates.push((ext, update)),
            Err(e) => problems.push(format!("{}: {e}", path.display())),
        }
    }
    if updates.is_empty() {
        return problems;
    }
    let result = src.store().write(|w| {
        for (ext, update) in &updates {
            let Some(mut rec) = w.state().files.get(*ext).cloned() else { continue };
            match update {
                Refresh::Original { size, mtime_ns, content } => {
                    for tier in [ImageTier::Thumb, ImageTier::Preview] {
                        let Some(image) = w.state().images.get(&((*ext).to_owned(), tier)) else { continue };
                        if (image.src_size, image.src_mtime_ns) == (rec.size, rec.mtime_ns) {
                            let image = ImageRec { src_size: *size, src_mtime_ns: *mtime_ns, ..image.clone() };
                            w.put_image(ext, tier, Some(image))?;
                        }
                    }
                    rec.size = *size;
                    rec.mtime_ns = *mtime_ns;
                    rec.content = content.clone();
                    rec.full = None;
                }
                Refresh::Sidecar { name, mtime_ns } => {
                    rec.sidecar = Some(name.clone());
                    rec.sidecar_mtime_ns = Some(*mtime_ns);
                }
            }
            w.put_file(ext, rec)?;
        }
        Ok(())
    });
    if let Err(e) = result {
        problems.push(format!("the files the export wrote could not be recorded: {e}"));
    }
    problems
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
    /// The root is there, with a file for each record left present, so a snapshot's walk
    /// finds them unchanged (and not offline).
    fn fixture_with(n: usize, edit: impl Fn(usize, &mut FileRec)) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let photos = dir.path().join("photos");
        fs::create_dir_all(photos.join("2024")).unwrap();
        let def = Definition::new("t", std::slice::from_ref(&photos));
        let location = dir.path().join("t.c1folders");
        def.save(&location).unwrap();
        let src = FolderSource::open(&location, &dir.path().join("work")).unwrap();
        let root = def.roots[0].id.clone();
        let mut recs = Vec::new();
        for i in 0..n {
            let mut r = rec(&root, &format!("2024/{i}.jpg"));
            edit(i, &mut r);
            if r.state == FileState::Present {
                let path = r.path_under(&def.roots[0].path);
                fs::write(&path, format!("photo {i}")).unwrap();
                let meta = fs::metadata(&path).unwrap();
                (r.size, r.mtime_ns) = (meta.len(), mtime_ns(&meta));
            }
            recs.push(r);
        }
        src.store()
            .write(|w| {
                for (i, r) in recs.into_iter().enumerate() {
                    w.put_file(&ext(i), r)?;
                }
                Ok(())
            })
            .unwrap();
        Fixture { _dir: dir, src, root }
    }

    /// A source over a root of the files `names` (JPEGs made with `image`; anything else a
    /// few bytes), after a first snapshot walked it. Gives the root too.
    fn scanned(names: &[&str], edit: impl FnOnce(&mut Definition)) -> (tempfile::TempDir, FolderSource, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        fs::create_dir_all(&root).unwrap();
        for (i, name) in names.iter().enumerate() {
            if name.ends_with(".jpg") {
                let shade = (i * 40) as u8;
                image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([(x * 4) as u8, (y * 5) as u8, shade]))
                    .save(root.join(name))
                    .unwrap();
            } else {
                fs::write(root.join(name), format!("not really a raw {i}")).unwrap();
            }
        }
        let mut def = Definition::new("t", std::slice::from_ref(&root));
        edit(&mut def);
        let location = dir.path().join("t.c1folders");
        def.save(&location).unwrap();
        let src = FolderSource::open(&location, &dir.path().join("work")).unwrap();
        let (photos, problems) = c1model::conformance::collect(&src).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(photos.len(), names.len());
        assert!(photos.iter().all(|p| !p.file.offline));
        (dir, src, root)
    }

    /// The id of the photo at `relpath`.
    fn id_of(src: &FolderSource, relpath: &str) -> PhotoId {
        let state = src.store().read();
        let ext = state.files.iter().find(|(_, r)| r.relpath == relpath).map(|(ext, _)| ext.clone()).unwrap();
        PhotoId::new(&src.definition().source_id(), ext)
    }

    /// Sets the photo's rating, from none.
    fn rate(src: &FolderSource, id: &PhotoId, stars: u8) -> ApplyReport {
        let change = Change { photo: id.clone(), op: rating(None, Some(stars)) };
        src.apply(&src.plan(&[change]).unwrap()).unwrap()
    }

    /// Walks the roots again; what the walk found.
    fn rescan(src: &FolderSource) -> (Vec<Photo>, crate::ScanOutcome) {
        src.rescan().unwrap();
        let (photos, _) = c1model::conformance::collect(src).unwrap();
        (photos, src.last_scan().unwrap())
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
    fn a_write_reaches_photos_it_does_not_name() {
        let f = fixture_with(4, |i, r| {
            r.meta.keywords = match i {
                0 => vec![k("a")],
                1 => vec![k("b|x")],
                _ => vec![],
            }
        });
        // The second remap moves a keyword photo 1 had all along, and the one the first made.
        let remaps = [KeywordRemap::rename(k("a"), "b"), KeywordRemap::rename(k("b"), "c")];
        f.write_after(&remaps, &[], &[f.change(3, rating(None, Some(1)))]);
        assert_eq!(f.photo(0).keywords, [k("c")]);
        assert_eq!(f.photo(1).keywords, [k("c|x")]);
        assert_eq!(f.layer(2), None, "a photo nothing touched gets no layer");
        assert_eq!(f.photo(3).rating, Some(1));

        let album = CollectionId::new_local("a");
        f.write_after(
            &[],
            &[new_album(&album, "Rome", CollectionKind::Album, None)],
            &[f.change(2, Op::AddToCollection { collection: album })],
        );
        let made = f.albums()[0].id.clone();
        f.write_after(&[], &[CollectionChange::Delete { id: made }], &[f.change(3, rating(Some(1), Some(2)))]);
        assert_eq!(f.layer(2), None, "the deleted album leaves the photo the plan did not name");
        assert_eq!(f.photo(3).rating, Some(2));
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
        assert!(!f.photo(1).file.offline, "on a source whose photos are there");
    }

    #[test]
    fn passes_the_backup_conformance_check() {
        let f = fixture(3);
        assert_eq!(c1model::conformance::check_backup(&f.src, &f.id(1)), Vec::<String>::new());
        assert_eq!(f.photo(1).rating, None, "probe restored");
        assert!(!f.photo(1).file.offline, "on a source whose photos are there");
    }

    #[test]
    fn passes_the_conformance_checks_on_photos_a_walk_found() {
        let (_dir, src, _) = scanned(&["a.jpg", "b.jpg", "c.jpg"], |_| {});
        let probe = id_of(&src, "b.jpg");
        assert_eq!(c1model::conformance::check_write(&src, &probe), Vec::<String>::new());
        assert_eq!(c1model::conformance::check_incremental(&src, &probe), Vec::<String>::new());
        assert_eq!(c1model::conformance::check_backup(&src, &probe), Vec::<String>::new());
    }

    #[test]
    fn xmp_written_into_an_original_is_recorded_and_its_images_stay() {
        let (_dir, src, root) = scanned(&["a.jpg", "b.jpg"], |def| {
            def.write_xmp = true;
            def.modify_originals = true;
        });
        let id = id_of(&src, "a.jpg");
        let thumb = src.make_image(&id, ImageTier::Thumb).unwrap().unwrap();
        let before = src.store().read().files[&id.ext].clone();
        let report = rate(&src, &id, 4);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let path = root.join("a.jpg");
        let meta = fs::metadata(&path).unwrap();
        let rec = src.store().read().files[&id.ext].clone();
        assert_ne!(rec.size, before.size, "the XMP went in");
        assert_eq!((rec.size, rec.mtime_ns), (meta.len(), mtime_ns(&meta)));
        assert_eq!(rec.content, ids::content_key(&path).unwrap());
        assert_eq!(rec.full, None);
        assert_eq!(src.store().read().user[&id.ext].rating, Some(Some(4)));
        assert_eq!(src.image(&id, ImageTier::Thumb).unwrap(), Some(thumb.clone()), "its pixels are the same");

        // The next walk finds nothing changed and nothing to make again.
        let (_, scan) = rescan(&src);
        assert_eq!((scan.changed, scan.read), (0, 0), "{scan:?}");
        assert!(thumb.path.is_file());

        // Moved after the export, it is still known by its content: same id, same edits.
        fs::create_dir_all(root.join("moved")).unwrap();
        fs::rename(&path, root.join("moved").join("a.jpg")).unwrap();
        let (photos, scan) = rescan(&src);
        assert_eq!((scan.moved, scan.added, scan.removed), (1, 0, 0), "{scan:?}");
        assert_eq!(photos.iter().find(|p| p.id == id).expect("the same id").rating, Some(4));
    }

    #[test]
    fn a_sidecar_the_export_wrote_is_recorded() {
        let (_dir, src, root) = scanned(&["a.jpg", "DSCF0001.RAF"], |def| def.write_xmp = true);
        let id = id_of(&src, "DSCF0001.RAF");
        let report = rate(&src, &id, 3);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let rec = src.store().read().files[&id.ext].clone();
        let sidecar = fs::metadata(root.join("DSCF0001.xmp")).unwrap();
        assert_eq!(rec.sidecar.as_deref(), Some("DSCF0001.xmp"));
        assert_eq!(rec.sidecar_mtime_ns, Some(mtime_ns(&sidecar)));
        let (_, scan) = rescan(&src);
        assert_eq!((scan.changed, scan.read), (0, 0), "{scan:?}");
    }
}
