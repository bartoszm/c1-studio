//! Images made for photos that move: with their folder, without it, under a root whose path
//! changed; and images kept when only a sidecar changed.

use std::fs;
use std::path::{Path, PathBuf};

use c1model::{ImageTier, Photo, PhotoId, PhotoSource};
use source_folders::{Definition, FolderSource, ImageStore, Preset, ScanOutcome};

/// A small picture (its format from the extension; a PNG is never its own preview), its
/// colour from `seed`, so each photo's content is its own.
fn picture(path: &Path, seed: u8) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::RgbImage::from_fn(96, 64, |x, y| image::Rgb([(x * 2) as u8, (y * 3) as u8, seed])).save(path).unwrap();
}

struct Fixture {
    _dir: tempfile::TempDir,
    base: PathBuf,
    def_path: PathBuf,
    src: FolderSource,
}

impl Fixture {
    /// A source over `base/photos` holding pictures at `files`, with the image store
    /// `images`, after a first snapshot.
    fn new(files: &[&str], images: ImageStore) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_owned();
        let root = base.join("photos");
        for (i, f) in files.iter().enumerate() {
            picture(&root.join(f), (i * 30) as u8);
        }
        let mut def = Definition::new("Test", std::slice::from_ref(&root));
        def.images = images;
        let def_path = base.join("test.c1folders");
        def.save(&def_path).unwrap();
        let src = FolderSource::open(&def_path, &base.join("work")).unwrap();
        let fx = Self { _dir: dir, base, def_path, src };
        fx.photos();
        fx
    }

    fn root(&self) -> PathBuf {
        self.base.join("photos")
    }

    /// Every photo shown, from a full snapshot.
    fn photos(&self) -> Vec<Photo> {
        let (photos, problems) = c1model::conformance::collect(&self.src).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        photos
    }

    /// Walks the roots again: the photos then, and what the walk found.
    fn rescan(&self) -> (Vec<Photo>, ScanOutcome) {
        self.src.rescan().unwrap();
        let photos = self.photos();
        (photos, self.src.last_scan().unwrap())
    }

    /// The id of the photo whose file is `name`.
    fn id(&self, name: &str) -> PhotoId {
        let photos = self.photos();
        photos.iter().find(|p| p.file.name == name).unwrap_or_else(|| panic!("no {name}")).id.clone()
    }

    fn make(&self, id: &PhotoId, tier: ImageTier) -> PathBuf {
        self.src.make_image(id, tier).unwrap().expect("made").path
    }

    fn image(&self, id: &PhotoId, tier: ImageTier) -> Option<PathBuf> {
        self.src.image(id, tier).unwrap().map(|i| i.path)
    }
}

fn custom(pattern: &str) -> ImageStore {
    ImageStore { thumb: pattern.into(), preview: pattern.into(), ..ImageStore::preset(Preset::Custom) }
}

#[test]
fn a_folder_renamed_with_its_previews_keeps_its_images() {
    let fx = Fixture::new(&["2024/rome/a.png", "2024/rome/b.png"], ImageStore::preset(Preset::Beside));
    let a = fx.id("a.png");
    let preview = fx.make(&a, ImageTier::Preview);
    assert_eq!(preview, fx.root().join("2024").join("rome").join(".previews").join("a.preview.jpg"));
    fs::rename(fx.root().join("2024").join("rome"), fx.root().join("2024").join("roma")).unwrap();

    let (photos, scan) = fx.rescan();
    assert_eq!((scan.moved, scan.added, scan.removed), (2, 0, 0), "{scan:?}");
    assert_eq!(photos.len(), 2);
    let previews = fx.root().join("2024").join("roma").join(".previews");
    assert_eq!(fx.image(&a, ImageTier::Preview), Some(previews.join("a.preview.jpg")), "found where it went");
    assert_eq!(fx.image(&a, ImageTier::Thumb), Some(previews.join("a.thumb.jpg")));
}

#[test]
fn a_photo_moved_without_its_images_has_them_made_again() {
    let fx = Fixture::new(&["2024/a.png", "2024/b.png"], ImageStore::preset(Preset::Beside));
    let a = fx.id("a.png");
    let old = fx.make(&a, ImageTier::Preview);
    let old_thumb = fx.image(&a, ImageTier::Thumb).unwrap();
    fs::create_dir_all(fx.root().join("2025")).unwrap();
    fs::rename(fx.root().join("2024").join("a.png"), fx.root().join("2025").join("a.png")).unwrap();

    let (_, scan) = fx.rescan();
    assert_eq!(scan.moved, 1, "{scan:?}");
    assert_eq!(fx.image(&a, ImageTier::Thumb), None, "not served from where the photo was");
    assert_eq!(fx.image(&a, ImageTier::Preview), None);
    assert!(!old.exists() && !old_thumb.exists(), "the old images are deleted");
    let thumb = fx.make(&a, ImageTier::Thumb);
    assert_eq!(thumb, fx.root().join("2025").join(".previews").join("a.thumb.jpg"));
    assert!(thumb.is_file());
}

