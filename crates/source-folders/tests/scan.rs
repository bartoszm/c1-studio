//! Walking the roots and the snapshots that follow: ids that survive moves, renames and
//! roots picked again, offline roots, and incremental snapshots that cost nothing.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use c1model::{CollectionKind, Cursor, Photo, PhotoId, PhotoSource, SourceEvent};
use source_folders::layer::folder_id;
use source_folders::{Definition, FolderSource};

/// A small fake JPEG whose content is unique to `seed`.
fn jpeg(seed: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.extend_from_slice(seed.as_bytes());
    bytes.extend(std::iter::repeat_n(0u8, 64));
    bytes.extend_from_slice(&[0xFF, 0xD9]);
    bytes
}

fn put(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

struct Fixture {
    _dir: tempfile::TempDir,
    /// Holds the roots.
    base: PathBuf,
    def_path: PathBuf,
    src: FolderSource,
}

impl Fixture {
    /// A source over one root, `base/photos`, holding `files` (relpaths), each a unique JPEG.
    fn new(files: &[&str]) -> Self {
        Self::with(files, |_| {})
    }

    fn with(files: &[&str], edit: impl FnOnce(&mut Definition)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_owned();
        let root = base.join("photos");
        std::fs::create_dir_all(&root).unwrap();
        for f in files {
            put(&root.join(f), &jpeg(f));
        }
        let mut def = Definition::new("Test", std::slice::from_ref(&root));
        edit(&mut def);
        let def_path = base.join("test.c1folders");
        def.save(&def_path).unwrap();
        let src = FolderSource::open(&def_path, &base.join("work")).unwrap();
        Self { _dir: dir, base, def_path, src }
    }

    fn root(&self) -> PathBuf {
        self.base.join("photos")
    }

    fn snap(&self, since: Option<&Cursor>) -> Vec<SourceEvent> {
        let mut events = Vec::new();
        self.src
            .snapshot(since, &mut |e| {
                events.push(e);
                Ok(())
            })
            .unwrap();
        events
    }

    fn rescan(&self, since: &Cursor) -> Vec<SourceEvent> {
        self.src.rescan().unwrap();
        self.snap(Some(since))
    }

    fn edit_def(&self, edit: impl FnOnce(&mut Definition)) {
        let mut def = Definition::load(&self.def_path).unwrap();
        edit(&mut def);
        def.save(&self.def_path).unwrap();
        self.src.reload_definition(None).unwrap();
    }
}

fn cursor(events: &[SourceEvent]) -> Cursor {
    match events.last() {
        Some(SourceEvent::End(c)) => c.clone(),
        other => panic!("no End: {other:?}"),
    }
}

fn upserts(events: &[SourceEvent]) -> Vec<&Photo> {
    events
        .iter()
        .filter_map(|e| match e {
            SourceEvent::Upsert(p) => Some(&**p),
            _ => None,
        })
        .collect()
}

fn deletes(events: &[SourceEvent]) -> Vec<&PhotoId> {
    events
        .iter()
        .filter_map(|e| match e {
            SourceEvent::Delete(id) => Some(id),
            _ => None,
        })
        .collect()
}

/// Photo path below `root` → photo id, of the upserts.
fn ids(events: &[SourceEvent], root: &Path) -> BTreeMap<String, String> {
    upserts(events)
        .into_iter()
        .map(|p| {
            let path = PathBuf::from(p.file.path.as_deref().unwrap());
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            (rel, p.id.ext.clone())
        })
        .collect()
}

fn is_nothing(events: &[SourceEvent]) -> bool {
    matches!(events, [SourceEvent::BeginIncremental, SourceEvent::End(_)])
}

#[test]
fn a_first_snapshot_lists_every_file_with_its_folder() {
    let fx = Fixture::new(&["a.jpg", "2024/rome/b.JPG", "2024/c.png", "notes.txt", "2024/rome/b.xmp"]);
    let events = fx.snap(None);
    assert!(matches!(events.first(), Some(SourceEvent::BeginFull)));
    let found = ids(&events, &fx.root());
    assert_eq!(found.keys().collect::<Vec<_>>(), ["2024/c.png", "2024/rome/b.JPG", "a.jpg"]);
    assert!(found.values().all(|id| id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit())));
    let collections = events
        .iter()
        .find_map(|e| match e {
            SourceEvent::Collections(c) => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    let folders: HashSet<_> =
        collections.iter().filter(|c| c.kind == CollectionKind::Folder).map(|c| c.id.clone()).collect();
    assert_eq!(folders.len(), 3, "the root, 2024 and 2024/rome");
    let root_id = fx.src.definition().roots[0].id.clone();
    for photo in upserts(&events) {
        assert_eq!(photo.asset.0, photo.id.ext);
        assert!(!photo.file.offline);
        assert!(photo.collections.iter().all(|c| folders.contains(c)), "{photo:?}");
        assert_eq!(photo.collections.len(), 1);
    }
    let b = upserts(&events).into_iter().find(|p| p.file.name == "b.JPG").unwrap();
    assert_eq!(b.collections, [folder_id(&root_id, "2024/rome")]);
    let scan = fx.src.last_scan().unwrap();
    assert_eq!((scan.added, scan.read), (3, 3));
    assert!(fx.src.progress().is_none(), "progress is cleared after the walk");
}

#[test]
fn snapshots_without_a_rescan_do_not_walk() {
    let fx = Fixture::new(&["a.jpg", "b.jpg"]);
    let first = fx.snap(None);
    // Gone on disk, but nobody asked for a walk.
    std::fs::remove_file(fx.root().join("a.jpg")).unwrap();
    let again = fx.snap(Some(&cursor(&first)));
    assert!(is_nothing(&again), "{again:?}");
    assert_eq!(cursor(&again), cursor(&first));
    // A cursor of another store, or from the future, gets everything.
    let other = fx.snap(Some(&Cursor("ffff:1".into())));
    assert!(matches!(other.first(), Some(SourceEvent::BeginFull)));
    let (uuid, _) = cursor(&first).0.split_once(':').map(|(u, g)| (u.to_owned(), g.to_owned())).unwrap();
    let future = fx.snap(Some(&Cursor(format!("{uuid}:999"))));
    assert!(matches!(future.first(), Some(SourceEvent::BeginFull)));
}

#[test]
fn a_rescan_of_nothing_changed_reads_nothing() {
    let fx = Fixture::new(&["a.jpg", "x/b.jpg", "x/c.jpg"]);
    let first = fx.snap(None);
    let again = fx.rescan(&cursor(&first));
    assert!(is_nothing(&again), "{again:?}");
    let scan = fx.src.last_scan().unwrap();
    assert_eq!(scan.read, 0);
    assert_eq!((scan.added, scan.changed, scan.moved, scan.removed), (0, 0, 0, 0));
}

#[test]
fn a_touched_file_is_read_again_alone() {
    let fx = Fixture::new(&["a.jpg", "b.jpg", "c.jpg"]);
    let first = fx.snap(None);
    let before = ids(&first, &fx.root());

    put(&fx.root().join("b.jpg"), &jpeg("b, edited"));
    let events = fx.rescan(&cursor(&first));
    let up = upserts(&events);
    assert_eq!(up.len(), 1, "{events:?}");
    assert_eq!(up[0].id.ext, before["b.jpg"], "a changed file keeps its id");
    assert!(deletes(&events).is_empty());
    let scan = fx.src.last_scan().unwrap();
    assert_eq!((scan.changed, scan.read), (1, 1));

    // Only its time.
    let file = std::fs::File::options().write(true).open(fx.root().join("c.jpg")).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(60)).unwrap();
    let next = fx.rescan(&cursor(&events));
    assert_eq!(ids(&next, &fx.root()).into_keys().collect::<Vec<_>>(), ["c.jpg"]);
    assert_eq!(fx.src.last_scan().unwrap().read, 1);

    // A sidecar beside it.
    put(&fx.root().join("a.xmp"), b"<x:xmpmeta/>");
    let next = fx.rescan(&cursor(&next));
    assert_eq!(ids(&next, &fx.root()).into_keys().collect::<Vec<_>>(), ["a.jpg"]);
}

#[test]
fn moved_and_renamed_files_keep_their_ids() {
    let fx = Fixture::new(&["a/1.jpg", "a/2.jpg", "a/3.jpg"]);
    let first = fx.snap(None);
    let before = ids(&first, &fx.root());
    std::fs::rename(fx.root().join("a/1.jpg"), fx.root().join("a/one.jpg")).unwrap();
    std::fs::create_dir_all(fx.root().join("b/c")).unwrap();
    std::fs::rename(fx.root().join("a/2.jpg"), fx.root().join("b/c/2.jpg")).unwrap();
    let events = fx.rescan(&cursor(&first));
    let after = ids(&events, &fx.root());
    assert_eq!(after.get("a/one.jpg"), Some(&before["a/1.jpg"]));
    assert_eq!(after.get("b/c/2.jpg"), Some(&before["a/2.jpg"]));
    assert_eq!(after.len(), 2, "{after:?}");
    assert!(deletes(&events).is_empty(), "{events:?}");
    let scan = fx.src.last_scan().unwrap();
    assert_eq!((scan.moved, scan.added, scan.removed, scan.read), (2, 0, 0, 0));
    // The folder moved into shows up.
    let full = fx.snap(None);
    let root_id = fx.src.definition().roots[0].id.clone();
    let moved = upserts(&full).into_iter().find(|p| p.id.ext == before["a/2.jpg"]).unwrap();
    assert_eq!(moved.collections, [folder_id(&root_id, "b/c")]);
}

#[test]
fn a_copy_gets_a_new_id() {
    let fx = Fixture::new(&["a.jpg"]);
    let first = fx.snap(None);
    let original = ids(&first, &fx.root())["a.jpg"].clone();
    std::fs::copy(fx.root().join("a.jpg"), fx.root().join("copy.jpg")).unwrap();
    let events = fx.rescan(&cursor(&first));
    let after = ids(&fx.snap(None), &fx.root());
    assert_eq!(after["a.jpg"], original);
    assert_ne!(after["copy.jpg"], original);
    assert!(deletes(&events).is_empty());
    assert_eq!(fx.src.last_scan().unwrap().added, 1);

    // The original then moves while the copy stays: the original's id follows its content.
    std::fs::rename(fx.root().join("a.jpg"), fx.root().join("moved.jpg")).unwrap();
    let events = fx.rescan(&cursor(&events));
    let now = ids(&fx.snap(None), &fx.root());
    assert_eq!(now["moved.jpg"], original);
    assert_eq!(now["copy.jpg"], after["copy.jpg"]);
    assert!(deletes(&events).is_empty());
}

#[test]
fn a_deleted_file_is_deleted() {
    let fx = Fixture::new(&["a.jpg", "b.jpg"]);
    let first = fx.snap(None);
    let before = ids(&first, &fx.root());
    std::fs::remove_file(fx.root().join("a.jpg")).unwrap();
    let events = fx.rescan(&cursor(&first));
    assert_eq!(deletes(&events).iter().map(|id| id.ext.clone()).collect::<Vec<_>>(), [before["a.jpg"].clone()]);
    assert!(upserts(&events).is_empty());
    assert_eq!(fx.src.last_scan().unwrap().removed, 1);
    assert!(is_nothing(&fx.rescan(&cursor(&events))));
}

#[test]
fn a_root_out_of_reach_goes_offline_and_comes_back() {
    let fx = Fixture::new(&["a.jpg", "x/b.jpg"]);
    let first = fx.snap(None);
    let before = ids(&first, &fx.root());
    let away = fx.base.join("photos-renamed");

    std::fs::rename(fx.root(), &away).unwrap();
    let events = fx.rescan(&cursor(&first));
    assert!(deletes(&events).is_empty());
    let up = upserts(&events);
    assert_eq!(up.len(), 2);
    assert!(up.iter().all(|p| p.file.offline));
    assert_eq!(ids(&events, &fx.root()), before, "same ids and paths");
    let scan = fx.src.last_scan().unwrap();
    assert_eq!(scan.offline_roots, ["photos"]);
    assert_eq!(scan.removed, 0);

    // Back under its name.
    std::fs::rename(&away, fx.root()).unwrap();
    let events = fx.rescan(&cursor(&events));
    assert!(deletes(&events).is_empty());
    assert!(upserts(&events).iter().all(|p| !p.file.offline));
    assert_eq!(ids(&events, &fx.root()), before);
    assert!(fx.src.last_scan().unwrap().offline_roots.is_empty());

    // Away again, and the definition follows it to its new path.
    std::fs::rename(fx.root(), &away).unwrap();
    let events = fx.rescan(&cursor(&events));
    assert!(upserts(&events).iter().all(|p| p.file.offline));
    fx.edit_def(|def| def.roots[0].path = away.to_string_lossy().into_owned());
    let events = fx.snap(Some(&cursor(&events)));
    assert!(deletes(&events).is_empty(), "{events:?}");
    assert!(upserts(&events).iter().all(|p| !p.file.offline));
    assert_eq!(ids(&events, &away), before, "same ids, at the new path");
    assert_eq!(fx.src.last_scan().unwrap().read, 0);
}

#[test]
fn a_root_moved_by_the_definition_alone_sends_its_new_paths() {
    let fx = Fixture::new(&["a.jpg"]);
    let first = fx.snap(None);
    let before = ids(&first, &fx.root());
    let copy = fx.base.join("elsewhere");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(fx.root().join("a.jpg"), copy.join("a.jpg")).unwrap();
    // Keep the time, so the file looks the same.
    let mtime = std::fs::metadata(fx.root().join("a.jpg")).unwrap().modified().unwrap();
    std::fs::File::options().write(true).open(copy.join("a.jpg")).unwrap().set_modified(mtime).unwrap();
    fx.edit_def(|def| def.roots[0].path = copy.to_string_lossy().into_owned());
    let events = fx.snap(Some(&cursor(&first)));
    assert_eq!(ids(&events, &copy), before, "{events:?}");
}

#[test]
fn a_root_taken_out_and_picked_again_gets_its_ids_back() {
    let fx = Fixture::new(&["a.jpg", "x/b.jpg"]);
    let other = fx.base.join("other");
    put(&other.join("c.jpg"), &jpeg("c"));
    fx.edit_def(|def| {
        def.add_root(&other);
    });
    let first = fx.snap(None);
    let before = ids(&first, &fx.root());
    assert_eq!(before.len(), 3);

    let photos = fx.root().to_string_lossy().into_owned();
    fx.edit_def(|def| def.roots.retain(|r| r.path != photos));
    let events = fx.snap(Some(&cursor(&first)));
    let gone: HashSet<_> = deletes(&events).into_iter().map(|id| id.ext.clone()).collect();
    assert_eq!(gone, HashSet::from([before["a.jpg"].clone(), before["x/b.jpg"].clone()]));
    assert!(upserts(&events).is_empty(), "{events:?}");
    assert_eq!(fx.src.last_scan().unwrap().removed, 2);
    let full = fx.snap(None);
    assert_eq!(upserts(&full).len(), 1, "detached files are not shown");

    fx.edit_def(|def| {
        def.add_root(&fx.root());
    });
    let events = fx.snap(Some(&cursor(&events)));
    assert!(deletes(&events).is_empty());
    let back = ids(&events, &fx.root());
    assert_eq!(back["a.jpg"], before["a.jpg"]);
    assert_eq!(back["x/b.jpg"], before["x/b.jpg"]);
    let scan = fx.src.last_scan().unwrap();
    assert_eq!((scan.moved, scan.added, scan.read), (2, 0, 0));
}

/// About 200 KB, the same first and last 64 KB, differing in the middle.
fn twin(middle: u8) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.extend((0..200_000u32).map(|i| (i % 249) as u8));
    bytes.extend_from_slice(&[0xFF, 0xD9]);
    bytes[100_000] = middle;
    bytes
}

