use c1index::{Dim, Filter, Library, Presence, QueryOptions, Range, Sort, Trashed, parse_rating};
use c1model::memory::MemorySource;
use c1model::*;

fn photo(n: u32) -> Photo {
    let source = SourceId("mem".into());
    Photo {
        id: PhotoId::new(&source, n.to_string()),
        asset: AssetId(n.to_string()),
        variant_index: 0,
        file: FileInfo {
            name: format!("DSCF{n:04}.RAF"),
            ext: "raf".into(),
            path: Some(format!(r"H:\fotos\{}\DSCF{n:04}.RAF", 2020 + n % 3)),
            size: None,
        },
        // 2023-11-14 plus n days.
        captured_at: Some(1_700_000_000_000 + i64::from(n) * 86_400_000),
        imported_at: None,
        exif: Exif {
            camera_model: Some(if n.is_multiple_of(2) { "X100VI" } else { "X-Pro2" }.into()),
            lens: Some("XF23mmF2 R WR".into()),
            iso: Some(100 * n),
            aperture: Some(2.0 + f64::from(n % 3)),
            ..Exif::default()
        },
        rating: if n.is_multiple_of(5) { None } else { Some((n % 5) as u8) },
        color: if n == 3 { Some(ColorLabel::Red) } else { None },
        keywords: match n % 3 {
            0 => vec![KeywordPath::from_pipe("street"), KeywordPath::from_pipe("best|best2018")],
            1 => vec![KeywordPath::from_pipe("street")],
            _ => vec![],
        },
        iptc: Iptc { description: (n == 7).then(|| "Rynek we Wrocławiu".to_owned()), ..Iptc::default() },
        place: Place { city: (n == 7).then(|| "Wrocław".to_owned()), ..Place::default() },
        geo: (n == 7).then_some(GeoPoint { lat: 51.11, lon: 17.03, alt: None, origin: GeoOrigin::Exif }),
        collections: if n <= 4 { vec![CollectionId("10".into())] } else { vec![] },
        edit: EditInfo { edited: n.is_multiple_of(4), ..EditInfo::default() },
        trashed: n == 9,
    }
}

fn setup() -> (tempfile::TempDir, Library, MemorySource) {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(dir.path()).unwrap();
    let album = Collection {
        id: CollectionId("10".into()),
        name: "for street".into(),
        kind: CollectionKind::Album,
        parent: None,
        path: "Best / for street".into(),
    };
    let source = MemorySource::new("mem", (1..=10).map(photo).collect(), vec![album]);
    let report = library.sync(&source).unwrap();
    assert_eq!((report.full, report.added), (true, 10));
    (dir, library, source)
}

fn ids(library: &Library, filter: &Filter) -> Vec<u32> {
    let result = library.query(filter, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() }).unwrap();
    let mut out: Vec<u32> = result.rows.iter().map(|r| library.photo_id(*r).unwrap().ext.parse().unwrap()).collect();
    out.sort();
    assert_eq!(out.len(), result.total);
    out
}

#[test]
fn trashed_photos_are_hidden_by_default() {
    let (_d, lib, _s) = setup();
    assert_eq!(ids(&lib, &Filter::default()), [1, 2, 3, 4, 5, 6, 7, 8, 10]);
    assert_eq!(ids(&lib, &Filter { trashed: Trashed::Only, ..Filter::default() }), [9]);
}

#[test]
fn rating_ranges_and_presence() {
    let (_d, lib, _s) = setup();
    let f = |r: &str| Filter { rating: Some(parse_rating(r).unwrap()), ..Filter::default() };
    assert_eq!(ids(&lib, &f(">=3")), [3, 4, 8]);
    assert_eq!(ids(&lib, &f("2-3")), [2, 3, 7, 8]);
    assert_eq!(ids(&lib, &f(">3")), [4]);
    assert_eq!(ids(&lib, &f("<2")), [1, 6]);
    assert_eq!(ids(&lib, &f("none")), [5, 10]);
}

#[test]
fn keywords_exact_under_and_not() {
    let (_d, lib, _s) = setup();
    let exact = Filter { keywords: vec!["best|best2018".into()], ..Filter::default() };
    assert_eq!(ids(&lib, &exact), [3, 6]);
    let under = Filter { keywords_under: vec!["best".into()], ..Filter::default() };
    assert_eq!(ids(&lib, &under), [3, 6]);
    // "best" itself is never assigned, only its child.
    let plain = Filter { keywords: vec!["best".into()], ..Filter::default() };
    assert!(ids(&lib, &plain).is_empty());
    let not = Filter { keywords_not: vec!["street".into()], ..Filter::default() };
    assert_eq!(ids(&lib, &not), [2, 5, 8]);
    let none = Filter { unkeyworded: true, ..Filter::default() };
    assert_eq!(ids(&lib, &none), [2, 5, 8]);
}