#[test]
fn central_images_stay_when_their_photo_moves() {
    let fx = Fixture::new(&["2024/a.jpg"], ImageStore::preset(Preset::Central));
    let a = fx.id("a.jpg");
    let thumb = fx.make(&a, ImageTier::Thumb);
    fs::rename(fx.root().join("2024"), fx.root().join("2025")).unwrap();
    let (_, scan) = fx.rescan();
    assert_eq!(scan.moved, 1, "{scan:?}");
    assert_eq!(fx.image(&a, ImageTier::Thumb), Some(thumb.clone()));
    assert!(thumb.is_file());
}

#[test]
fn a_root_whose_path_changed_takes_its_images_along() {
    let fx = Fixture::new(&["x/a.jpg", "b.jpg"], ImageStore::preset(Preset::PerRoot));
    let a = fx.id("a.jpg");
    let b = fx.id("b.jpg");
    fx.make(&a, ImageTier::Thumb);
    let b_thumb = fx.make(&b, ImageTier::Thumb);
    // The drive comes back under another path, its `.c1studio` with it, but B's thumbnail lost.
    let moved = fx.base.join("elsewhere");
    fs::rename(fx.root(), &moved).unwrap();
    fs::remove_file(moved.join(b_thumb.strip_prefix(fx.root()).unwrap())).unwrap();
    let mut def = Definition::load(&fx.def_path).unwrap();
    def.roots[0].path = moved.to_string_lossy().into_owned();
    def.save(&fx.def_path).unwrap();
    fx.src.reload_definition(None).unwrap();

    let (photos, scan) = fx.rescan();
    assert_eq!((scan.moved, scan.added, scan.removed, scan.changed), (0, 0, 0, 0), "{scan:?}");
    assert_eq!(photos.len(), 2);
    let base = moved.join(".c1studio").join("thumb");
    assert_eq!(fx.image(&a, ImageTier::Thumb), Some(base.join("x").join("a.jpg.jpg")));
    assert_eq!(fx.image(&b, ImageTier::Thumb), None);
    assert_eq!(fx.make(&b, ImageTier::Thumb), base.join("b.jpg.jpg"));
}

#[test]
fn images_among_the_photos_are_carried_along_not_taken_for_photos() {
    let fx = Fixture::new(&["rome/a.png"], custom("{dir}/{name}.{tier}.jpg"));
    let a = fx.id("a.png");
    fx.make(&a, ImageTier::Preview);
    let (photos, scan) = fx.rescan();
    assert_eq!((photos.len(), scan.added), (1, 0), "the images are known: {scan:?}");

    fs::rename(fx.root().join("rome"), fx.root().join("roma")).unwrap();
    let (photos, scan) = fx.rescan();
    assert_eq!((photos.len(), scan.added, scan.moved), (1, 0, 1), "{scan:?}");
    assert_eq!(fx.image(&a, ImageTier::Thumb), Some(fx.root().join("roma").join("a.png.thumb.jpg")));
    let (photos, scan) = fx.rescan();
    assert_eq!((photos.len(), scan.added), (1, 0), "nor on the walk after: {scan:?}");
}

#[test]
fn an_image_is_not_made_over_a_photo() {
    let fx = Fixture::new(&["a.jpg", "a.thumb.jpg"], custom("{dir}/{stem}.{tier}.jpg"));
    let problem = fx.src.check_image_store(&custom("{dir}/{stem}.{tier}.jpg")).expect("a problem");
    assert!(problem.contains("would be written over the photo"), "{problem}");
    let a = fx.id("a.jpg");
    let photo = fs::read(fx.root().join("a.thumb.jpg")).unwrap();
    let err = fx.src.make_image(&a, ImageTier::Thumb).unwrap_err();
    assert!(err.to_string().contains("a.thumb.jpg"), "{err}");
    assert_eq!(fs::read(fx.root().join("a.thumb.jpg")).unwrap(), photo);
}

#[test]
fn a_changed_sidecar_is_read_and_the_images_stay() {
    let fx = Fixture::new(&["a.jpg"], ImageStore::preset(Preset::Beside));
    let a = fx.id("a.jpg");
    let thumb = fx.make(&a, ImageTier::Thumb);
    let made = fs::metadata(&thumb).unwrap().modified().unwrap();
    fs::write(
        fx.root().join("a.xmp"),
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="3"/></rdf:RDF></x:xmpmeta>"#,
    )
    .unwrap();

    let (photos, scan) = fx.rescan();
    assert_eq!((scan.changed, scan.read), (1, 1), "{scan:?}");
    assert_eq!(photos[0].rating, Some(3), "the sidecar is read");
    assert_eq!(fx.image(&a, ImageTier::Thumb), Some(thumb.clone()), "the image stays");
    assert_eq!(fs::metadata(&thumb).unwrap().modified().unwrap(), made, "not made again");
}