#[test]
fn files_alike_at_both_ends_are_told_apart_by_their_whole_content() {
    let fx = Fixture::new(&[]);
    put(&fx.root().join("a/x.jpg"), &twin(1));
    put(&fx.root().join("a/y.jpg"), &twin(2));
    let first = fx.snap(None);
    let before = ids(&first, &fx.root());
    assert_eq!(before.len(), 2);

    // Both move at once, each to the other's name.
    std::fs::create_dir_all(fx.root().join("b")).unwrap();
    std::fs::rename(fx.root().join("a/x.jpg"), fx.root().join("b/y.jpg")).unwrap();
    std::fs::rename(fx.root().join("a/y.jpg"), fx.root().join("b/x.jpg")).unwrap();
    let events = fx.rescan(&cursor(&first));
    assert!(deletes(&events).is_empty());
    let after = ids(&events, &fx.root());
    assert_eq!(after["b/y.jpg"], before["a/x.jpg"]);
    assert_eq!(after["b/x.jpg"], before["a/y.jpg"]);
    assert_eq!(fx.src.last_scan().unwrap().moved, 2);
}

#[test]
fn excluded_and_generated_folders_are_skipped() {
    let fx = Fixture::with(
        &[
            "keep.jpg",
            ".hidden/kept.jpg",
            "tmp/a.jpg",
            "deep/tmp/b.jpg",
            "x/skip-me.jpg",
            ".c1studio/thumb/c.jpg",
            "y/.previews/d.thumb.jpg",
        ],
        |def| def.excludes = vec!["**/tmp/**".into(), "*skip-me.jpg".into()],
    );
    let found = ids(&fx.snap(None), &fx.root());
    assert_eq!(found.into_keys().collect::<Vec<_>>(), [".hidden/kept.jpg", "keep.jpg"]);
}

