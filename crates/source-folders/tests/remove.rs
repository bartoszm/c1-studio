//! Deleting photos for good: their files go to the recycle bin (here, deleted), a sidecar
//! another photo shares stays, and a file that could not go yet is never found again as a
//! new photo.

use std::path::{Path, PathBuf};

use c1model::{Cursor, Photo, PhotoId, PhotoSource, SourceEvent};
use source_folders::{Definition, FolderSource};

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

fn delete(paths: &[PathBuf]) -> Result<(), String> {
    paths.iter().try_for_each(|p| std::fs::remove_file(p).map_err(|e| e.to_string()))
}

fn refuse(_paths: &[PathBuf]) -> Result<(), String> {
    Err("the file is in use".into())
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    src: FolderSource,
}

impl Fixture {
    fn new(files: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        std::fs::create_dir_all(&root).unwrap();
        for f in files {
            put(&root.join(f), &jpeg(f));
        }
        let def_path = dir.path().join("test.c1folders");
        Definition::new("Test", std::slice::from_ref(&root)).save(&def_path).unwrap();
        let src = FolderSource::open(&def_path, &dir.path().join("work")).unwrap();
        src.set_recycler(delete);
        Self { _dir: dir, root, src }
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

    /// The photo whose file is `name`, from a snapshot.
    fn id(&self, events: &[SourceEvent], name: &str) -> PhotoId {
        upserts(events).into_iter().find(|p| p.file.name == name).unwrap_or_else(|| panic!("no {name}")).id.clone()
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

#[test]
fn a_photo_deleted_for_good_takes_its_files_and_is_deleted() {
    let fx = Fixture::new(&["a.jpg", "b.jpg"]);
    put(&fx.root.join("a.xmp"), b"<x:xmpmeta/>");
    let first = fx.snap(None);
    let a = fx.id(&first, "a.jpg");

    let report = fx.src.remove(std::slice::from_ref(&a)).unwrap();
    assert_eq!(report.removed, std::slice::from_ref(&a));
    assert!(report.failed.is_empty() && report.warnings.is_empty(), "{report:?}");
    assert!(!fx.root.join("a.jpg").exists() && !fx.root.join("a.xmp").exists(), "both went");
    assert!(fx.root.join("b.jpg").exists());

    let events = fx.snap(Some(&cursor(&first)));
    assert_eq!(deletes(&events), [&a]);
    let events = fx.rescan(&cursor(&events));
    assert!(upserts(&events).is_empty() && deletes(&events).is_empty(), "{events:?}");
}

#[test]
fn a_sidecar_shared_with_a_photo_that_stays_stays() {
    let fx = Fixture::new(&["x.jpg", "x.png"]);
    put(&fx.root.join("x.xmp"), b"<x:xmpmeta/>");
    let first = fx.snap(None);
    let jpg = fx.id(&first, "x.jpg");
    let png = fx.id(&first, "x.png");

    fx.src.remove(std::slice::from_ref(&jpg)).unwrap();
    assert!(!fx.root.join("x.jpg").exists());
    assert!(fx.root.join("x.xmp").exists(), "x.png still reads it");

    // Once neither is left, it goes too.
    fx.src.remove(std::slice::from_ref(&png)).unwrap();
    assert!(!fx.root.join("x.xmp").exists());
}

#[test]
fn photos_that_cannot_go_are_listed_and_the_rest_still_go() {
    let fx = Fixture::new(&["a.jpg", "b.jpg"]);
    let first = fx.snap(None);
    let (a, b) = (fx.id(&first, "a.jpg"), fx.id(&first, "b.jpg"));
    // Changed on disk since it was read: what is there now is not what the user saw.
    std::thread::sleep(std::time::Duration::from_millis(20));
    put(&fx.root.join("b.jpg"), b"something else entirely");
    let stranger = PhotoId::new(&a.source, "0000000000000000");

    let report = fx.src.remove(&[a.clone(), b.clone(), stranger.clone()]).unwrap();
    assert_eq!(report.removed, [a]);
    let failed: Vec<&PhotoId> = report.failed.iter().map(|(id, _)| id).collect();
    assert_eq!(failed, [&b, &stranger]);
    assert!(fx.root.join("b.jpg").exists());
}

#[test]
fn a_photo_whose_folder_is_offline_stays() {
    let fx = Fixture::new(&["a.jpg"]);
    let first = fx.snap(None);
    let a = fx.id(&first, "a.jpg");
    let away = fx.root.with_file_name("photos-away");
    std::fs::rename(&fx.root, &away).unwrap();
    fx.rescan(&cursor(&first));

    let report = fx.src.remove(std::slice::from_ref(&a)).unwrap();
    assert!(report.removed.is_empty());
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert!(away.join("a.jpg").exists());
}

#[test]
fn a_file_that_could_not_go_yet_is_not_found_again() {
    let fx = Fixture::new(&["a.jpg", "b.jpg"]);
    let first = fx.snap(None);
    let a = fx.id(&first, "a.jpg");
    fx.src.set_recycler(refuse);

    let report = fx.src.remove(std::slice::from_ref(&a)).unwrap();
    assert_eq!(report.removed, std::slice::from_ref(&a), "the photo is gone even though its file is not");
    assert_eq!(report.warnings.len(), 1, "{report:?}");
    assert!(fx.root.join("a.jpg").exists());

    let events = fx.rescan(&cursor(&first));
    assert_eq!(deletes(&events), [&a]);
    assert!(upserts(&events).is_empty(), "not taken for a new photo: {events:?}");

    // The next scan tries again, and this time it goes.
    fx.src.set_recycler(delete);
    let events = fx.rescan(&cursor(&events));
    assert!(upserts(&events).is_empty(), "{events:?}");
    assert!(!fx.root.join("a.jpg").exists());
}

#[test]
fn a_sidecar_stays_with_a_photo_of_the_same_batch_that_was_refused() {
    let fx = Fixture::new(&["x.jpg", "x.png"]);
    put(&fx.root.join("x.xmp"), b"<x:xmpmeta/>");
    let first = fx.snap(None);
    let (jpg, png) = (fx.id(&first, "x.jpg"), fx.id(&first, "x.png"));
    std::thread::sleep(std::time::Duration::from_millis(20));
    put(&fx.root.join("x.png"), b"changed since it was read");

    let report = fx.src.remove(&[jpg.clone(), png.clone()]).unwrap();
    assert_eq!(report.removed, std::slice::from_ref(&jpg));
    assert!(fx.root.join("x.png").exists());
    assert!(fx.root.join("x.xmp").exists(), "x.png stays, and reads it");
}

#[test]
fn another_file_put_where_one_waits_for_the_recycle_bin_is_left_alone() {
    let fx = Fixture::new(&["a.jpg", "b.jpg"]);
    let first = fx.snap(None);
    let a = fx.id(&first, "a.jpg");
    fx.src.set_recycler(refuse);
    fx.src.remove(std::slice::from_ref(&a)).unwrap();
    // Gone some other way, and a new file put at the same path.
    std::fs::remove_file(fx.root.join("a.jpg")).unwrap();
    put(&fx.root.join("a.jpg"), &jpeg("a new photo, not the one deleted"));

    fx.src.set_recycler(delete);
    let events = fx.rescan(&cursor(&first));
    assert!(fx.root.join("a.jpg").exists(), "not recycled");
    assert!(upserts(&events).iter().any(|p| p.file.name == "a.jpg" && p.id != a), "found as a new photo: {events:?}");
}

#[test]
fn conforms() {
    let fx = Fixture::new(&["a.jpg", "x/b.jpg"]);
    let probe = fx.id(&fx.snap(None), "a.jpg");
    let problems = c1model::conformance::check_remove(&fx.src, &probe);
    assert!(problems.is_empty(), "{problems:?}");
}