#[test]
fn substring_text_folder_and_geo() {
    let (_d, lib, _s) = setup();
    assert_eq!(ids(&lib, &Filter { camera: Some("x100".into()), ..Filter::default() }), [2, 4, 6, 8, 10]);
    assert_eq!(ids(&lib, &Filter { city: Some("wro".into()), ..Filter::default() }), [7]);
    // Diacritics fold both ways.
    assert_eq!(ids(&lib, &Filter { text: Some("wroclawiu".into()), ..Filter::default() }), [7]);
    assert_eq!(ids(&lib, &Filter { text: Some("rynek wrocławiu".into()), ..Filter::default() }), [7]);
    assert_eq!(ids(&lib, &Filter { folder: Some(r"H:\fotos\2021".into()), ..Filter::default() }), [1, 4, 7, 10]);
    assert_eq!(ids(&lib, &Filter { folder: Some("h:/FOTOS".into()), ..Filter::default() }).len(), 9);
    assert_eq!(ids(&lib, &Filter { gps: Some(true), ..Filter::default() }), [7]);
    let near = Filter { within: Some(c1index::BBox::around(51.1, 17.0, 10.0)), ..Filter::default() };
    assert_eq!(ids(&lib, &near), [7]);
    assert_eq!(ids(&lib, &Filter { filename: Some("dscf000?.raf".into()), ..Filter::default() }).len(), 8);
}

#[test]
fn numeric_collection_and_edit_filters() {
    let (_d, lib, _s) = setup();
    let iso = Filter { iso: Some(Range::parse("300-500").unwrap()), ..Filter::default() };
    assert_eq!(ids(&lib, &iso), [3, 4, 5]);
    let aperture = Filter { aperture: Some(Range::parse("<=2.0").unwrap()), ..Filter::default() };
    assert_eq!(ids(&lib, &aperture), [3, 6]);
    let album = lib.resolve_collection("for street").unwrap();
    assert_eq!(lib.resolve_collection("FOR STREET").unwrap(), album);
    assert_eq!(ids(&lib, &Filter { collections: vec![album.clone()], ..Filter::default() }), [1, 2, 3, 4]);
    assert_eq!(ids(&lib, &Filter { not_in_collections: vec![album], ..Filter::default() }), [5, 6, 7, 8, 10]);
    assert_eq!(ids(&lib, &Filter { edited: Some(true), ..Filter::default() }), [4, 8]);
    let red = Filter { color: Some(Presence::Values(vec![ColorLabel::Red])), ..Filter::default() };
    assert_eq!(ids(&lib, &red), [3]);
}

#[test]
fn facets_count_the_matches() {
    let (_d, lib, _s) = setup();
    let options = QueryOptions {
        facets: vec![Dim::Camera, Dim::KeywordTree, Dim::Rating],
        limit: Some(0),
        ..QueryOptions::default()
    };
    let result = lib.query(&Filter::default(), &options).unwrap();
    assert_eq!(result.total, 9);
    assert!(result.rows.is_empty());
    let get = |dim: Dim| -> Vec<(String, u64)> { result.facets[&dim].clone() };
    assert_eq!(get(Dim::Camera), [("X100VI".to_owned(), 5), ("X-Pro2".to_owned(), 4)]);
    // "best" counts photos at or under it, without double counting.
    assert!(get(Dim::KeywordTree).contains(&("best".to_owned(), 2)));
    assert!(get(Dim::KeywordTree).contains(&("street".to_owned(), 6)));
    assert!(get(Dim::Rating).contains(&("none".to_owned(), 2)));
}