#[test]
fn conforms() {
    let fx = Fixture::new(&["a.jpg", "x/b.jpg"]);
    let problems = c1model::conformance::check_read(&fx.src);
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn conforms_incrementally() {
    let fx = Fixture::new(&["a.jpg", "x/b.jpg"]);
    let probe = upserts(&fx.snap(None))[0].id.clone();
    let problems = c1model::conformance::check_incremental(&fx.src, &probe);
    assert!(problems.is_empty(), "{problems:?}");
}

#[cfg(unix)]
#[test]
fn a_folder_that_cannot_be_read_keeps_its_photos() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fixture::new(&["a.jpg", "locked/b.jpg"]);
    let first = fx.snap(None);
    let locked = fx.root().join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&locked).is_ok() {
        // Running as root: permissions do not stop it.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let events = fx.rescan(&cursor(&first));
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(deletes(&events).is_empty(), "{events:?}");
    assert_eq!(fx.src.last_scan().unwrap().problems.len(), 1);
}

#[test]
#[ignore = "slow: 10k files"]
fn ten_thousand_files() {
    let fx = Fixture::new(&[]);
    for folder in 0..100 {
        for file in 0..100 {
            let rel = format!("f{folder:03}/p{file:03}.jpg");
            put(&fx.root().join(&rel), &jpeg(&rel));
        }
    }
    let started = Instant::now();
    let first = fx.snap(None);
    let first_ms = started.elapsed().as_millis();
    assert_eq!(upserts(&first).len(), 10_000);
    let started = Instant::now();
    let again = fx.rescan(&cursor(&first));
    let rescan_ms = started.elapsed().as_millis();
    assert!(is_nothing(&again));
    assert_eq!(fx.src.last_scan().unwrap().read, 0);
    println!("10k files: first snapshot {first_ms} ms, rescan of nothing changed {rescan_ms} ms");
}