#[test]
fn facets_can_ignore_their_own_filter() {
    let (_d, lib, _s) = setup();
    let filter = Filter { camera: Some("X100VI".into()), keywords_under: vec!["street".into()], ..Filter::default() };
    let options = QueryOptions {
        facets: vec![Dim::Camera, Dim::KeywordTree, Dim::Rating],
        limit: Some(0),
        facets_ignore_own: true,
        ..QueryOptions::default()
    };
    let result = lib.query(&filter, &options).unwrap();
    assert_eq!(result.total, ids(&lib, &filter).len());
    // Camera counts over "street" alone: both cameras stay visible.
    let street = ids(&lib, &Filter { keywords_under: vec!["street".into()], ..Filter::default() });
    let cameras: u64 = result.facets[&Dim::Camera].iter().map(|(_, n)| n).sum();
    assert_eq!(cameras as usize, street.len());
    assert_eq!(result.facets[&Dim::Camera].len(), 2);
    // Keywords count over the camera alone.
    let x100 = ids(&lib, &Filter { camera: Some("X100VI".into()), ..Filter::default() });
    let unkeyworded = ids(&lib, &Filter { camera: Some("X100VI".into()), unkeyworded: true, ..Filter::default() });
    let street_n = result.facets[&Dim::KeywordTree].iter().find(|(v, _)| v == "street").unwrap().1;
    assert_eq!(street_n as usize, x100.len() - unkeyworded.len());
    // Rating is not filtered, so it counts the matches themselves.
    let rated: u64 = result.facets[&Dim::Rating].iter().map(|(_, n)| n).sum();
    assert_eq!(rated as usize, result.total);
}

#[test]
fn resync_detects_changes_and_deletions() {
    let (_d, lib, source) = setup();
    // Untouched source: incremental, nothing to do.
    let again = lib.sync(&source).unwrap();
    assert!(!again.full);
    assert_eq!((again.added, again.updated, again.deleted), (0, 0, 0));

    let probe = PhotoId::new(&SourceId("mem".into()), "1");
    source.edit_externally(&probe, |p| p.rating = Some(5));
    let report = lib.sync(&source).unwrap();
    assert_eq!((report.full, report.updated, report.unchanged), (true, 1, 9));
    let five = Filter { rating: Some(parse_rating("5").unwrap()), ..Filter::default() };
    assert_eq!(ids(&lib, &five), [1]);

    // A source that no longer reports a photo has deleted it.
    let smaller = MemorySource::new("mem", (1..=8).map(photo).collect(), vec![]);
    let report = lib.sync(&smaller).unwrap();
    assert_eq!(report.deleted, 2);
    assert_eq!(lib.len(), 8);
}

#[test]
fn library_reopens_from_disk() {
    let (dir, lib, _s) = setup();
    drop(lib);
    let reopened = Library::open(dir.path()).unwrap();
    assert_eq!(reopened.len(), 10);
    assert_eq!(ids(&reopened, &Filter { city: Some("wroc".into()), ..Filter::default() }), [7]);
}

#[test]
fn sorting_and_paging() {
    let (_d, lib, _s) = setup();
    let page = |offset| {
        let opts = QueryOptions { sort: Sort::DateDesc, limit: Some(3), offset, ..QueryOptions::default() };
        let r = lib.query(&Filter::default(), &opts).unwrap();
        r.rows.iter().map(|row| lib.photo_id(*row).unwrap().ext).collect::<Vec<_>>()
    };
    assert_eq!(page(0), ["10", "8", "7"]);
    assert_eq!(page(3), ["6", "5", "4"]);
}

#[test]
fn facets_match_brute_force_across_segments() {
    // Thousands of distinct keyword paths span many dictionary blocks, and two sources
    // synced separately give two segments: the case where ordinals are easiest to mix up.
    let make = |source: &str, n: u32| -> Photo {
        let mut p = photo(n);
        p.id = PhotoId::new(&SourceId(source.into()), n.to_string());
        p.keywords = (0..n % 4)
            .map(|k| {
                KeywordPath(vec![format!("root{}", n % 7), format!("branch{}", (n + k) % 50), format!("leaf-{n}-{k}")])
            })
            .collect();
        p.trashed = false;
        p
    };
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let a: Vec<Photo> = (0..1500).map(|n| make("a", n)).collect();
    let b: Vec<Photo> = (1500..3000).map(|n| make("b", n)).collect();
    lib.sync(&MemorySource::new("a", a.clone(), vec![])).unwrap();
    lib.sync(&MemorySource::new("b", b.clone(), vec![])).unwrap();
    assert!(lib.searcher().segment_readers().len() >= 2);

    let mut expected: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for p in a.iter().chain(&b) {
        let mut paths: Vec<String> =
            p.keywords.iter().flat_map(|k| k.ancestors_and_self().collect::<Vec<_>>()).collect();
        paths.sort();
        paths.dedup();
        for path in paths {
            *expected.entry(path).or_default() += 1;
        }
    }
    let options = QueryOptions { facets: vec![Dim::KeywordTree], limit: Some(0), ..QueryOptions::default() };
    let result = lib.query(&Filter::default(), &options).unwrap();
    let got: std::collections::HashMap<String, u64> = result.facets[&Dim::KeywordTree].iter().cloned().collect();
    assert_eq!(got.len(), expected.len());
    assert_eq!(got, expected);
}
