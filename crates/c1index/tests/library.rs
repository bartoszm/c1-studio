use c1index::{
    Before, Bin, Dim, Filter, Hidden, HistoryPick, Library, LogKind, LogQuery, Merged, NetWhat, Origin, Presence,
    QueryOptions, Range, Sort, SyncStep, TimeRange, Trashed, What, parse_rating,
};
use c1model::KeywordRemap;
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
            offline: false,
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
        rule: None,
        system: false,
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
fn a_sync_reports_reading_then_indexing_then_saving() {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(dir.path()).unwrap();
    let source = MemorySource::new("mem", (1..=600).map(photo).collect(), vec![]);
    let mut steps = Vec::new();
    library.sync_with(&source, Origin::Source, &mut |step| steps.push(step)).unwrap();
    assert_eq!(steps.first(), Some(&SyncStep::Reading));
    assert_eq!(steps.last(), Some(&SyncStep::Saving));
    let counts: Vec<(usize, usize)> = steps
        .iter()
        .filter_map(|s| if let SyncStep::Indexing { done, total } = s { Some((*done, *total)) } else { None })
        .collect();
    // Every event is counted: the 600 photos, and what the source sends around them.
    let total = counts[0].1;
    assert!(total >= 600);
    assert!(counts.iter().all(|(_, t)| *t == total));
    assert!(counts.windows(2).all(|w| w[0].0 < w[1].0));
    assert!(counts.len() >= 600 / 256);
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

fn either_of(under: &[&str], rating: &str) -> Vec<Filter> {
    vec![
        Filter { keywords_under: under.iter().map(|s| (*s).to_owned()).collect(), ..Filter::default() },
        Filter { rating: Some(parse_rating(rating).unwrap()), ..Filter::default() },
    ]
}

#[test]
fn multi_value_and_negated_filters() {
    let (_d, lib, _s) = setup();
    let f = |f: Filter| ids(&lib, &f);
    assert_eq!(f(Filter { collections_any: vec!["10".into(), "99".into()], ..Filter::default() }), [1, 2, 3, 4]);
    assert_eq!(f(Filter { uncollected: Some(true), ..Filter::default() }), [5, 6, 7, 8, 10]);
    assert_eq!(f(Filter { uncollected: Some(false), ..Filter::default() }), [1, 2, 3, 4]);
    assert_eq!(f(Filter { cameras: vec!["X100VI".into()], ..Filter::default() }), [2, 4, 6, 8, 10]);
    assert_eq!(f(Filter { cameras_not: vec!["X100VI".into()], ..Filter::default() }), [1, 3, 5, 7]);
    assert!(f(Filter { lenses_not: vec!["XF23mmF2 R WR".into()], ..Filter::default() }).is_empty());
    assert_eq!(f(Filter { colors_not: vec![ColorLabel::Red], ..Filter::default() }), [1, 2, 4, 5, 6, 7, 8, 10]);
    // "No keywords" OR rated 4 and up.
    let either = Filter {
        any_of: vec![
            Filter { unkeyworded: true, ..Filter::default() },
            Filter { rating: Some(parse_rating(">=4").unwrap()), ..Filter::default() },
        ],
        ..Filter::default()
    };
    assert_eq!(f(either), [2, 4, 5, 8]);
    // Match any: counting a dimension leaves out the alternatives that pick among its values.
    let any = Filter {
        all_of: vec![Filter { any_of: either_of(&["street"], ">=4"), ..Filter::default() }],
        ..Filter::default()
    };
    assert_eq!(any.without(Dim::KeywordTree).all_of, [Filter::default()]);
    assert_eq!(any.without(Dim::Camera), any);
    // Picking a camera leaves its alternatives countable.
    let picked = Filter { cameras: vec!["X100VI".into()], ..Filter::default() };
    assert!(picked.without(Dim::Camera).cameras.is_empty());
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
fn map_clusters_the_filtered_photos_with_a_position() {
    let (_d, lib, _s) = setup();
    let world = c1index::BBox { min_lat: -85.0, max_lat: 85.0, min_lon: -180.0, max_lon: 180.0 };
    let view = c1index::MapView { bounds: world, zoom: 2.0 };
    let all = lib.map(&Filter::default(), &view).unwrap();
    assert_eq!((all.located, all.clusters.len(), all.clusters[0].best), (1, 1, 7));
    // Another photo's filter leaves nothing to show; a filter's own area is set aside.
    assert_eq!(lib.map(&Filter { camera: Some("x100".into()), ..Filter::default() }, &view).unwrap().located, 0);
    let near = Filter { within: Some(c1index::BBox::around(10.0, 10.0, 5.0)), ..Filter::default() };
    assert_eq!(lib.map(&near, &view).unwrap().located, 1);
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
fn years_stay_visible_when_a_month_is_picked() {
    let (_d, lib, _s) = setup();
    let options = QueryOptions {
        facets: vec![Dim::Year, Dim::Month],
        limit: Some(0),
        facets_ignore_own: true,
        ..QueryOptions::default()
    };
    let all = lib.query(&Filter::default(), &options).unwrap();
    // A month with no photos: years count as if none were picked, months within the year.
    let empty = Filter { year: Some(2023), month: Some("2023-02".into()), ..Filter::default() };
    let result = lib.query(&empty, &options).unwrap();
    assert_eq!(result.total, 0);
    assert_eq!(result.facets[&Dim::Year], all.facets[&Dim::Year]);
    assert_eq!(result.facets[&Dim::Month], all.facets[&Dim::Month]);
    // Captured bounds are inclusive.
    let day = 86_400_000;
    let at = |n: i64| Some(1_700_000_000_000 + n * day);
    let range = |from, to| Filter { captured: Some(TimeRange { from, to }), ..Filter::default() };
    assert_eq!(ids(&lib, &range(at(2), at(4))), [2, 3, 4]);
    assert_eq!(ids(&lib, &range(at(8), None)), [8, 10]);
    // Weeks nest in months: photo n is taken on 14+n November 2023 (local), so ISO
    // week 47 holds 20 to 24 November.
    let weeks = lib.query(&Filter::default(), &QueryOptions { facets: vec![Dim::Week], ..options.clone() }).unwrap();
    let w47 = Filter { week: Some("2023-11-W47".into()), ..Filter::default() };
    assert_eq!(ids(&lib, &w47), ids(&lib, &range(at(6), at(10))));
    assert!(weeks.facets[&Dim::Week].iter().any(|(k, n)| k == "2023-11-W47" && *n == ids(&lib, &w47).len() as u64));
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
fn a_photos_position_is_its_place_in_the_sorted_result() {
    // Names repeat within and across two segments: equal names go by address.
    let make = |source: &str, n: u32| -> Photo {
        let mut p = photo(n);
        p.id = PhotoId::new(&SourceId(source.into()), n.to_string());
        p.file.name = format!("IMG_{:04}.RAF", (n * 7) % 900);
        p.trashed = false;
        p
    };
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    lib.sync(&MemorySource::new("a", (0..1200).map(|n| make("a", n)).collect(), vec![])).unwrap();
    lib.sync(&MemorySource::new("b", (1200..2400).map(|n| make("b", n)).collect(), vec![])).unwrap();
    assert!(lib.searcher().segment_readers().len() >= 2);

    let rated = Filter { rating: Some(parse_rating(">=2").unwrap()), ..Filter::default() };
    let unrated = Filter { rating: Some(parse_rating("none").unwrap()), ..Filter::default() };
    let outside = lib.query(&unrated, &QueryOptions::default()).unwrap().rows[0];
    for sort in [Sort::Filename, Sort::FilenameDesc, Sort::DateDesc] {
        let options = QueryOptions { sort, ..QueryOptions::default() };
        let rows = lib.query(&rated, &options).unwrap().rows;
        for (i, &row) in rows.iter().enumerate().step_by(7) {
            assert_eq!(lib.position(&rated, &options, row).unwrap(), Some(i), "{sort:?}");
        }
        assert_eq!(lib.position(&rated, &options, outside).unwrap(), None, "{sort:?}");
    }
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

#[test]
fn smart_albums_follow_their_rules() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let smart = |id: &str, match_all, conditions| Collection {
        id: CollectionId(id.into()),
        name: id.into(),
        kind: CollectionKind::Smart,
        parent: None,
        path: id.into(),
        rule: Some(SmartRule { match_all, conditions }),
        system: false,
    };
    let source = MemorySource::new(
        "mem",
        (1..=10).map(photo).collect(),
        vec![
            // Keyword names match ignoring case; "best" matches "best" and "best|best2018".
            smart(
                "street3",
                true,
                vec![Condition::KeywordContains("STREE".into()), Condition::Rating { min: 3, max: 5 }],
            ),
            smart(
                "best-or-4",
                false,
                vec![Condition::KeywordContains("best".into()), Condition::Rating { min: 4, max: 4 }],
            ),
            smart("odd", true, vec![Condition::Unsupported("file name contains DSCF".into())]),
        ],
    );
    lib.sync(&source).unwrap();
    let pick = |id: &str| Filter { collections: vec![id.into()], ..Filter::default() };
    assert_eq!(ids(&lib, &pick("street3")), [3, 4]);
    assert_eq!(ids(&lib, &pick("best-or-4")), [3, 4, 6]);
    assert!(lib.query(&pick("odd"), &QueryOptions::default()).is_err());

    // Counts under other filters, setting aside the album picked.
    let even = Filter { camera: Some("X100VI".into()), collections: vec!["best-or-4".into()], ..Filter::default() };
    let mut counts = lib.smart_counts(&even).unwrap();
    counts.sort();
    assert_eq!(counts, [("best-or-4".to_owned(), 2), ("street3".to_owned(), 1)]);
}

#[test]
fn folders_count_and_filter_as_a_tree() {
    let (_d, lib, _s) = setup();
    let options = QueryOptions { facets: vec![Dim::Folder], limit: Some(0), ..QueryOptions::default() };
    let result = lib.query(&Filter::default(), &options).unwrap();
    let got: std::collections::HashMap<String, u64> = result.facets[&Dim::Folder].iter().cloned().collect();
    assert_eq!(got["H:"], 9);
    assert_eq!(got["H:/fotos"], 9);
    assert_eq!(got["H:/fotos/2020"], 2);
    let in_2021 = Filter { folder: Some("H:/fotos/2021".into()), ..Filter::default() };
    assert_eq!(ids(&lib, &in_2021), [1, 4, 7, 10]);
    // Picking a folder leaves its siblings counted.
    let options = QueryOptions { facets_ignore_own: true, ..options };
    let result = lib.query(&in_2021, &options).unwrap();
    assert_eq!(result.facets[&Dim::Folder].len(), 5);
}

#[test]
fn filters_nest() {
    let (_d, lib, _s) = setup();
    let rated = |n: f64| Filter {
        rating: Some(Presence::Values(Range { min: Some(n), max: Some(n), ..Range::default() })),
        ..Filter::default()
    };
    let either = Filter { any_of: vec![rated(1.0), rated(4.0)], ..Filter::default() };
    assert_eq!(ids(&lib, &either), [1, 4, 6]);
    let both = Filter {
        all_of: vec![either.clone(), Filter { keywords_containing: vec!["Street".into()], ..Filter::default() }],
        ..Filter::default()
    };
    assert_eq!(ids(&lib, &both), [1, 4, 6]);
    let none = Filter {
        all_of: vec![either, Filter { keywords_containing: vec!["best2018".into()], ..Filter::default() }],
        ..Filter::default()
    };
    assert_eq!(ids(&lib, &none), [6]);
}

// ---------------------------------------------------------------------------------------
// Edits made in the app, queued for the source
// ---------------------------------------------------------------------------------------

fn row_of(library: &Library, n: u32) -> u64 {
    let all = library.query(
        &Filter { trashed: Trashed::Include, hidden: Hidden::Include, bin: Bin::Include, ..Filter::default() },
        &QueryOptions::default(),
    );
    all.unwrap().rows.into_iter().find(|r| library.photo_id(*r).unwrap().ext == n.to_string()).unwrap()
}

fn rate(value: Option<u8>) -> Op {
    // The library takes the base from what it holds; the one sent is ignored.
    Op::SetRating { base: None, value }
}

fn rated(library: &Library, stars: f64) -> Vec<u32> {
    let one = Range { min: Some(stars), max: Some(stars), ..Range::default() };
    ids(library, &Filter { rating: Some(Presence::Values(one)), ..Filter::default() })
}

#[test]
fn edits_show_at_once_and_queue_for_the_source() {
    let (_d, lib, source) = setup();
    let row = row_of(&lib, 5); // unrated
    let undo = lib.edit(&[(row, rate(Some(5)))]).unwrap();
    assert_eq!(rated(&lib, 5.0), [5]);
    assert_eq!(lib.photo(row).unwrap().unwrap().rating, Some(5));
    assert_eq!(undo, [(row, Op::SetRating { base: Some(5), value: None })]);

    let pending = lib.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].row, Some(row));
    assert_eq!(pending[0].pending.change.op, Op::SetRating { base: None, value: Some(5) });
    assert_eq!(source.photo(&pending[0].pending.change.photo).unwrap().rating, None, "nothing written yet");

    // A second edit keeps the source's value as the base; undoing it empties the queue.
    lib.edit(&[(row, rate(Some(3)))]).unwrap();
    assert_eq!(lib.pending().unwrap()[0].pending.change.op, Op::SetRating { base: None, value: Some(3) });
    lib.edit(&undo).unwrap();
    assert!(lib.pending().unwrap().is_empty());
    assert_eq!(lib.photo(row).unwrap().unwrap().rating, None);
}

#[test]
fn queued_edits_stay_on_top_of_a_sync() {
    let (_d, lib, source) = setup();
    let row = row_of(&lib, 2); // rated 2
    lib.edit(&[(row, rate(Some(5)))]).unwrap();
    let id = lib.photo_id(row).unwrap();
    source.edit_externally(&id, |p| p.keywords.push(KeywordPath::from_pipe("night")));
    lib.sync(&source).unwrap();

    let shown = lib.photo(row).unwrap().unwrap();
    assert_eq!(shown.rating, Some(5), "the queued rating is kept");
    assert!(shown.keywords.contains(&KeywordPath::from_pipe("night")), "the source's change arrives");
    let pending = lib.pending().unwrap();
    assert_eq!((pending.len(), pending[0].pending.theirs.as_ref()), (1, None));

    let events = lib.events(10).unwrap();
    assert_eq!(events[0].origin, c1index::Origin::Source);
    assert_eq!(op_of(&events[0]), &Op::AddKeywords { paths: vec![KeywordPath::from_pipe("night")] });
    assert_eq!(events[1].origin, c1index::Origin::App);
    assert_eq!(op_of(&events[1]), &Op::SetRating { base: Some(2), value: Some(5) });
}

#[test]
fn a_field_changed_on_both_sides_waits_for_a_choice() {
    let (_d, lib, source) = setup();
    let (row2, row3) = (row_of(&lib, 2), row_of(&lib, 3));
    lib.edit(&[(row2, rate(Some(5))), (row3, rate(Some(5)))]).unwrap();
    let (id2, id3) = (lib.photo_id(row2).unwrap(), lib.photo_id(row3).unwrap());
    source.edit_externally(&id2, |p| p.rating = Some(1));
    source.edit_externally(&id3, |p| p.rating = Some(1));
    lib.sync(&source).unwrap();

    let pending = lib.pending().unwrap();
    assert_eq!(pending[0].pending.theirs, Some(Op::SetRating { base: Some(2), value: Some(1) }));
    assert_eq!(lib.photo(row2).unwrap().unwrap().rating, Some(5), "the user's value shows until they choose");

    // Photo 2: keep mine, now based on the source's new value. Photo 3: take theirs.
    lib.resolve(std::slice::from_ref(&pending[0].key), true).unwrap();
    lib.resolve(std::slice::from_ref(&pending[1].key), false).unwrap();
    let pending = lib.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].pending.change.op, Op::SetRating { base: Some(1), value: Some(5) });
    assert_eq!(pending[0].pending.theirs, None);
    assert_eq!(lib.photo(row3).unwrap().unwrap().rating, Some(1));

    // The kept edit now writes cleanly.
    let plan = source.plan(&[pending[0].pending.change.clone()]).unwrap();
    assert_eq!((plan.apply.len(), plan.conflicts.len()), (1, 0));
}

#[test]
fn the_same_change_made_in_the_source_settles_the_queue() {
    let (_d, lib, source) = setup();
    let row = row_of(&lib, 4);
    lib.edit(&[(row, Op::SetColor { base: None, value: Some(ColorLabel::Green) })]).unwrap();
    source.edit_externally(&lib.photo_id(row).unwrap(), |p| p.color = Some(ColorLabel::Green));
    lib.sync(&source).unwrap();
    assert!(lib.pending().unwrap().is_empty());
    assert_eq!(lib.photo(row).unwrap().unwrap().color, Some(ColorLabel::Green));
}

#[test]
fn writing_the_queue_to_the_source() {
    let (_d, lib, source) = setup();
    let rows = [row_of(&lib, 1), row_of(&lib, 6)];
    lib.edit(&rows.map(|r| (r, rate(Some(4))))).unwrap();
    let changes: Vec<Change> = lib.pending().unwrap().into_iter().map(|p| p.pending.change).collect();
    let plan = source.plan(&changes).unwrap();
    assert_eq!(plan.apply.len(), 2);
    source.apply(&plan).unwrap();
    lib.settle(&plan.apply).unwrap();
    assert!(lib.pending().unwrap().is_empty());
    lib.sync(&source).unwrap();
    assert_eq!(rated(&lib, 4.0), [1, 4, 6]);
    assert!(lib.pending().unwrap().is_empty());
}

#[test]
fn discarding_shows_the_source_value_again() {
    let (_d, lib, _source) = setup();
    let row = row_of(&lib, 3); // red
    lib.edit(&[(row, Op::SetColor { base: None, value: None })]).unwrap();
    assert_eq!(lib.photo(row).unwrap().unwrap().color, None);
    let keys: Vec<String> = lib.pending().unwrap().into_iter().map(|p| p.key).collect();
    lib.discard(&keys).unwrap();
    assert!(lib.pending().unwrap().is_empty());
    assert_eq!(lib.photo(row).unwrap().unwrap().color, Some(ColorLabel::Red));
    let red = ids(&lib, &Filter { color: Some(Presence::Values(vec![ColorLabel::Red])), ..Filter::default() });
    assert_eq!(red, [3]);
}

#[test]
fn queued_edits_outlive_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let source = MemorySource::new("mem", (1..=10).map(photo).collect(), vec![]);
    {
        let lib = Library::open(dir.path()).unwrap();
        lib.sync(&source).unwrap();
        lib.edit(&[(row_of(&lib, 5), rate(Some(5)))]).unwrap();
    }
    // The store and index are a copy of the source and can be dropped (a format change).
    std::fs::remove_file(dir.path().join("store.redb")).unwrap();
    std::fs::remove_dir_all(dir.path().join("index")).unwrap();
    let lib = Library::open(dir.path()).unwrap();
    lib.sync(&source).unwrap();
    assert_eq!(rated(&lib, 5.0), [5]);
    assert_eq!(lib.pending().unwrap().len(), 1);
}

#[test]
fn an_edit_keeps_photos_that_tie_in_place() {
    let dir = tempfile::tempdir().unwrap();
    // Ten photos taken in the same second, imported together, unrated.
    let photos = (1..=10)
        .map(|n| Photo { captured_at: Some(1_700_000_000_000), imported_at: Some(1), rating: None, ..photo(n) })
        .collect();
    let source = MemorySource::new("mem", photos, vec![]);
    let lib = Library::open(dir.path()).unwrap();
    lib.sync(&source).unwrap();
    for sort in [Sort::DateDesc, Sort::Date, Sort::ImportedDesc, Sort::Rating] {
        let order = |lib: &Library| {
            lib.query(&Filter::default(), &QueryOptions { sort, ..QueryOptions::default() }).unwrap().rows
        };
        let before = order(&lib);
        // Re-adds the document of a photo in the middle; a colour does not change any key.
        lib.edit(&[(before[4], Op::SetColor { base: None, value: Some(ColorLabel::Blue) })]).unwrap();
        assert_eq!(order(&lib), before, "{sort:?}");
    }
}

#[test]
fn keyword_edits_queue_one_keyword_each() {
    let (_d, lib, source) = setup();
    let (row1, row2) = (row_of(&lib, 1), row_of(&lib, 2)); // 1 has street; 2 has none
    let add = |paths: &[&str]| Op::AddKeywords { paths: paths.iter().map(|p| KeywordPath::from_pipe(p)).collect() };
    let remove =
        |paths: &[&str]| Op::RemoveKeywords { paths: paths.iter().map(|p| KeywordPath::from_pipe(p)).collect() };

    let undo = lib.edit(&[(row1, add(&["street", "night"])), (row2, add(&["street", "night"]))]).unwrap();
    // Photo 1 already had street: three keywords were added, not four.
    assert_eq!(undo.len(), 3);
    let night = KeywordPath::from_pipe("night");
    assert_eq!(ids(&lib, &Filter { keywords_under: vec!["night".into()], ..Filter::default() }), [1, 2]);
    let keys: Vec<String> =
        lib.pending().unwrap().into_iter().map(|p| p.key.rsplit('\u{1f}').next().unwrap().to_owned()).collect();
    assert_eq!(keys, ["keyword:night", "keyword:night", "keyword:street"]);

    // Taking night off photo 2 again leaves nothing queued for it.
    lib.edit(&[(row2, remove(&["night"]))]).unwrap();
    assert_eq!(lib.pending().unwrap().len(), 2);

    // Written to the source, then synced: the queue empties and the keywords stay.
    let changes: Vec<Change> = lib.pending().unwrap().into_iter().map(|p| p.pending.change).collect();
    let plan = source.plan(&changes).unwrap();
    source.apply(&plan).unwrap();
    lib.settle(&plan.apply).unwrap();
    lib.sync(&source).unwrap();
    assert!(lib.pending().unwrap().is_empty());
    assert!(lib.photo(row1).unwrap().unwrap().keywords.contains(&night));
    assert_eq!(lib.photo(row2).unwrap().unwrap().keywords, vec![KeywordPath::from_pipe("street")]);

    // Undo puts the photos back as they were before the first edit (queued again).
    lib.edit(&undo).unwrap();
    assert!(!lib.photo(row1).unwrap().unwrap().keywords.contains(&night));
}

fn under(library: &Library, path: &str) -> Vec<u32> {
    ids(library, &Filter { keywords_under: vec![path.into()], ..Filter::default() })
}

#[test]
fn a_queued_rename_shows_at_once_survives_a_sync_and_writes() {
    let (_d, lib, source) = setup();
    let street = under(&lib, "street");
    let row2 = row_of(&lib, 2); // no keywords
    lib.edit(&[(row2, Op::AddKeywords { paths: vec![KeywordPath::from_pipe("street")] })]).unwrap();

    lib.remap_keywords(KeywordRemap::rename(KeywordPath::from_pipe("street"), "avenue")).unwrap();
    let mut expected = street.clone();
    expected.push(2);
    expected.sort();
    assert_eq!(under(&lib, "avenue"), expected);
    assert!(under(&lib, "street").is_empty());
    let pending = lib.pending().unwrap();
    assert_eq!(
        pending[0].pending.change.op,
        Op::AddKeywords { paths: vec![KeywordPath::from_pipe("avenue")] },
        "the queued add follows"
    );
    assert!(lib.keywords().unwrap().iter().any(|n| n.path == KeywordPath::from_pipe("avenue")));

    // Capture One changes photo 1 meanwhile: its keywords arrive renamed.
    source.edit_externally(&lib.photo_id(row_of(&lib, 1)).unwrap(), |p| p.rating = Some(5));
    lib.sync(&source).unwrap();
    assert_eq!(under(&lib, "avenue"), expected);

    // Written: tree change first, then the photo edits planned as they will be.
    let remaps: Vec<_> = lib.remaps().unwrap();
    let changes: Vec<Change> = lib.pending().unwrap().into_iter().map(|p| p.pending.change).collect();
    let plan =
        source.plan_after(&remaps.iter().map(|(_, q)| q.remap.clone()).collect::<Vec<_>>(), &[], &changes).unwrap();
    assert_eq!(plan.apply.len(), 1);
    source.apply(&plan).unwrap();
    lib.settle_remaps(&remaps.iter().map(|(s, _)| *s).collect::<Vec<_>>()).unwrap();
    lib.settle(&plan.apply).unwrap();
    lib.sync(&source).unwrap();
    assert!(lib.remaps().unwrap().is_empty() && lib.pending().unwrap().is_empty());
    assert_eq!(under(&lib, "avenue"), expected);
    assert!(source.photo(&lib.photo_id(row2).unwrap()).unwrap().keywords.contains(&KeywordPath::from_pipe("avenue")));
}

#[test]
fn undoing_a_queued_join_brings_the_source_keywords_back() {
    let (_d, lib, source) = setup();
    let best = under(&lib, "best");
    let seq = lib
        .remap_keywords(KeywordRemap::join(vec![KeywordPath::from_pipe("best")], KeywordPath::from_pipe("street")))
        .unwrap();
    assert!(under(&lib, "best").is_empty());
    assert_eq!(under(&lib, "street|best2018"), best);
    lib.undo_remap(seq).unwrap();
    lib.sync(&source).unwrap();
    assert_eq!(under(&lib, "best"), best);
    assert!(under(&lib, "street|best2018").is_empty());
}

#[test]
fn grouping_and_ungrouping_show_in_the_keyword_list() {
    let (_d, lib, _source) = setup();
    let paths = |lib: &Library| {
        let mut p: Vec<String> = lib.keywords().unwrap().into_iter().map(|n| n.path.to_pipe()).collect();
        p.sort();
        p
    };
    let before = paths(&lib);
    lib.remap_keywords(KeywordRemap::group(
        vec![KeywordPath::from_pipe("best"), KeywordPath::from_pipe("street")],
        Some(KeywordPath::from_pipe("things")),
    ))
    .unwrap();
    assert_eq!(paths(&lib), ["things", "things|best", "things|best|best2018", "things|street"]);
    let children = vec![KeywordPath::from_pipe("things|best"), KeywordPath::from_pipe("things|street")];
    lib.remap_keywords(KeywordRemap::ungroup(KeywordPath::from_pipe("things"), children)).unwrap();
    assert_eq!(paths(&lib), before);
}

#[test]
fn removing_a_keyword_takes_the_keywords_under_it() {
    let (_d, lib, _source) = setup();
    let row = row_of(&lib, 3); // street, best|best2018
    let kws = |lib: &Library| lib.photo(row).unwrap().unwrap().keywords.iter().map(|k| k.to_pipe()).collect::<Vec<_>>();
    lib.edit(&[(
        row,
        Op::AddKeywords { paths: vec![KeywordPath::from_pipe("best"), KeywordPath::from_pipe("best|top")] },
    )])
    .unwrap();
    assert_eq!(kws(&lib), ["street", "best|best2018", "best", "best|top"]);

    let undo = lib.edit(&[(row, Op::RemoveKeywords { paths: vec![KeywordPath::from_pipe("best")] })]).unwrap();
    assert_eq!(kws(&lib), ["street"]);
    assert_eq!(undo.len(), 3, "best, best|best2018 and best|top each come back on undo");
    // best and best|top were only queued adds: removing them cancels those. best|best2018
    // came from the source: its removal is queued.
    let queued: Vec<String> =
        lib.pending().unwrap().into_iter().map(|p| p.key.rsplit('\u{1f}').next().unwrap().to_owned()).collect();
    assert_eq!(queued, ["keyword:best|best2018"]);

    lib.edit(&undo).unwrap();
    let mut back = kws(&lib);
    back.sort();
    assert_eq!(back, ["best", "best|best2018", "best|top", "street"]);
}

#[test]
fn places_nest_country_state_city_location_with_gaps_kept() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let place = |country: Option<&str>, state: Option<&str>, city: Option<&str>, code: Option<&str>| Place {
        country: country.map(str::to_owned),
        state: state.map(str::to_owned),
        city: city.map(str::to_owned),
        country_code: code.map(str::to_owned),
        ..Place::default()
    };
    let photos: Vec<Photo> = [
        place(Some("Poland"), Some("Malopolska"), Some("Kraków"), Some("PL")),
        place(Some("Poland"), None, Some("Kraków"), None),
        place(None, None, Some("London"), None),
        place(Some("Japan"), None, None, None),
        Place::default(),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, p)| Photo { place: p, ..photo(i as u32 + 1) })
    .collect();
    lib.sync(&MemorySource::new("mem", photos, vec![])).unwrap();
    let options = QueryOptions { facets: vec![Dim::PlaceTree], limit: Some(0), ..QueryOptions::default() };
    let tree: std::collections::HashMap<String, u64> =
        lib.query(&Filter::default(), &options).unwrap().facets[&Dim::PlaceTree].iter().cloned().collect();
    assert_eq!(tree["Poland"], 2);
    assert_eq!(tree["Poland|Malopolska|Kraków"], 1);
    // No state: its own level, so Kraków is not counted twice under one parent.
    assert_eq!(tree["Poland|—|Kraków"], 1);
    assert_eq!(tree["—|—|London"], 1);
    assert_eq!(tree["Japan"], 1);
    assert!(!tree.contains_key("Japan|—"), "no trailing empty levels");
    let under = |p: &str| Filter { place_under: Some(p.into()), ..Filter::default() };
    assert_eq!(ids(&lib, &under("Poland")), [1, 2]);
    assert_eq!(ids(&lib, &under("Poland|—")), [2]);
    // A country without its ISO code.
    assert_eq!(ids(&lib, &Filter { no_country_code: true, ..Filter::default() }), [2, 4]);
}

#[test]
fn a_city_typed_here_is_found_at_once() {
    let (_dir, lib, _source) = setup();
    let row = row_of(&lib, 3);
    let base = lib.photo(row).unwrap().unwrap().place;
    let value = Place { city: Some("Zakopane".into()), ..base.clone() };
    lib.edit(&[(row, Op::SetPlace { base, value })]).unwrap();
    assert_eq!(ids(&lib, &Filter { city: Some("zakop".into()), ..Filter::default() }), [3]);
}

#[test]
fn field_edits_build_on_each_other() {
    let (_dir, lib, _source) = setup();
    let row = row_of(&lib, 3);
    // Each edit sets one field on the place the photo has when it runs.
    let set = |field: fn(&mut Place) -> &mut Option<String>, text: &str| {
        let text = text.to_owned();
        move |_: u64, photo: &Photo| {
            let mut value = photo.place.clone();
            *field(&mut value) = Some(text.clone());
            Ok(vec![Op::SetPlace { base: photo.place.clone(), value }])
        }
    };
    // The state and location are asked for while the country is still being written,
    // as when they are typed during a save: they wait, then build on it.
    let lib = &lib;
    std::thread::scope(|s| {
        lib.edit_photos(&[row], |row, photo| {
            s.spawn(move || lib.edit_photos(&[row], set(|p| &mut p.state, "New York")).unwrap());
            s.spawn(move || lib.edit_photos(&[row], set(|p| &mut p.location, "Brooklyn Bridge")).unwrap());
            std::thread::sleep(std::time::Duration::from_millis(100));
            set(|p| &mut p.country, "United States")(row, photo)
        })
        .unwrap();
    });
    let place = lib.photo(row).unwrap().unwrap().place;
    assert_eq!(
        (place.country.as_deref(), place.state.as_deref(), place.location.as_deref()),
        (Some("United States"), Some("New York"), Some("Brooklyn Bridge"))
    );
    // Queued as one place edit for the photo.
    assert_eq!(lib.pending().unwrap().len(), 1);
}

fn in_album(library: &Library, id: &CollectionId) -> Vec<u32> {
    ids(library, &Filter { collections: vec![id.0.clone()], ..Filter::default() })
}

fn add_to(id: &CollectionId) -> Op {
    Op::AddToCollection { collection: id.clone() }
}

#[test]
fn albums_made_here_fill_show_and_write() {
    let (_d, lib, source) = setup();
    let trips = CollectionId::new_local("g-1");
    let japan = CollectionId::new_local("a-1");
    let best = CollectionId::new_local("s-1");
    let create = |id: &CollectionId, name: &str, kind, parent: Option<&CollectionId>, rule| CollectionChange::Create {
        id: id.clone(),
        name: name.into(),
        kind,
        parent: parent.cloned(),
        rule,
    };
    lib.change_collection(create(&trips, "trips", CollectionKind::Group, None, None)).unwrap();
    lib.change_collection(create(&japan, "japan", CollectionKind::Album, Some(&trips), None)).unwrap();
    let rule = SmartRule { match_all: true, conditions: vec![Condition::Rating { min: 3, max: 5 }] };
    lib.change_collection(create(&best, "good", CollectionKind::Smart, None, Some(rule))).unwrap();
    let all = lib.collections().unwrap();
    assert_eq!(all.iter().find(|c| c.id == japan).unwrap().path, "trips / japan");

    lib.edit(&[(row_of(&lib, 5), add_to(&japan)), (row_of(&lib, 6), add_to(&japan))]).unwrap();
    assert_eq!(in_album(&lib, &japan), [5, 6]);
    // A smart album is its rule: photos 3 and 8 are rated 3, 4 is rated 4.
    assert_eq!(in_album(&lib, &best), [3, 4, 8]);
    let err = lib.edit(&[(row_of(&lib, 1), add_to(&best))]).unwrap_err().to_string();
    assert!(err.contains("smart album"), "{err}");

    // Written as one: albums first, then the photos put into them.
    let queued = lib.collection_changes().unwrap();
    let changes: Vec<Change> = lib.pending().unwrap().into_iter().map(|p| p.pending.change).collect();
    let plan =
        source.plan_after(&[], &queued.iter().map(|(_, q)| q.change.clone()).collect::<Vec<_>>(), &changes).unwrap();
    assert_eq!((plan.collections.len(), plan.apply.len()), (3, 2));
    source.apply(&plan).unwrap();
    lib.settle_collections(&queued.iter().map(|(s, _)| *s).collect::<Vec<_>>()).unwrap();
    lib.settle(&plan.apply).unwrap();
    lib.sync(&source).unwrap();
    assert!(lib.collection_changes().unwrap().is_empty() && lib.pending().unwrap().is_empty());
    // Now under the source's own ids.
    let all = lib.collections().unwrap();
    let japan = all.iter().find(|c| c.path == "trips / japan").unwrap();
    assert!(japan.id.local_uuid().is_none());
    assert_eq!(in_album(&lib, &japan.id), [5, 6]);
    let good = all.iter().find(|c| c.name == "good").unwrap();
    assert_eq!(in_album(&lib, &good.id), [3, 4, 8]);
}

#[test]
fn deleting_an_album_and_undoing_it() {
    let (_d, lib, source) = setup();
    let album = CollectionId("10".into());
    assert_eq!(in_album(&lib, &album), [1, 2, 3, 4]);
    lib.edit(&[(row_of(&lib, 5), add_to(&album))]).unwrap();
    let seq = lib.change_collection(CollectionChange::Delete { id: album.clone() }).unwrap();
    assert!(lib.collections().unwrap().is_empty());
    assert!(lib.pending().unwrap().is_empty(), "putting a photo into it is pointless now");
    assert!(lib.edit(&[(row_of(&lib, 6), add_to(&album))]).is_err());
    lib.undo_collection(seq).unwrap();
    assert_eq!(lib.collections().unwrap().len(), 1);
    assert_eq!(lib.pending().unwrap().len(), 1, "the edit it dropped is back");

    // Deleted for real: the source loses the album and every photo's place in it.
    lib.change_collection(CollectionChange::Delete { id: album.clone() }).unwrap();
    let queued = lib.collection_changes().unwrap();
    let plan = source.plan_after(&[], &[queued[0].1.change.clone()], &[]).unwrap();
    source.apply(&plan).unwrap();
    lib.settle_collections(&[queued[0].0]).unwrap();
    lib.sync(&source).unwrap();
    assert!(lib.collections().unwrap().is_empty());
    assert!(source.photo(&lib.photo_id(row_of(&lib, 1)).unwrap()).unwrap().collections.is_empty());
}

#[test]
fn undoing_a_new_album_takes_what_was_queued_for_it() {
    let (_d, lib, _source) = setup();
    let group = CollectionId::new_local("g-1");
    let album = CollectionId::new_local("a-1");
    let seq = lib
        .change_collection(CollectionChange::Create {
            id: group.clone(),
            name: "trips".into(),
            kind: CollectionKind::Group,
            parent: None,
            rule: None,
        })
        .unwrap();
    lib.change_collection(CollectionChange::Create {
        id: album.clone(),
        name: "japan".into(),
        kind: CollectionKind::Album,
        parent: Some(group.clone()),
        rule: None,
    })
    .unwrap();
    lib.change_collection(CollectionChange::Rename { id: album.clone(), name: "nippon".into() }).unwrap();
    lib.edit(&[(row_of(&lib, 5), add_to(&album))]).unwrap();
    assert_eq!(lib.collections().unwrap().iter().find(|c| c.id == album).unwrap().path, "trips / nippon");
    lib.undo_collection(seq).unwrap();
    assert!(lib.collection_changes().unwrap().is_empty());
    assert!(lib.pending().unwrap().is_empty());
    assert_eq!(lib.collections().unwrap().len(), 1);
    assert!(lib.photo(row_of(&lib, 5)).unwrap().unwrap().collections.is_empty());
}

fn look(w: u32, h: u32, rgb: [u8; 3], sharpness: f32, hash: u64) -> c1analysis::Analysis {
    c1analysis::Analysis {
        width: w,
        height: h,
        palette: vec![c1analysis::Swatch { rgb, share: 1.0 }],
        chroma: 40.0,
        brightness: 0.5,
        contrast: 0.2,
        highlights: 0.0,
        shadows: 0.0,
        sharpness,
        hash,
        hue_stats: c1analysis::stats_from_palette(&[c1analysis::Swatch { rgb, share: 1.0 }], 40.0),
    }
}

#[test]
fn analyses_are_searchable_and_grouped() {
    let (_d, lib, source) = setup();
    let key = |n: u32| lib.photo_id(row_of(&lib, n)).unwrap().key();
    let h = 0x0f0f_3c3c_5a5a_a5a5u64;
    lib.put_analyses(vec![
        (key(1), (1, 1), look(600, 400, [40, 80, 210], 5000.0, h)),
        (key(2), (1, 1), look(400, 600, [40, 80, 210], 10.0, h ^ 1)),
        (key(3), (1, 1), look(600, 400, [200, 30, 30], 400.0, !h)),
    ])
    .unwrap();
    assert_eq!(ids(&lib, &Filter { shape: vec!["portrait".into()], ..Filter::default() }), [2]);
    assert_eq!(ids(&lib, &Filter { hues: vec!["blue".into()], ..Filter::default() }), [1, 2]);
    assert_eq!(ids(&lib, &Filter { focus: vec!["blurry".into(), "soft".into()], ..Filter::default() }), [2, 3]);
    assert_eq!(ids(&lib, &Filter { analysed: Some(false), ..Filter::default() }), [4, 5, 6, 7, 8, 10]);
    // 1 and 2 look the same, but were taken a day apart: not duplicates.
    assert_eq!(lib.regroup().unwrap(), 0);
    // Taken at the same instant (imported twice), they are; the sync that brings the new
    // time works the groups out again.
    let one = source.photo(&lib.photo_id(row_of(&lib, 1)).unwrap()).unwrap().captured_at;
    source.edit_externally(&lib.photo_id(row_of(&lib, 2)).unwrap(), |p| p.captured_at = one);
    lib.sync(&source).unwrap();
    assert_eq!(lib.regroup().unwrap(), 0, "the sync already grouped them");
    assert_eq!(ids(&lib, &Filter { duplicates: true, ..Filter::default() }), [1, 2]);
    let group = lib.look(row_of(&lib, 2)).unwrap().duplicates.unwrap();
    assert_eq!(ids(&lib, &Filter { group: Some(group), ..Filter::default() }), [1, 2]);
    assert_eq!(lib.regroup().unwrap(), 0, "nothing changed");
    let groups = lib.groups(&Filter::default(), false, 0, 10).unwrap().groups;
    assert_eq!(groups.len(), 1);
    assert_eq!((groups[0].size, groups[0].rows.len()), (2, 2));
    assert!(lib.groups(&Filter::default(), true, 0, 10).unwrap().groups.is_empty());
    // Counted per value, as the sidebar shows them.
    let r = lib
        .query(&Filter::default(), &QueryOptions { facets: vec![Dim::Shape, Dim::Focus], ..QueryOptions::default() })
        .unwrap();
    assert!(r.facets[&Dim::Shape].contains(&("landscape".into(), 2)));
    assert_eq!(lib.analysed().len(), 3);
}

fn ordered(lib: &Library, filter: &Filter, sort: Sort) -> Vec<u32> {
    let rows = lib.query(filter, &QueryOptions { sort, ..QueryOptions::default() }).unwrap().rows;
    rows.iter().map(|r| lib.photo_id(*r).unwrap().ext.parse().unwrap()).collect()
}

#[test]
fn photos_sort_by_what_the_analysis_measured_and_unmeasured_ones_come_last() {
    let (_d, lib, _source) = setup();
    let key = |n: u32| lib.photo_id(row_of(&lib, n)).unwrap().key();
    let mut dim = look(600, 400, [90; 3], 400.0, 3);
    dim.brightness = 0.1;
    let mut bright = look(600, 400, [90; 3], 10.0, 2);
    bright.brightness = 0.9;
    lib.put_analyses(vec![
        (key(1), (1, 1), look(600, 400, [90; 3], 5000.0, 1)),
        (key(2), (1, 1), bright),
        (key(3), (1, 1), dim),
    ])
    .unwrap();
    let measured = Filter { analysed: Some(true), ..Filter::default() };
    assert_eq!(ordered(&lib, &measured, Sort::SharpnessDesc), [1, 3, 2]);
    assert_eq!(ordered(&lib, &measured, Sort::Sharpness), [2, 3, 1]);
    assert_eq!(ordered(&lib, &measured, Sort::BrightnessDesc)[0], 2);
    assert_eq!(ordered(&lib, &measured, Sort::Brightness)[2], 2);
    // The nine photos: the three measured first, in either direction.
    for sort in [Sort::SharpnessDesc, Sort::Sharpness, Sort::BrightnessDesc, Sort::Brightness] {
        let all = ordered(&lib, &Filter::default(), sort);
        assert_eq!(all.len(), 9);
        let mut first: Vec<u32> = all[..3].to_vec();
        first.sort();
        assert_eq!(first, [1, 2, 3], "{sort:?}");
    }
}

#[test]
fn best_match_ranks_by_the_picked_colours_and_falls_back_to_date() {
    let (_d, lib, _source) = setup();
    let key = |n: u32| lib.photo_id(row_of(&lib, n)).unwrap().key();
    let (red, green, blue) = ([210, 30, 30], [40, 160, 50], [40, 80, 210]);
    let with = |parts: &[([u8; 3], f32)]| {
        let mut a = look(600, 400, red, 5000.0, 1);
        a.palette = parts.iter().map(|(rgb, share)| c1analysis::Swatch { rgb: *rgb, share: *share }).collect();
        a.hue_stats = c1analysis::stats_from_palette(&a.palette, 40.0);
        a
    };
    lib.put_analyses(vec![
        (key(1), (1, 1), with(&[(red, 0.6), (blue, 0.4)])),
        (key(2), (1, 1), with(&[(red, 0.2), (green, 0.3), (blue, 0.5)])),
        (key(3), (1, 1), with(&[(green, 0.9), (blue, 0.1)])),
        (key(4), (1, 1), with(&[(blue, 1.0)])),
    ])
    .unwrap();
    let picked = Filter { hues: vec!["red".into(), "green".into()], ..Filter::default() };
    // The share in red or green: 3 (0.9), 1 (0.6), 2 (0.5); 4 is not in either.
    assert_eq!(ordered(&lib, &picked, Sort::Match), [3, 1, 2]);
    assert_eq!(ordered(&lib, &picked, Sort::MatchWorst), [2, 1, 3]);
    // One colour: the red share alone.
    let red_only = Filter { hues: vec!["red".into()], ..Filter::default() };
    assert_eq!(ordered(&lib, &red_only, Sort::Match), [1, 2]);
    // Pages of the ranking add up to the whole.
    let page = |offset| {
        let opts = QueryOptions { sort: Sort::Match, limit: Some(2), offset, ..QueryOptions::default() };
        let rows = lib.query(&picked, &opts).unwrap().rows;
        rows.iter().map(|r| lib.photo_id(*r).unwrap().ext.parse::<u32>().unwrap()).collect::<Vec<_>>()
    };
    assert_eq!([page(0), page(2)].concat(), [3, 1, 2]);
    // Nothing picked to match: the newest first, as the date sort.
    assert_eq!(ordered(&lib, &Filter::default(), Sort::Match), ordered(&lib, &Filter::default(), Sort::DateDesc));
    // Picks asked for all at once (as `all_of` alternatives) rank the same way.
    let both = Filter {
        all_of: vec![
            Filter { hues: vec!["red".into()], ..Filter::default() },
            Filter { hues: vec!["blue".into()], ..Filter::default() },
        ],
        ..Filter::default()
    };
    assert_eq!(ordered(&lib, &both, Sort::Match), [1, 2], "red and blue: 1 (60% red) before 2 (20%)");
    // A shape pick has no measure either.
    let shape = Filter { shape: vec!["landscape".into()], ..Filter::default() };
    assert_eq!(ordered(&lib, &shape, Sort::Match), ordered(&lib, &shape, Sort::DateDesc));
}

#[test]
fn best_match_averages_the_dimensions_and_ranks_unmeasured_photos_last() {
    let (_d, lib, _source) = setup();
    let key = |n: u32| lib.photo_id(row_of(&lib, n)).unwrap().key();
    let red = [210, 30, 30];
    // All red; 1 is sharp, 2 soft; 3 is sharper than 2 but not red enough to be matched
    // differently: only sharpness tells them apart.
    lib.put_analyses(vec![
        (key(1), (1, 1), look(600, 400, red, 9000.0, 1)),
        (key(2), (1, 1), look(600, 400, red, 800.0, 2)),
        (key(3), (1, 1), look(600, 400, red, 3000.0, 3)),
    ])
    .unwrap();
    let both = Filter { hues: vec!["red".into()], focus: vec!["sharp".into()], ..Filter::default() };
    assert_eq!(ordered(&lib, &both, Sort::Match), [1, 3, 2]);
    assert_eq!(ordered(&lib, &both, Sort::MatchWorst), [2, 3, 1]);
    // Soft or sharp: the better of the two counts, so all three are still matched.
    let either = Filter { hues: vec!["red".into()], focus: vec!["soft".into(), "sharp".into()], ..Filter::default() };
    assert_eq!(ordered(&lib, &either, Sort::Match).len(), 3);
}

#[test]
fn stacking_under_a_filter_hides_only_frames_whose_sharpest_matches_and_follows_the_sync() {
    let (_d, lib, source) = setup();
    let id = |n: u32| lib.photo_id(row_of(&lib, n)).unwrap();
    let h = 0x0f0f_3c3c_5a5a_a5a5u64;
    let (red, blue) = ([210, 30, 30], [40, 80, 210]);
    // A burst of three, seconds apart (1, 3, 5): 3 is the sharpest and the only blue one.
    lib.put_analyses(vec![
        (id(1).key(), (1, 1), look(600, 400, red, 100.0, h)),
        (id(3).key(), (1, 1), look(600, 400, blue, 900.0, h ^ 0b11)),
        (id(5).key(), (1, 1), look(600, 400, red, 300.0, h ^ 0b1111)),
    ])
    .unwrap();
    let at = source.photo(&id(1)).unwrap().captured_at.unwrap();
    for (k, n) in [3, 5].into_iter().enumerate() {
        source.edit_externally(&id(n), |p| p.captured_at = Some(at + 2000 * (k as i64 + 1)));
    }
    lib.sync(&source).unwrap();

    let stacked = |f: Filter| ids(&lib, &Filter { stacked: true, ..f });
    // Nothing else picked: only the sharpest of the burst shows.
    assert_eq!(stacked(Filter::default()), [2, 3, 4, 6, 7, 8, 10]);
    // Red picks 1 and 5, but their sharpest (3) is blue: both stay, the burst does not vanish.
    assert_eq!(stacked(Filter { hues: vec!["red".into()], ..Filter::default() }), [1, 5]);
    // Blue picks only the sharpest.
    assert_eq!(stacked(Filter { hues: vec!["blue".into()], ..Filter::default() }), [3]);
    // Measured (all three): the sharpest matches, so the others are stacked away.
    assert_eq!(stacked(Filter { analysed: Some(true), ..Filter::default() }), [3]);
    // One group picked: all its frames show.
    let group = lib.look(row_of(&lib, 3)).unwrap().series.unwrap();
    assert_eq!(stacked(Filter { group: Some(group), ..Filter::default() }), [1, 3, 5]);

    // The sharpest is trashed in the source: two frames are no burst, so none stays hidden.
    source.edit_externally(&id(3), |p| p.trashed = true);
    lib.sync(&source).unwrap();
    assert_eq!(stacked(Filter { analysed: Some(true), ..Filter::default() }), [1, 5]);
}

#[test]
fn colour_labels_sort_both_ways_with_unlabelled_photos_last() {
    let (_d, lib, source) = setup();
    for (n, label) in [(6, c1model::ColorLabel::Blue), (8, c1model::ColorLabel::Purple)] {
        source.edit_externally(&lib.photo_id(row_of(&lib, n)).unwrap(), |p| p.color = Some(label));
    }
    lib.sync(&source).unwrap();
    // Photo 3 is red from the start.
    let all = Filter::default();
    assert_eq!(ordered(&lib, &all, Sort::ColorLabel)[..3], [3, 6, 8]);
    assert_eq!(ordered(&lib, &all, Sort::ColorLabelRev)[..3], [8, 6, 3]);
    assert_eq!(ordered(&lib, &all, Sort::ColorLabel).len(), 9, "unlabelled photos follow");
}

#[test]
fn stacking_keeps_the_sharpest_of_each_group_and_groups_sort() {
    let (_d, lib, source) = setup();
    let id = |n: u32| lib.photo_id(row_of(&lib, n)).unwrap();
    let h = 0x0f0f_3c3c_5a5a_a5a5u64;
    let (a, b) = (0x3333_cccc_5555_aaaau64, 0x5555_0f0f_3333_ccccu64);
    let c = [90, 90, 90];
    // A burst of three (1, 3, 5: X-Pro2, seconds apart; 3 is sharpest) and two duplicate
    // pairs: 2 and 4 (4 is sharper), 6 and 8 (8 is sharper, and the sharpest of all).
    lib.put_analyses(vec![
        (id(1).key(), (1, 1), look(600, 400, c, 100.0, h)),
        (id(3).key(), (1, 1), look(600, 400, c, 900.0, h ^ 0b11)),
        (id(5).key(), (1, 1), look(600, 400, c, 300.0, h ^ 0b1111)),
        (id(2).key(), (1, 1), look(600, 400, c, 50.0, a)),
        (id(4).key(), (1, 1), look(600, 400, c, 60.0, a ^ 1)),
        (id(6).key(), (1, 1), look(600, 400, c, 70.0, b)),
        (id(8).key(), (1, 1), look(600, 400, c, 9000.0, b ^ 1)),
    ])
    .unwrap();
    let at = source.photo(&id(1)).unwrap().captured_at.unwrap();
    for (k, n) in [3, 5].into_iter().enumerate() {
        source.edit_externally(&id(n), |p| p.captured_at = Some(at + 2000 * (k as i64 + 1)));
    }
    for (same_as, n) in [(2, 4), (6, 8)] {
        let when = source.photo(&id(same_as)).unwrap().captured_at;
        source.edit_externally(&id(n), |p| p.captured_at = when);
    }
    lib.sync(&source).unwrap();
    lib.regroup().unwrap();

    assert_eq!(ids(&lib, &Filter::default()).len(), 9);
    let stacked = Filter { stacked: true, ..Filter::default() };
    // Photos 1 and 5 (burst), 2 (pair) and 6 (pair) are stacked away.
    assert_eq!(ids(&lib, &stacked), [3, 4, 7, 8, 10]);
    // One group picked: all of its photos show.
    let group = lib.look(row_of(&lib, 3)).unwrap().series.unwrap();
    assert_eq!(ids(&lib, &Filter { group: Some(group), stacked: true, ..Filter::default() }), [1, 3, 5]);
    // The groups themselves are counted whole, stacked or not.
    let pairs = |sort| lib.groups_sorted(&stacked, false, sort, false, 0, 10).unwrap().groups;
    assert!(pairs(c1index::GroupSort::Matched).iter().all(|g| g.matched == 2));

    // Pairs by age: 6 and 8 were taken later than 2 and 4.
    let first_row =
        |sort| pairs(sort)[0].rows.iter().map(|r| lib.photo_id(*r).unwrap().ext.parse::<u32>().unwrap()).min().unwrap();
    assert_eq!(first_row(c1index::GroupSort::Newest), 6);
    assert_eq!(first_row(c1index::GroupSort::Oldest), 2);
    assert_eq!(first_row(c1index::GroupSort::Sharpest), 6, "9000 beats 60");

    // Another line moves the stack: asking for a 1-bit difference leaves 2 and 4 apart.
    let strict = c1analysis::ClassSettings { duplicate_bits: 0, ..lib.class_settings() };
    lib.set_class_settings(&strict).unwrap();
    assert_eq!(ids(&lib, &stacked), [2, 3, 4, 6, 7, 8, 10]);
}

#[test]
fn the_colour_mode_reclasses_by_patch_or_by_cover_without_measuring_again() {
    let (_d, lib, _source) = setup();
    let key = |n: u32| lib.photo_id(row_of(&lib, n)).unwrap().key();
    let red = c1analysis::HUES.iter().position(|h| *h == "red").unwrap();
    let grey = [128, 128, 128];
    // 1: a vivid red coat (3% in one patch) on grey. 2: red specks over 30%, none bigger
    // than a pixel or two.
    let mut coat = look(600, 400, grey, 5000.0, 1);
    coat.chroma = 3.0;
    coat.hue_stats[red] = c1analysis::HueStat { share: 0.03, patch: 0.03, chroma: 60.0 };
    let mut specks = look(600, 400, grey, 5000.0, 2);
    specks.hue_stats[red] = c1analysis::HueStat { share: 0.30, patch: 0.001, chroma: 50.0 };
    lib.put_analyses(vec![(key(1), (1, 1), coat), (key(2), (1, 1), specks)]).unwrap();
    let reds = Filter { hues: vec!["red".into()], ..Filter::default() };
    let bw = Filter { hues: vec!["black & white".into()], ..Filter::default() };
    // Covering a sixth of the picture: the specks are red; the coat is not black and white.
    assert_eq!(ids(&lib, &reds), [2]);
    assert!(ids(&lib, &bw).is_empty());
    let patch = c1analysis::ClassSettings { colour_mode: c1analysis::ColourMode::Patch, ..lib.class_settings() };
    lib.set_class_settings(&patch).unwrap();
    assert_eq!(ids(&lib, &reds), [1]);
    assert_eq!(lib.analysed().len(), 2, "nothing measured again");
    // Best match in patch mode: the coat first.
    assert_eq!(ordered(&lib, &Filter { hues: vec!["red".into()], ..Filter::default() }, Sort::Match), [1]);
}

#[test]
fn class_settings_reclass_the_measured_photos_and_survive_a_reopen() {
    let (d, lib, _source) = setup();
    let key = |n: u32| lib.photo_id(row_of(&lib, n)).unwrap().key();
    lib.put_analyses(vec![
        (key(1), (1, 1), look(600, 400, [40, 80, 210], 5000.0, 1)),
        (key(2), (1, 1), look(400, 600, [40, 80, 210], 500.0, 2)),
        (key(3), (1, 1), look(600, 400, [200, 30, 30], 100.0, 3)),
    ])
    .unwrap();
    let soft = Filter { focus: vec!["soft".into()], ..Filter::default() };
    let blurry = Filter { focus: vec!["blurry".into()], ..Filter::default() };
    assert_eq!(ids(&lib, &soft), [2]);
    assert_eq!(ids(&lib, &blurry), [3]);

    // Asked first, nothing changes.
    let stricter = c1analysis::ClassSettings { soft_below: 6000.0, blurry_below: 400.0, ..lib.class_settings() };
    let preview = lib.preview_classes(&stricter).unwrap();
    assert_eq!(preview.measured, 3);
    assert_eq!((preview.counts["focus|soft"], preview.counts["focus|blurry"]), (2, 1));
    assert!(!preview.counts.contains_key("focus|sharp"));
    assert_eq!(ids(&lib, &soft), [2]);

    lib.set_class_settings(&stricter).unwrap();
    assert_eq!(ids(&lib, &soft), [1, 2]);
    assert_eq!(ids(&lib, &blurry), [3]);
    // Nothing was measured again.
    assert_eq!(lib.analysed().len(), 3);

    // Out-of-order values are brought into range.
    let kept = lib
        .set_class_settings(&c1analysis::ClassSettings { soft_below: 100.0, blurry_below: 900.0, ..stricter.clone() })
        .unwrap();
    assert!(kept.blurry_below <= kept.soft_below);

    // Kept for the library: a reopened one classes by it.
    lib.set_class_settings(&stricter).unwrap();
    drop(lib);
    let lib = Library::open(d.path()).unwrap();
    assert_eq!(lib.class_settings(), stricter);
    assert_eq!(ids(&lib, &soft), [1, 2]);
    lib.set_class_settings(&c1analysis::ClassSettings::default()).unwrap();
    assert_eq!(ids(&lib, &soft), [2]);
}

#[test]
fn groups_split_by_a_filter_count_whole_and_drop_what_is_no_group() {
    let (_d, lib, source) = setup();
    let id = |n: u32| lib.photo_id(row_of(&lib, n)).unwrap();
    let (red, blue) = ([200, 30, 30], [40, 80, 210]);
    let h = 0x0f0f_3c3c_5a5a_a5a5u64;
    let d = 0x3333_cccc_5555_aaaau64;
    // A burst of four from the X-Pro2 (odd photos), two seconds apart, one of them blue; and
    // a duplicate pair from the X100VI (2 and 4, the same instant), one red and one blue.
    lib.put_analyses(vec![
        (id(1).key(), (1, 1), look(600, 400, red, 5000.0, h)),
        (id(3).key(), (1, 1), look(400, 600, red, 5000.0, h ^ 0b11)),
        (id(5).key(), (1, 1), look(600, 400, blue, 5000.0, h ^ 0b1111)),
        (id(7).key(), (1, 1), look(400, 600, red, 5000.0, h ^ 0b11_1111)),
        (id(2).key(), (1, 1), look(600, 400, red, 5000.0, d)),
        (id(4).key(), (1, 1), look(600, 400, blue, 5000.0, d ^ 1)),
    ])
    .unwrap();
    let at = source.photo(&id(1)).unwrap().captured_at.unwrap();
    for (k, n) in [3, 5, 7].into_iter().enumerate() {
        source.edit_externally(&id(n), |p| p.captured_at = Some(at + 2000 * (k as i64 + 1)));
    }
    let two = source.photo(&id(2)).unwrap().captured_at;
    source.edit_externally(&id(4), |p| p.captured_at = two);
    lib.sync(&source).unwrap();
    assert_eq!(lib.regroup().unwrap(), 0, "the sync already grouped them");
    let size = |g: &c1index::Group| (g.size, g.matched, g.rows.len());

    // Unfiltered: the whole burst and the whole pair.
    let series = lib.groups(&Filter::default(), true, 0, 10).unwrap().groups;
    assert_eq!(series.iter().map(size).collect::<Vec<_>>(), [(4, 4, 4)]);
    let pairs = lib.groups(&Filter::default(), false, 0, 10).unwrap().groups;
    assert_eq!(pairs.iter().map(size).collect::<Vec<_>>(), [(2, 2, 2)]);

    // Red: three of the burst are still a series, of four; one of the pair is no pair.
    let reds = Filter { hues: vec!["red".into()], ..Filter::default() };
    let series = lib.groups(&reds, true, 0, 10).unwrap().groups;
    assert_eq!(series.iter().map(size).collect::<Vec<_>>(), [(4, 3, 3)]);
    assert!(lib.groups(&reds, false, 0, 10).unwrap().groups.is_empty());
    // The size is the one the details panel gives: every photo of the group.
    let whole = Filter { group: Some(series[0].id.clone()), ..Filter::default() };
    assert_eq!(ids(&lib, &whole), [1, 3, 5, 7]);

    // Blue: one of each, neither a group.
    let blues = Filter { hues: vec!["blue".into()], ..Filter::default() };
    assert!(lib.groups(&blues, true, 0, 10).unwrap().groups.is_empty());
    assert!(lib.groups(&blues, false, 0, 10).unwrap().groups.is_empty());
    // Portrait: two of the burst (3 and 7) are fewer than a series needs.
    let portraits = Filter { shape: vec!["portrait".into()], ..Filter::default() };
    assert_eq!(ids(&lib, &Filter { series: true, ..portraits.clone() }), [3, 7]);
    assert!(lib.groups(&portraits, true, 0, 10).unwrap().groups.is_empty());
}

#[test]
fn groups_page_with_an_exact_total() {
    let (_d, lib, source) = setup();
    let id = |n: u32| lib.photo_id(row_of(&lib, n)).unwrap();
    // Three pairs and a three, each taken at one instant and with its own picture.
    let sets: [(&[u32], u64); 4] = [
        (&[1, 2], 0x0f0f_3c3c_5a5a_a5a5),
        (&[3, 4], 0x3333_cccc_5555_aaaa),
        (&[5, 6], 0x00ff_00ff_f0f0_0f0f),
        (&[7, 8, 10], 0x6969_9696_c3c3_3c3c),
    ];
    let mut analyses = Vec::new();
    for (photos, hash) in sets {
        let at = source.photo(&id(photos[0])).unwrap().captured_at;
        for (k, &n) in photos.iter().enumerate() {
            source.edit_externally(&id(n), |p| p.captured_at = at);
            analyses.push((id(n).key(), (1, 1), look(600, 400, [200, 30, 30], 5000.0, hash ^ k as u64)));
        }
    }
    lib.put_analyses(analyses).unwrap();
    lib.sync(&source).unwrap();
    assert_eq!(lib.regroup().unwrap(), 0, "the sync already grouped them");

    let all = lib.groups(&Filter::default(), false, 0, 100).unwrap();
    assert_eq!(all.total, 4);
    assert_eq!(all.groups.iter().map(|g| g.size).collect::<Vec<_>>(), [3, 2, 2, 2]);
    // Pages of two follow on from each other: every group once, in the same order.
    let first = lib.groups(&Filter::default(), false, 0, 2).unwrap();
    let second = lib.groups(&Filter::default(), false, 2, 2).unwrap();
    assert_eq!((first.total, second.total), (4, 4));
    assert_eq!([first.groups, second.groups].concat(), all.groups);
    let unique: std::collections::HashSet<&str> = all.groups.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(unique.len(), 4);
    let last = lib.groups(&Filter::default(), false, 3, 2).unwrap();
    assert_eq!(last.groups, all.groups[3..]);
    // Past the end: nothing, and still the total.
    let past = lib.groups(&Filter::default(), false, 4, 2).unwrap();
    assert_eq!((past.total, past.groups.len()), (4, 0));
    assert_eq!(lib.groups(&Filter::default(), false, 50, 2).unwrap().total, 4);
}

#[test]
fn analyses_outlive_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let source = MemorySource::new("mem", (1..=3).map(photo).collect(), vec![]);
    {
        let lib = Library::open(dir.path()).unwrap();
        lib.sync(&source).unwrap();
        let key = lib.photo_id(row_of(&lib, 1)).unwrap().key();
        lib.put_analyses(vec![(key, (7, 7), look(600, 400, [40, 80, 210], 500.0, 1))]).unwrap();
    }
    let lib = Library::open(dir.path()).unwrap();
    assert_eq!(ids(&lib, &Filter { hues: vec!["blue".into()], ..Filter::default() }), [1]);
    assert_eq!(lib.analysed().values().next(), Some(&(7, 7)));
}

// The change log.

fn op_of(event: &c1index::Event) -> &Op {
    match &event.what {
        What::Change(change) => &change.op,
        other => panic!("not a change: {other:?}"),
    }
}

#[test]
fn the_first_fill_is_not_news() {
    let (_d, lib, _s) = setup();
    assert!(lib.events(10).unwrap().is_empty(), "photos and albums already there are not logged as added");
}

#[test]
fn source_changes_to_every_kept_field_are_logged() {
    let (_d, lib, source) = setup();
    let id = lib.photo_id(row_of(&lib, 7)).unwrap();
    source.edit_externally(&id, |p| {
        p.iptc.description = Some("Rynek nocą".into());
        p.place.city = Some("Breslau".into());
        p.geo = Some(GeoPoint { lat: 51.12, lon: 17.04, alt: None, origin: GeoOrigin::Exif });
    });
    lib.sync(&source).unwrap();
    let events = lib.photo_events(&id).unwrap();
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(|e| e.origin == Origin::Source));
    let ops: Vec<&Op> = events.iter().map(op_of).collect();
    assert!(
        ops.iter()
            .any(|op| matches!(op, Op::SetIptc { value, .. } if value.description.as_deref() == Some("Rynek nocą")))
    );
    assert!(ops.iter().any(|op| matches!(op, Op::SetPlace { value, .. } if value.city.as_deref() == Some("Breslau"))));
    assert!(ops.iter().any(|op| matches!(op, Op::SetGeo { value: Some(g), .. } if g.lat == 51.12)));

    // A position that only changes its origin did not move.
    source.edit_externally(&id, |p| p.geo.as_mut().unwrap().origin = GeoOrigin::Manual);
    lib.sync(&source).unwrap();
    assert_eq!(lib.photo_events(&id).unwrap().len(), 3);
}

#[test]
fn photos_added_and_removed_in_the_source_are_logged() {
    let (_d, lib, source) = setup();
    source.add_externally(photo(11));
    let gone = lib.photo_id(row_of(&lib, 2)).unwrap();
    source.remove_externally(&gone);
    lib.sync(&source).unwrap();
    let mut names: Vec<_> = lib
        .events(10)
        .unwrap()
        .iter()
        .map(|e| match &e.what {
            What::Added { name, .. } => format!("+{name}"),
            What::Removed { name, .. } => format!("-{name}"),
            other => panic!("{other:?}"),
        })
        .collect();
    names.sort();
    assert_eq!(names, ["+DSCF0011.RAF", "-DSCF0002.RAF"]);
    // Still found by the photo once it is gone.
    assert_eq!(lib.photo_events(&gone).unwrap().len(), 1);
}

#[test]
fn albums_changed_in_the_source_are_logged() {
    let (_d, lib, source) = setup();
    source.edit_collections_externally(|all| {
        all[0].name = "street best".into();
        all.push(Collection {
            id: CollectionId("11".into()),
            name: "night".into(),
            kind: CollectionKind::Album,
            parent: None,
            path: "night".into(),
            rule: None,
            system: false,
        });
    });
    lib.sync(&source).unwrap();
    let labels: Vec<String> = lib
        .events(10)
        .unwrap()
        .into_iter()
        .map(|e| match e.what {
            What::Collection { label, .. } => label,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(labels, ["New album “night”", "Rename “Best / for street” to “street best”"]);
}

/// Writes everything queued to the source, as the app does: no sync after.
fn write_all(lib: &Library, source: &MemorySource) {
    let remaps = lib.remaps().unwrap();
    let albums = lib.collection_changes().unwrap();
    let changes: Vec<Change> = lib.pending().unwrap().into_iter().map(|p| p.pending.change).collect();
    let plan = source
        .plan_after(
            &remaps.iter().map(|(_, q)| q.remap.clone()).collect::<Vec<_>>(),
            &albums.iter().map(|(_, q)| q.change.clone()).collect::<Vec<_>>(),
            &changes,
        )
        .unwrap();
    source.apply(&plan).unwrap();
    lib.settle_remaps(&remaps.iter().map(|(s, _)| *s).collect::<Vec<_>>()).unwrap();
    lib.settle_collections(&albums.iter().map(|(s, _)| *s).collect::<Vec<_>>()).unwrap();
    lib.settle(&plan.apply.iter().chain(&plan.no_ops).cloned().collect::<Vec<_>>()).unwrap();
    lib.log_written(&plan.apply, albums.len(), remaps.len()).unwrap();
}

/// The log, newest first, as kind and origin.
fn logged(lib: &Library) -> Vec<(&'static str, Origin)> {
    lib.events(100)
        .unwrap()
        .iter()
        .map(|e| {
            let kind = match &e.what {
                What::Change(_) => "change",
                What::Added { .. } => "added",
                What::Removed { .. } => "removed",
                What::Deleted { .. } => "deleted",
                What::Collection { .. } => "collection",
                What::Remap(_) => "remap",
                What::Written { .. } => "written",
                What::Undone { .. } => "undone",
            };
            (kind, e.origin)
        })
        .collect()
}

fn new_album(id: &CollectionId, name: &str, kind: CollectionKind, parent: Option<&CollectionId>) -> CollectionChange {
    CollectionChange::Create { id: id.clone(), name: name.into(), kind, parent: parent.cloned(), rule: None }
}

#[test]
fn albums_made_here_are_logged_once() {
    let (_d, lib, source) = setup();
    let japan = CollectionId::new_local("a-1");
    lib.change_collection(new_album(&japan, "japan", CollectionKind::Album, None)).unwrap();
    lib.edit(&[(row_of(&lib, 5), add_to(&japan))]).unwrap();
    write_all(&lib, &source);
    lib.sync(&source).unwrap();
    // Not again when the source sends the album back under its own id, nor the photo
    // moving from the app's id to the source's.
    use Origin::App;
    assert_eq!(logged(&lib), [("written", App), ("change", App), ("collection", App)]);
    let Some(What::Written { photos: 1, changes: 1, albums: 1, keywords: 0 }) =
        lib.events(1).unwrap().pop().map(|e| e.what)
    else {
        panic!("{:?}", lib.events(1))
    };
}

#[test]
fn albums_deleted_here_are_logged_once() {
    let (_d, lib, source) = setup();
    lib.change_collection(CollectionChange::Delete { id: CollectionId("10".into()) }).unwrap();
    write_all(&lib, &source);
    lib.sync(&source).unwrap();
    // Its four photos leaving it are part of the delete, not news from the source.
    assert_eq!(logged(&lib), [("written", Origin::App), ("collection", Origin::App)]);
}

#[test]
fn albums_written_are_known_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let source = MemorySource::new("mem", (1..=3).map(photo).collect(), vec![]);
    let japan = CollectionId::new_local("a-1");
    {
        let lib = Library::open(dir.path()).unwrap();
        lib.sync(&source).unwrap();
        lib.change_collection(new_album(&japan, "japan", CollectionKind::Album, None)).unwrap();
        lib.edit(&[(row_of(&lib, 1), add_to(&japan))]).unwrap();
        // Written, and the app gone before the sync after it.
        write_all(&lib, &source);
    }
    let lib = Library::open(dir.path()).unwrap();
    lib.sync(&source).unwrap();
    use Origin::App;
    assert_eq!(logged(&lib), [("written", App), ("change", App), ("collection", App)]);
}

#[test]
fn albums_of_one_name_made_here_are_told_apart() {
    let (_d, lib, source) = setup();
    let (rome, oslo) = (CollectionId::new_local("g-1"), CollectionId::new_local("g-2"));
    let (in_rome, in_oslo) = (CollectionId::new_local("a-1"), CollectionId::new_local("a-2"));
    lib.change_collection(new_album(&rome, "rome", CollectionKind::Group, None)).unwrap();
    lib.change_collection(new_album(&oslo, "oslo", CollectionKind::Group, None)).unwrap();
    lib.change_collection(new_album(&in_rome, "selects", CollectionKind::Album, Some(&rome))).unwrap();
    lib.change_collection(new_album(&in_oslo, "selects", CollectionKind::Album, Some(&oslo))).unwrap();
    lib.edit(&[(row_of(&lib, 5), add_to(&in_rome)), (row_of(&lib, 6), add_to(&in_oslo))]).unwrap();
    write_all(&lib, &source);
    let before = lib.events(100).unwrap().len();
    lib.sync(&source).unwrap();
    assert_eq!(lib.events(100).unwrap().len(), before, "{:?}", logged(&lib));
    // And each photo is in the album under its own group.
    let albums = lib.collections().unwrap();
    let path_of = |n| {
        let photo = lib.photo(row_of(&lib, n)).unwrap().unwrap();
        albums.iter().find(|c| photo.collections.contains(&c.id)).unwrap().path.clone()
    };
    assert_eq!((path_of(5), path_of(6)), ("rome / selects".to_owned(), "oslo / selects".to_owned()));
}

#[test]
fn a_restored_backup_is_logged_as_one() {
    let (_d, lib, source) = setup();
    let id = lib.photo_id(row_of(&lib, 7)).unwrap();
    source.edit_externally(&id, |p| p.rating = Some(1));
    lib.sync_as(&source, Origin::Restore).unwrap();
    assert_eq!(logged(&lib), [("change", Origin::Restore)]);
}

#[test]
fn a_touched_source_with_nothing_new_is_no_change() {
    let (_d, lib, source) = setup();
    source.edit_externally(&lib.photo_id(row_of(&lib, 1)).unwrap(), |_| {});
    let report = lib.sync(&source).unwrap();
    assert!(report.full && !report.changed(), "{report:?}");
    source.edit_collections_externally(|all| all[0].name = "street best".into());
    assert!(lib.sync(&source).unwrap().changed());
}

#[test]
fn captions_and_places_written_come_back_as_written() {
    let (_d, lib, source) = setup();
    let row = row_of(&lib, 7);
    let iptc = Iptc { headline: Some("Rynek".into()), ..Iptc::default() };
    let place = Place { city: Some("Breslau".into()), country: Some("Poland".into()), ..Place::default() };
    lib.edit(&[(row, Op::SetIptc { base: Iptc::default(), value: iptc })]).unwrap();
    lib.edit(&[(row, Op::SetPlace { base: Place::default(), value: place })]).unwrap();
    write_all(&lib, &source);
    lib.sync(&source).unwrap();
    use Origin::App;
    assert_eq!(logged(&lib), [("written", App), ("change", App), ("change", App)]);
}

#[test]
fn gpx_placements_carry_their_origin() {
    let (_d, lib, _s) = setup();
    let row = row_of(&lib, 1);
    let value = Some(GeoPoint { lat: 50.0, lon: 19.9, alt: None, origin: GeoOrigin::Gpx });
    lib.edit_as(&[(row, Op::SetGeo { base: None, value })], Origin::Gpx).unwrap();
    lib.edit(&[(row, rate(Some(5)))]).unwrap();
    let events = lib.photo_events(&lib.photo_id(row).unwrap()).unwrap();
    assert_eq!(events.iter().map(|e| e.origin).collect::<Vec<_>>(), [Origin::App, Origin::Gpx]);
}

#[test]
fn photo_history_outlives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let source = MemorySource::new("mem", (1..=3).map(photo).collect(), vec![]);
    let id = {
        let lib = Library::open(dir.path()).unwrap();
        lib.sync(&source).unwrap();
        let row = row_of(&lib, 1);
        lib.edit(&[(row, rate(Some(4)))]).unwrap();
        lib.photo_id(row).unwrap()
    };
    let lib = Library::open(dir.path()).unwrap();
    assert_eq!(lib.photo_events(&id).unwrap().len(), 1);
}

/// When photo `n` of [`trip_photos`] was taken: three a day around midday UTC (the same
/// local day wherever the tests run), from 2026-01-01.
fn trip_time(n: u32, hour: i64) -> i64 {
    1_767_225_600_000 + i64::from((n - 1) / 3) * 86_400_000 + hour * 3_600_000
}

/// Photos 1..=30 over ten days: 1-9 in Kraków, 10-21 in Tokyo (one of them rated 5),
/// 22-30 in Kraków again.
fn trip_photos() -> Vec<Photo> {
    (1..=30)
        .map(|n| {
            let mut p = photo(n);
            p.captured_at = Some(trip_time(n, 10 + 2 * i64::from((n - 1) % 3)));
            p.trashed = false;
            p.rating = (n == 15).then_some(5);
            p.place = match n {
                10..=21 => Place { country: Some("Japan".into()), city: Some("Tokyo".into()), ..Place::default() },
                _ => Place { country: Some("Poland".into()), city: Some("Kraków".into()), ..Place::default() },
            };
            p
        })
        .collect()
}

fn set_home(lib: &Library) {
    let home = c1index::trips::Home { country: Some("Poland".into()), city: Some("Kraków".into()) };
    lib.set_trip_settings(&c1index::trips::TripSettings { home: Some(home), ..Default::default() }).unwrap();
}

#[test]
fn trips_are_found_counted_and_filter_the_grid() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    lib.sync(&MemorySource::new("mem", trip_photos(), vec![])).unwrap();
    let list = lib.trips(&Filter::default()).unwrap();
    assert!(list.trips.is_empty(), "no home, no trips");
    assert_eq!(list.home_guess.and_then(|h| h.city).as_deref(), Some("Kraków"));

    set_home(&lib);
    let list = lib.trips(&Filter::default()).unwrap();
    assert_eq!(list.trips.len(), 1);
    let tokyo = &list.trips[0];
    assert_eq!((tokyo.label.as_str(), tokyo.photos, tokyo.count), ("Tokyo", 12, 12));
    assert_eq!(tokyo.cover, Some(row_of(&lib, 15)), "the best rated photo stands for it");

    // Counted over the filter, and the trip's own span set aside.
    let span = TimeRange { from: Some(tokyo.trip.start), to: Some(tokyo.trip.end) };
    let rated = Filter { rating: Some(parse_rating(">=1").unwrap()), trip: Some(span), ..Filter::default() };
    assert_eq!(lib.trips(&rated).unwrap().trips[0].count, 1);
    let in_trip = Filter { trip: Some(span), ..Filter::default() };
    assert_eq!(lib.query(&in_trip, &QueryOptions::default()).unwrap().total, 12);

    // Days of the timeline add up to the photos.
    let days = lib.timeline(&Filter::default()).unwrap();
    assert_eq!(days.iter().map(|(_, n)| n).sum::<usize>(), 30);
    assert!(days.windows(2).all(|w| w[0].0 < w[1].0));
}

#[test]
fn trip_edits_outlive_reopening_and_new_photos() {
    let dir = tempfile::tempdir().unwrap();
    let source = MemorySource::new("mem", trip_photos(), vec![]);
    let id = {
        let lib = Library::open(dir.path()).unwrap();
        lib.sync(&source).unwrap();
        set_home(&lib);
        let id = lib.trips(&Filter::default()).unwrap().trips[0].trip.id;
        let edit = c1index::trips::TripEdit::Rename { id, name: Some("Japan 2026".into()) };
        assert_eq!(lib.edit_trips(edit).unwrap(), Some(id));
        id
    };
    // A photo from Tokyo after the last one: the kept trip stretches over it.
    let mut late = photo(31);
    late.place = Place { country: Some("Japan".into()), city: Some("Tokyo".into()), ..Place::default() };
    late.captured_at = Some(trip_time(21, 15));
    late.trashed = false;
    source.add_externally(late);
    let lib = Library::open(dir.path()).unwrap();
    lib.sync(&source).unwrap();
    let list = lib.trips(&Filter::default()).unwrap();
    assert_eq!(list.trips.len(), 1);
    let t = &list.trips[0];
    assert_eq!((t.trip.id, t.trip.name.as_deref(), t.photos), (id, Some("Japan 2026"), 13));
}

#[test]
fn offline_photos_are_told_apart() {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(dir.path()).unwrap();
    let photos = (1..=4)
        .map(|n| {
            let mut p = photo(n);
            p.file.offline = n.is_multiple_of(2);
            p
        })
        .collect();
    library.sync(&MemorySource::new("mem", photos, vec![])).unwrap();
    assert_eq!(ids(&library, &Filter { offline: Some(true), ..Filter::default() }), [2, 4]);
    assert_eq!(ids(&library, &Filter { offline: Some(false), ..Filter::default() }), [1, 3]);
    assert_eq!(ids(&library, &Filter::default()), [1, 2, 3, 4]);
    assert!(library.photo(0).unwrap().is_some());
}

#[test]
fn a_selection_counts_and_merges_what_its_photos_have() {
    let (_d, lib, _source) = setup();
    let rows = |ns: &[u32]| ns.iter().map(|&n| row_of(&lib, n)).collect::<Vec<_>>();
    // 3: ★3, red, street and best|best2018, in the album; 6: ★1, street and best|best2018;
    // 7: ★2, street, a description, Wrocław and a position.
    let s = lib.selection(&rows(&[3, 6, 7])).unwrap();
    assert_eq!(s.keywords, [("best|best2018".to_owned(), 2), ("street".to_owned(), 3)]);
    assert_eq!(s.albums, [("10".to_owned(), 1)]);
    let places: Vec<_> = s.places.iter().map(|(p, rows)| (p.city.as_deref(), rows.len())).collect();
    assert_eq!(places, [(None, 2), (Some("Wrocław"), 1)]);
    fn shape<T: Clone>(m: &Merged<T>) -> (Option<T>, usize, usize) {
        (m.same.clone(), m.values, m.empty)
    }
    assert_eq!(shape(&s.rating), (None, 3, 0));
    assert_eq!((s.color.values, s.color.empty), (1, 2));
    assert_eq!(s.color.top, [(ColorLabel::Red, 1)]);
    assert_eq!((s.geo.values, s.geo.empty), (1, 2));
    assert_eq!(s.geo.top, [([51.11, 17.03], 1)]);
    // Each value with how many have it, for the field's tooltip.
    assert_eq!(s.iptc["description"].top, [("Rynek we Wrocławiu".to_owned(), 1)]);
    assert_eq!(shape(&s.iptc["description"]), (None, 1, 2));
    assert_eq!(shape(&s.iptc["headline"]), (None, 0, 3));
    // What all of them share is given as such: 1 and 6 are both ★1.
    let s = lib.selection(&rows(&[1, 6])).unwrap();
    assert_eq!((s.rating.same, s.rating.values, s.rating.empty), (Some(1), 1, 0));
    assert_eq!(s.rating.top, []);
    assert_eq!(s.keywords, [("best|best2018".to_owned(), 1), ("street".to_owned(), 2)]);
    // Rows that don't exist are left out.
    assert_eq!(lib.selection(&[9_999]).unwrap().places, []);
}

#[test]
fn hidden_photos_leave_every_view_until_shown_and_the_mark_lasts() {
    let (d, lib, source) = setup();
    let all = ids(&lib, &Filter::default());
    assert_eq!(all, [1, 2, 3, 4, 5, 6, 7, 8, 10]);
    let changed = lib.set_hidden(&[row_of(&lib, 2), row_of(&lib, 5)], true).unwrap();
    assert_eq!(changed.len(), 2);
    // Hiding them again changes nothing (so an undo knows what to show again).
    assert!(lib.set_hidden(&[row_of(&lib, 2)], true).unwrap().is_empty());

    // Gone from the grid and from the counts beside it.
    assert_eq!(ids(&lib, &Filter::default()), [1, 3, 4, 6, 7, 8, 10]);
    let counted = |f: &Filter| {
        let options = QueryOptions { limit: Some(0), facets: vec![Dim::Year], ..QueryOptions::default() };
        lib.query(f, &options).unwrap().facets[&Dim::Year].iter().map(|(_, n)| *n).sum::<u64>()
    };
    assert_eq!(counted(&Filter::default()), 7);
    // Shown on request, or alone.
    assert_eq!(ids(&lib, &Filter { hidden: Hidden::Include, ..Filter::default() }), all);
    assert_eq!(ids(&lib, &Filter { hidden: Hidden::Only, ..Filter::default() }), [2, 5]);
    assert_eq!(lib.hidden_rows(&Filter::default()).unwrap().len(), 2);
    // Alternatives (Match: any) are under the outer filter's say: shown, a hidden photo
    // matching one of them comes back; asked for alone, it is found.
    let either = |hidden| Filter {
        any_of: vec![
            Filter { ids: vec!["2".into()], ..Filter::default() },
            Filter { ids: vec!["3".into()], ..Filter::default() },
        ],
        hidden,
        ..Filter::default()
    };
    assert_eq!(ids(&lib, &either(Hidden::Exclude)), [3]);
    assert_eq!(ids(&lib, &either(Hidden::Include)), [2, 3]);
    assert_eq!(ids(&lib, &either(Hidden::Only)), [2]);
    assert_eq!(lib.hidden_of(&[row_of(&lib, 2), row_of(&lib, 3)]), [row_of(&lib, 2)]);
    assert!(lib.is_hidden(row_of(&lib, 5)) && !lib.is_hidden(row_of(&lib, 1)));
    // Nothing is queued for the source: the mark is C1 Studio's own.
    assert!(lib.pending().unwrap().is_empty());

    // An edit or a sync rewrites the photo's document: it stays hidden.
    lib.edit(&[(row_of(&lib, 2), Op::SetRating { base: None, value: Some(4) })]).unwrap();
    source.edit_externally(&lib.photo_id(row_of(&lib, 5)).unwrap(), |p| p.rating = Some(1));
    lib.sync(&source).unwrap();
    assert_eq!(ids(&lib, &Filter { hidden: Hidden::Only, ..Filter::default() }), [2, 5]);

    // A rebuilt index and a reopened library keep it.
    lib.reindex().unwrap();
    assert_eq!(ids(&lib, &Filter::default()), [1, 3, 4, 6, 7, 8, 10]);
    drop(lib);
    let lib = Library::open(d.path()).unwrap();
    assert_eq!(ids(&lib, &Filter { hidden: Hidden::Only, ..Filter::default() }), [2, 5]);

    // Shown again (an undo).
    assert_eq!(lib.set_hidden(&[row_of(&lib, 2), row_of(&lib, 5)], false).unwrap().len(), 2);
    assert_eq!(ids(&lib, &Filter::default()), all);
    assert!(lib.hidden_rows(&Filter::default()).unwrap().is_empty());
}

#[test]
fn hidden_photos_are_left_out_of_duplicate_groups_shown_but_still_measured() {
    let (_d, lib, source) = setup();
    let id = |n: u32| lib.photo_id(row_of(&lib, n)).unwrap();
    let h = 0x0f0f_3c3c_5a5a_a5a5u64;
    lib.put_analyses(vec![
        (id(1).key(), (1, 1), look(600, 400, [40, 80, 210], 5000.0, h)),
        (id(2).key(), (1, 1), look(600, 400, [40, 80, 210], 5000.0, h ^ 1)),
    ])
    .unwrap();
    let one = source.photo(&id(1)).unwrap().captured_at;
    source.edit_externally(&id(2), |p| p.captured_at = one);
    lib.sync(&source).unwrap();
    lib.regroup().unwrap();
    assert_eq!(lib.groups(&Filter::default(), false, 0, 10).unwrap().groups.len(), 1);
    lib.set_hidden(&[row_of(&lib, 2)], true).unwrap();
    // One of the pair is hidden: no pair is left to show.
    assert!(lib.groups(&Filter::default(), false, 0, 10).unwrap().groups.is_empty());
    let shown = Filter { hidden: Hidden::Include, ..Filter::default() };
    assert_eq!(lib.groups(&shown, false, 0, 10).unwrap().groups.len(), 1);
    // The analysis still knows it.
    assert!(lib.look(row_of(&lib, 2)).is_some());
}

#[test]
fn photos_in_the_trash_leave_every_view_but_its_own_and_the_mark_lasts() {
    let (d, lib, source) = setup();
    let all = ids(&lib, &Filter::default());
    let trash = Filter { bin: Bin::Only, ..Filter::default() };
    let changed = lib.set_binned(&[row_of(&lib, 2), row_of(&lib, 5)], true, false).unwrap();
    assert_eq!(changed.len(), 2);
    assert!(lib.set_binned(&[row_of(&lib, 2)], true, false).unwrap().is_empty(), "in the trash already");

    assert_eq!(ids(&lib, &Filter::default()), [1, 3, 4, 6, 7, 8, 10]);
    let options = QueryOptions { limit: Some(0), facets: vec![Dim::Year], ..QueryOptions::default() };
    let counted: u64 = lib.query(&Filter::default(), &options).unwrap().facets[&Dim::Year].iter().map(|(_, n)| n).sum();
    assert_eq!(counted, 7, "out of the counts too");
    assert_eq!(ids(&lib, &trash), [2, 5]);
    assert_eq!(lib.bin_count(), 2);
    assert!(lib.is_binned(row_of(&lib, 2)) && !lib.is_binned(row_of(&lib, 1)));
    // Match: any inside the trash finds what is in it.
    let either = Filter {
        any_of: vec![
            Filter { ids: vec!["2".into()], ..Filter::default() },
            Filter { ids: vec!["3".into()], ..Filter::default() },
        ],
        bin: Bin::Only,
        ..Filter::default()
    };
    assert_eq!(ids(&lib, &either), [2]);
    // Nothing reaches the source until the trash is emptied.
    assert!(lib.pending().unwrap().is_empty());

    // A sync, a rebuilt index and a reopened library keep it.
    source.edit_externally(&lib.photo_id(row_of(&lib, 5)).unwrap(), |p| p.rating = Some(1));
    lib.sync(&source).unwrap();
    lib.reindex().unwrap();
    drop(lib);
    let lib = Library::open(d.path()).unwrap();
    assert_eq!(ids(&lib, &trash), [2, 5]);

    // Restored (an undo).
    assert_eq!(lib.set_binned(&[row_of(&lib, 2), row_of(&lib, 5)], false, false).unwrap().len(), 2);
    assert_eq!(ids(&lib, &Filter::default()), all);
    assert_eq!(lib.bin_count(), 0);
}

#[test]
fn emptying_the_trash_deletes_its_photos_and_their_queued_edits() {
    let (_d, lib, source) = setup();
    lib.edit(&[(row_of(&lib, 2), Op::SetRating { base: Some(2), value: Some(5) })]).unwrap();
    lib.set_binned(&[row_of(&lib, 2), row_of(&lib, 3)], true, false).unwrap();
    let going: Vec<PhotoId> = lib.bin_photos().unwrap().into_iter().map(|(_, id)| id).collect();
    assert_eq!(going.len(), 2);

    let report = source.remove(&going).unwrap();
    lib.emptied(&report.removed).unwrap();
    assert_eq!(lib.bin_count(), 0);
    assert!(lib.pending().unwrap().is_empty(), "nothing left to write the rating to");
    lib.sync(&source).unwrap();
    assert!(ids(&lib, &Filter { bin: Bin::Include, ..Filter::default() }).iter().all(|n| ![2, 3].contains(n)));
    let log = lib.events(10).unwrap();
    assert!(log.iter().any(|e| matches!(&e.what, What::Deleted { photo, .. } if photo.ext == "2")), "{log:?}");
}

#[test]
fn the_source_deleting_or_trashing_a_photo_takes_it_out_of_the_trash() {
    let (_d, lib, source) = setup();
    lib.set_binned(&[row_of(&lib, 2), row_of(&lib, 3)], true, false).unwrap();
    source.remove_externally(&lib.photo_id(row_of(&lib, 2)).unwrap());
    source.edit_externally(&lib.photo_id(row_of(&lib, 3)).unwrap(), |p| p.trashed = true);
    lib.sync(&source).unwrap();
    assert_eq!(lib.bin_count(), 0);
    assert!(ids(&lib, &Filter { bin: Bin::Only, ..Filter::default() }).is_empty());
}

#[test]
fn a_source_that_trashes_whole_assets_takes_every_variant_to_the_trash() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let variant =
        |n: u32, asset: &str, index: u32| Photo { asset: AssetId(asset.into()), variant_index: index, ..photo(n) };
    let source = MemorySource::new("mem", vec![variant(1, "a", 0), variant(2, "a", 1), variant(3, "b", 0)], vec![]);
    source.set_removal(Removal::MarksTrashed);
    lib.sync(&source).unwrap();
    let changed = lib.set_binned(&[row_of(&lib, 2)], true, true).unwrap();
    assert_eq!(changed.len(), 2);
    assert_eq!(ids(&lib, &Filter { bin: Bin::Only, ..Filter::default() }), [1, 2]);

    let going: Vec<PhotoId> = lib.bin_photos().unwrap().into_iter().map(|(_, id)| id).collect();
    lib.emptied(&source.remove(&going).unwrap().removed).unwrap();
    lib.sync(&source).unwrap();
    assert_eq!(ids(&lib, &Filter::default()), [3]);
    assert_eq!(ids(&lib, &Filter { trashed: Trashed::Only, ..Filter::default() }), [1, 2], "in the source's trash");
    assert!(ids(&lib, &Filter { bin: Bin::Only, ..Filter::default() }).is_empty());
}

/// A moment later than every entry so far: log times are the clock's, and the tests cut
/// the log into dates by it.
fn tick() -> i64 {
    std::thread::sleep(std::time::Duration::from_millis(3));
    let now = chrono::Utc::now().timestamp_millis();
    std::thread::sleep(std::time::Duration::from_millis(3));
    now
}

fn everything() -> Filter {
    Filter { trashed: Trashed::Include, hidden: Hidden::Include, bin: Bin::Include, ..Filter::default() }
}

fn history(pick: HistoryPick) -> Filter {
    Filter { history: Some(pick), ..everything() }
}

#[test]
fn the_log_reads_in_pages_grouped_by_batch() {
    let (_d, lib, source) = setup();
    // One batch: three photos rated together by the app. Another, by Capture One.
    let rows: Vec<u64> = (5..=7).map(|n| row_of(&lib, n)).collect();
    lib.edit(&rows.iter().map(|r| (*r, rate(Some(4)))).collect::<Vec<_>>()).unwrap();
    for n in [1, 2] {
        source.edit_externally(&lib.photo_id(row_of(&lib, n)).unwrap(), |p| p.rating = Some(5));
    }
    lib.sync(&source).unwrap();
    lib.edit(&[(row_of(&lib, 8), Op::SetColor { base: None, value: Some(ColorLabel::Blue) })]).unwrap();

    let page = lib.log_page(&LogQuery::default()).unwrap();
    let shape: Vec<(LogKind, Origin, usize, usize)> =
        page.groups.iter().map(|g| (g.kind, g.origin, g.entries, g.photos)).collect();
    use Origin::{App, Source};
    assert_eq!(shape, [(LogKind::Color, App, 1, 1), (LogKind::Rating, Source, 2, 2), (LogKind::Rating, App, 3, 3)]);
    assert!(page.groups[0].row.is_some() && page.groups[1].row.is_none(), "only a group of one names its photo");
    assert_eq!(page.next, None);

    // Paged: a group is not cut between pages, and the next page starts after it.
    let first = lib.log_page(&LogQuery { limit: 1, ..LogQuery::default() }).unwrap();
    assert_eq!(first.groups.len(), 1);
    let second = lib.log_page(&LogQuery { limit: 1, before: first.next, ..LogQuery::default() }).unwrap();
    assert_eq!((second.groups[0].kind, second.groups[0].entries), (LogKind::Rating, 2));
    let last = lib.log_page(&LogQuery { limit: 1, before: second.next, ..LogQuery::default() }).unwrap();
    assert_eq!((last.groups[0].entries, last.next), (3, None));

    let only_source = LogQuery { origins: vec![Source], ..LogQuery::default() };
    assert_eq!(lib.log_page(&only_source).unwrap().groups.len(), 1);
    let only_colour = LogQuery { kinds: vec![LogKind::Color], ..LogQuery::default() };
    assert_eq!(lib.log_page(&only_colour).unwrap().groups.len(), 1);
}

#[test]
fn a_log_entry_picks_its_photos() {
    let (_d, lib, _s) = setup();
    let rows: Vec<u64> = (5..=7).map(|n| row_of(&lib, n)).collect();
    lib.edit(&rows.iter().map(|r| (*r, rate(Some(4)))).collect::<Vec<_>>()).unwrap();
    lib.edit(&[(row_of(&lib, 8), rate(Some(1)))]).unwrap();
    let group = lib.log_page(&LogQuery::default()).unwrap().groups.pop().unwrap();
    assert_eq!(group.entries, 3);
    let pick = HistoryPick::Entries { first: group.first, last: group.last, kind: Some(LogKind::Rating), origin: None };
    assert_eq!(ids(&lib, &history(pick)), [5, 6, 7]);
    // Another kind in the same entries picks nothing.
    let pick = HistoryPick::Entries { first: group.first, last: group.last, kind: Some(LogKind::Color), origin: None };
    assert!(ids(&lib, &history(pick)).is_empty());
}

#[test]
fn compare_counts_net_changes_between_two_dates() {
    let (_d, lib, source) = setup();
    let before = tick();
    let (five, six) = (row_of(&lib, 5), row_of(&lib, 6));
    // 5 is rated and the rating put back; 6 is rated and keeps it.
    lib.edit(&[(five, rate(Some(3))), (six, rate(Some(3)))]).unwrap();
    lib.edit(&[(five, rate(None))]).unwrap();
    lib.edit(&[(six, Op::AddKeywords { paths: vec![KeywordPath::from_pipe("night")] })]).unwrap();
    source.add_externally(photo(11));
    source.remove_externally(&lib.photo_id(row_of(&lib, 10)).unwrap());
    lib.sync(&source).unwrap();
    let after = tick();
    lib.edit(&[(row_of(&lib, 1), rate(Some(5)))]).unwrap();

    let c = lib.compare(before, Some(after)).unwrap();
    assert_eq!((c.added, c.removed, c.deleted), (1, 1, 0));
    assert_eq!(c.fields, [(LogKind::Rating, 1), (LogKind::Keywords, 1)]);
    assert_eq!(c.keywords_added, [("night".to_owned(), 1)]);
    assert!(c.keywords_removed.is_empty());
    assert_eq!(c.by_origin.get("App"), Some(&4));
    assert!(c.log_start.is_some());
    // Open ended, the later rating counts too.
    assert_eq!(lib.compare(before, None).unwrap().fields, [(LogKind::Rating, 2), (LogKind::Keywords, 1)]);
    // Before anything, nothing.
    assert_eq!(lib.compare(0, Some(1)).unwrap().entries, 0);
}

#[test]
fn changed_between_dates_filters_the_grid() {
    let (_d, lib, _s) = setup();
    let (five, six) = (row_of(&lib, 5), row_of(&lib, 6));
    lib.edit(&[(five, rate(Some(3)))]).unwrap();
    let from = tick();
    lib.edit(&[(six, rate(Some(3)))]).unwrap();
    lib.edit(&[(six, Op::AddKeywords { paths: vec![KeywordPath::from_pipe("night")] })]).unwrap();
    let to = tick();
    lib.edit(&[(row_of(&lib, 7), rate(Some(3)))]).unwrap();
    let changed = |what| history(HistoryPick::Changed { from, to: Some(to), what });
    assert_eq!(ids(&lib, &changed(NetWhat::Any)), [6]);
    assert_eq!(ids(&lib, &changed(NetWhat::Field(LogKind::Rating))), [6]);
    assert_eq!(ids(&lib, &changed(NetWhat::KeywordAdded("night".into()))), [6]);
    assert!(ids(&lib, &changed(NetWhat::KeywordRemoved("night".into()))).is_empty());
    // It narrows like any other filter.
    let both = Filter { ext: vec!["raf".into()], ..changed(NetWhat::Any) };
    assert_eq!(ids(&lib, &both), [6]);
    // Open ended, and as a nested filter.
    let open =
        Filter { all_of: vec![history(HistoryPick::Changed { from, to: None, what: NetWhat::Any })], ..everything() };
    assert_eq!(ids(&lib, &open), [6, 7]);
    // A pick follows the log: another edit is in it at once.
    lib.edit(&[(row_of(&lib, 8), rate(Some(4)))]).unwrap();
    assert_eq!(ids(&lib, &open), [6, 7, 8]);
}

#[test]
fn growth_counts_photos_by_import_day() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let imported = |n: u32, day: i64| Photo {
        imported_at: Some(1_767_225_600_000 + day * 86_400_000 + 12 * 3_600_000),
        ..photo(n)
    };
    let mut photos = vec![imported(1, 0), imported(2, 0), imported(3, 3), photo(4)];
    photos[2].rating = Some(5);
    lib.sync(&MemorySource::new("mem", photos, vec![])).unwrap();
    let days = lib.growth(&Filter::default()).unwrap();
    assert_eq!(days.iter().map(|(_, n)| *n).collect::<Vec<_>>(), [2, 1], "{days:?}");
    assert_eq!(days[1].0 - days[0].0, 3 * 86_400_000);
    let five = Range { min: Some(5.0), ..Range::default() };
    let rated = Filter { rating: Some(Presence::Values(five)), ..Filter::default() };
    assert_eq!(lib.growth(&rated).unwrap().len(), 1);
}

#[test]
fn photos_removed_or_deleted_keep_what_they_were() {
    let (_d, lib, source) = setup();
    // Removed by the source.
    let gone = lib.photo_id(row_of(&lib, 2)).unwrap();
    let was = lib.photo(row_of(&lib, 2)).unwrap().unwrap();
    source.remove_externally(&gone);
    lib.sync(&source).unwrap();
    let events = lib.photo_events(&gone).unwrap();
    assert!(matches!(&events[0].what, What::Removed { .. }));
    assert_eq!(events[0].before, Some(Before::Photo(Box::new(was))));

    // Deleted for good from the trash.
    let id = lib.photo_id(row_of(&lib, 3)).unwrap();
    let kept = lib.photo(row_of(&lib, 3)).unwrap().unwrap();
    lib.emptied(std::slice::from_ref(&id)).unwrap();
    let events = lib.photo_events(&id).unwrap();
    assert!(matches!(&events[0].what, What::Deleted { .. }));
    assert_eq!(events[0].before, Some(Before::Photo(Box::new(kept))));

    // An ordinary change keeps nothing.
    lib.edit(&[(row_of(&lib, 1), rate(Some(5)))]).unwrap();
    assert_eq!(lib.events(1).unwrap()[0].before, None);
}

#[test]
fn album_changes_keep_what_they_overwrote() {
    let (_d, lib, source) = setup();
    let street = CollectionId("10".into());
    let album = lib.collections().unwrap().into_iter().find(|c| c.id == street).unwrap();
    lib.change_collection(CollectionChange::Rename { id: street.clone(), name: "streets".into() }).unwrap();
    let renamed = lib.events(1).unwrap().pop().unwrap();
    assert_eq!(renamed.before, Some(Before::Albums { albums: vec![album.clone()], members: vec![] }));

    // A delete keeps who was in it, the trashed ones too.
    lib.set_binned(&[row_of(&lib, 1)], true, false).unwrap();
    lib.change_collection(CollectionChange::Delete { id: street.clone() }).unwrap();
    let deleted = lib.events(1).unwrap().pop().unwrap();
    let Some(Before::Albums { albums, members }) = &deleted.before else { panic!("{deleted:?}") };
    assert_eq!(albums.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["streets"]);
    assert_eq!((members[0].0.clone(), members[0].1.len()), (street.clone(), 4));
    // Each of them has it in their own history.
    let one = lib.photo_id(row_of(&lib, 2)).unwrap();
    assert!(matches!(&lib.photo_events(&one).unwrap()[0].what, What::Collection { .. }));

    // The source's own changes keep the album too.
    let made = CollectionId::new_local("a-1");
    lib.change_collection(new_album(&made, "japan", CollectionKind::Album, None)).unwrap();
    write_all(&lib, &source);
    lib.sync(&source).unwrap();
    source.edit_collections_externally(|all| all.retain(|c| c.name != "japan"));
    lib.sync(&source).unwrap();
    let gone = lib.events(1).unwrap().pop().unwrap();
    let Some(Before::Albums { albums, .. }) = &gone.before else { panic!("{gone:?}") };
    assert_eq!(albums[0].name, "japan");
}

#[test]
fn keyword_tree_changes_keep_the_photos_they_rewrote() {
    let (_d, lib, _s) = setup();
    lib.remap_keywords(KeywordRemap::rename(KeywordPath::from_pipe("street"), "avenue")).unwrap();
    let event = lib.events(1).unwrap().pop().unwrap();
    let Some(Before::Keywords(photos)) = &event.before else { panic!("{event:?}") };
    // Photos with n % 3 of 0 or 1 carry it.
    let mut exts: Vec<u32> = photos.iter().map(|(id, _)| id.ext.parse().unwrap()).collect();
    exts.sort();
    assert_eq!(exts, [1, 3, 4, 6, 7, 9, 10]);
    let three = photos.iter().find(|(id, _)| id.ext == "3").unwrap();
    assert_eq!(three.1, [KeywordPath::from_pipe("street"), KeywordPath::from_pipe("best|best2018")]);
    // The photo's own history shows the rename now.
    let id = lib.photo_id(row_of(&lib, 4)).unwrap();
    assert!(matches!(&lib.photo_events(&id).unwrap()[0].what, What::Remap(_)));
    // And picking the entry picks the photos.
    let seq = lib.log_page(&LogQuery::default()).unwrap().groups[0].first;
    let pick = HistoryPick::Entries { first: seq, last: seq, kind: None, origin: None };
    assert_eq!(ids(&lib, &history(pick)), [1, 3, 4, 6, 7, 9, 10]);
}

#[test]
fn log_days_count_entries_and_grow_with_the_log() {
    let (_d, lib, _s) = setup();
    assert!(lib.log_days().unwrap().days.is_empty());
    lib.edit(&[(row_of(&lib, 1), rate(Some(5)))]).unwrap();
    lib.edit(&[(row_of(&lib, 2), rate(Some(5)))]).unwrap();
    let days = lib.log_days().unwrap();
    assert!(days.start.is_some());
    assert_eq!(days.days.iter().map(|(_, n)| *n).sum::<usize>(), 2);
    lib.edit(&[(row_of(&lib, 3), rate(Some(5)))]).unwrap();
    assert_eq!(lib.log_days().unwrap().days.iter().map(|(_, n)| *n).sum::<usize>(), 3);
}

#[test]
fn a_batch_picked_from_a_narrowed_list_leaves_out_other_origins() {
    let (_d, lib, source) = setup();
    lib.edit(&[(row_of(&lib, 5), rate(Some(4)))]).unwrap();
    source.edit_externally(&lib.photo_id(row_of(&lib, 6)).unwrap(), |p| p.rating = Some(4));
    lib.sync(&source).unwrap();
    lib.edit(&[(row_of(&lib, 7), rate(Some(4)))]).unwrap();
    // With only the app's entries listed, photos 5 and 7 are one batch's neighbours in time,
    // and Capture One's photo 6 lies between them.
    let only_app = LogQuery { origins: vec![Origin::App], kinds: vec![LogKind::Rating], ..LogQuery::default() };
    let groups = lib.log_page(&only_app).unwrap().groups;
    let pick = |g: &c1index::LogGroup| HistoryPick::Entries {
        first: g.first,
        last: g.last,
        kind: Some(g.kind),
        origin: Some(g.origin),
    };
    assert_eq!(groups.len(), 2);
    assert_eq!(ids(&lib, &history(pick(&groups[0]))), [7]);
    let wide =
        HistoryPick::Entries { first: groups[1].first, last: groups[0].last, kind: None, origin: Some(Origin::App) };
    assert_eq!(ids(&lib, &history(wide)), [5, 7]);
}

#[test]
fn a_keyword_tree_change_counts_in_compare_and_picks() {
    let (_d, lib, _s) = setup();
    let from = tick();
    lib.remap_keywords(KeywordRemap::rename(KeywordPath::from_pipe("street"), "avenue")).unwrap();
    let c = lib.compare(from, None).unwrap();
    assert_eq!(c.fields, [(LogKind::Keywords, 7)]);
    assert_eq!(c.keywords_added, [("avenue".to_owned(), 7)]);
    assert_eq!(c.keywords_removed, [("street".to_owned(), 7)]);
    let pick = HistoryPick::Changed { from, to: None, what: NetWhat::KeywordAdded("avenue".into()) };
    assert_eq!(ids(&lib, &history(pick)), [1, 3, 4, 6, 7, 9, 10]);
}

#[test]
fn photos_removed_since_do_not_count_as_changed() {
    let (_d, lib, source) = setup();
    let from = tick();
    lib.edit(&[(row_of(&lib, 5), rate(Some(4))), (row_of(&lib, 6), rate(Some(4)))]).unwrap();
    source.remove_externally(&lib.photo_id(row_of(&lib, 6)).unwrap());
    lib.sync(&source).unwrap();
    let c = lib.compare(from, None).unwrap();
    assert_eq!((c.changed, c.removed), (1, 1), "{c:?}");
    assert_eq!(c.fields, [(LogKind::Rating, 1)]);
    assert_eq!(ids(&lib, &history(HistoryPick::Changed { from, to: None, what: NetWhat::Any })), [5]);
}

#[test]
fn a_pick_made_into_rows_is_forgotten_when_a_read_adds_photos_without_logging_them() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let source = MemorySource::new("mem", (1..=3).map(photo).collect(), vec![]);
    lib.sync(&source).unwrap();
    let from = tick();
    lib.edit(&[(row_of(&lib, 1), rate(Some(4)))]).unwrap();
    let pick = history(HistoryPick::Entries { first: 0, last: 0, kind: None, origin: None });
    assert_eq!(ids(&lib, &pick), [1]);
    // Another source's first read logs nothing, and its rows are new: nothing stale is kept.
    let other = MemorySource::new("other", vec![], vec![]);
    lib.sync(&other).unwrap();
    assert_eq!(ids(&lib, &pick), [1]);
    assert_eq!(ids(&lib, &history(HistoryPick::Changed { from, to: None, what: NetWhat::Any })), [1]);
}

#[test]
fn with_history_off_the_log_keeps_nothing_and_history_reads_nothing() {
    let (_d, lib, source) = setup();
    assert!(lib.history_on(), "on unless turned off");
    lib.edit(&[(row_of(&lib, 5), rate(Some(4)))]).unwrap();
    let pick = HistoryPick::Changed { from: 0, to: None, what: NetWhat::Any };
    assert_eq!(ids(&lib, &history(pick.clone())), [5]);

    lib.set_history(false).unwrap();
    assert!(!lib.history_on());
    // The change log as 0.5 kept it goes on: every entry, without what it replaced.
    let gone = lib.photo_id(row_of(&lib, 2)).unwrap();
    source.remove_externally(&gone);
    lib.sync(&source).unwrap();
    let removed = lib.photo_events(&gone).unwrap();
    assert!(matches!(&removed[0].what, What::Removed { .. }));
    assert!(!removed[0].has_before && removed[0].before.is_none());
    lib.remap_keywords(KeywordRemap::rename(KeywordPath::from_pipe("street"), "avenue")).unwrap();
    let remapped = lib.events(1).unwrap().pop().unwrap();
    assert!(matches!(remapped.what, What::Remap(_)) && !remapped.has_before);
    lib.change_collection(CollectionChange::Delete { id: CollectionId("10".into()) }).unwrap();
    let deleted = lib.events(1).unwrap().pop().unwrap();
    assert!(matches!(deleted.what, What::Collection { .. }) && !deleted.has_before);
    lib.edit(&[(row_of(&lib, 8), rate(Some(1)))]).unwrap();
    let logged = lib.events(100).unwrap();
    assert!(matches!(&logged[0].what, What::Change(c) if c.photo.ext == "8"), "{:?}", logged[0]);
    assert!(logged.iter().all(|e| !e.has_before));

    // Nothing reads the log as history, and a pick picks nothing out.
    assert!(lib.log_page(&LogQuery::default()).is_err());
    assert!(lib.compare(0, None).is_err());
    assert!(lib.log_days().is_err());
    assert!(lib.growth(&Filter::default()).is_err());
    assert_eq!(ids(&lib, &history(pick.clone())), ids(&lib, &everything()));

    // Back on, the log is all there: what was logged while it was off reads too.
    lib.set_history(true).unwrap();
    let picked = ids(&lib, &history(pick));
    assert!(picked.contains(&5) && picked.contains(&8) && !picked.contains(&2), "{picked:?}");
    let days: usize = lib.log_days().unwrap().days.iter().map(|(_, n)| *n).sum();
    assert_eq!(days, logged.len());
    assert!(lib.log_page(&LogQuery::default()).unwrap().groups.len() >= 5);
}

#[test]
fn history_off_is_the_librarys_own_so_a_plain_open_keeps_nothing_either() {
    let (dir, lib, source) = setup();
    lib.set_history(false).unwrap();
    let logged_before = lib.events(1_000).unwrap().len();
    drop(lib);

    // `c1cat` opens `<data root>/<key>/library` with a plain `Library::open`, as here: it
    // is told nothing, and still finds History off.
    let lib = Library::open(dir.path()).unwrap();
    assert!(!lib.history_on());
    let gone = lib.photo_id(row_of(&lib, 2)).unwrap();
    source.remove_externally(&gone);
    lib.sync(&source).unwrap();
    lib.remap_keywords(KeywordRemap::rename(KeywordPath::from_pipe("street"), "avenue")).unwrap();
    lib.change_collection(CollectionChange::Delete { id: CollectionId("10".into()) }).unwrap();
    let logged = lib.events(1_000).unwrap();
    let off = logged.len() - logged_before;
    assert!(off >= 3, "{logged:?}");
    assert!(logged.iter().all(|e| !e.has_before && e.before.is_none()), "{logged:?}");
    drop(lib);

    // Turned back on (by the app): the stretch logged while off is known, and said.
    let lib = Library::open(dir.path()).unwrap();
    assert!(!lib.history_on());
    lib.set_history(true).unwrap();
    lib.edit(&[(row_of(&lib, 5), rate(Some(4)))]).unwrap();
    assert!(lib.events(1).unwrap()[0].what.photo().is_some());
    let spans = lib.history_off().unwrap();
    assert_eq!(spans.len(), 1, "{spans:?}");
    assert_eq!(spans[0].entries, off as u64);
    assert!(spans[0].to.is_some_and(|to| to >= spans[0].from));
    let compare = lib.compare(0, None).unwrap();
    assert_eq!((compare.entries, compare.logged_off), (logged.len() + 1, off as u64));
    drop(lib);
    let lib = Library::open(dir.path()).unwrap();
    assert!(lib.history_on());
    assert_eq!(lib.history_off().unwrap(), spans);
}

#[test]
fn a_read_that_finds_every_photo_gone_keeps_only_so_many_whole() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let photos: Vec<Photo> = (1..=5_010).map(photo).collect();
    let source = MemorySource::new("mem", photos.clone(), vec![]);
    lib.sync(&source).unwrap();
    // A source that comes back empty (a drive unplugged): every photo is gone.
    for p in &photos {
        source.remove_externally(&p.id);
    }
    lib.sync(&source).unwrap();
    let logged = lib.events(6_000).unwrap();
    let removed = logged.iter().filter(|e| matches!(e.what, What::Removed { .. })).count();
    let whole = logged.iter().filter(|e| e.has_before).count();
    assert_eq!((removed, whole), (5_010, 5_000));
}

#[test]
fn turning_history_off_holds_for_every_program_that_opens_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let source = MemorySource::new("mem", (1..=3).map(photo).collect(), vec![]);
    {
        let lib = Library::open(dir.path()).unwrap();
        lib.sync(&source).unwrap();
        lib.set_history(false).unwrap();
    }
    // The command line tool opens it the plain way, and keeps nothing either.
    let lib = Library::open(dir.path()).unwrap();
    assert!(!lib.history_on());
    source.remove_externally(&lib.photo_id(row_of(&lib, 2)).unwrap());
    lib.sync(&source).unwrap();
    assert_eq!(lib.events(1).unwrap()[0].before, None);
    lib.set_history(true).unwrap();
    drop(lib);
    assert!(Library::open(dir.path()).unwrap().history_on());
}

#[test]
fn compare_says_when_a_keyword_tree_change_kept_no_photos() {
    let (_d, lib, _s) = setup();
    let from = tick();
    lib.set_history(false).unwrap();
    lib.remap_keywords(KeywordRemap::rename(KeywordPath::from_pipe("street"), "avenue")).unwrap();
    lib.set_history(true).unwrap();
    lib.remap_keywords(KeywordRemap::rename(KeywordPath::from_pipe("best"), "top")).unwrap();
    let c = lib.compare(from, None).unwrap();
    assert_eq!(c.keywords_unkept, 1);
    // The one that kept its photos counts; the other is not in the numbers.
    assert_eq!(c.keywords_added, [("top|best2018".to_owned(), 3)]);
}

#[test]
fn accepted_suggestions_are_queued_as_the_model_s_edits() {
    use c1index::ai::{Entry, Status};
    let (_d, lib, _s) = setup();
    let row = row_of(&lib, 7);
    let suggestion = c1ai::Suggestion { headline: Some("Market square".into()), ..c1ai::Suggestion::default() };
    let entry = Entry {
        at: 1,
        provider: "claude".into(),
        model: "claude-haiku-5-5".into(),
        prompt: 7,
        suggestion,
        warnings: vec![],
        cost: 0.001,
        status: Status::Proposed,
    };
    lib.put_suggestion(row, &entry, &c1ai::RunRecord::default()).unwrap();
    assert_eq!(lib.described(&[row, row_of(&lib, 8)], "claude-haiku-5-5", 7).unwrap().len(), 1);
    assert!(lib.described(&[row], "claude-opus-5-5", 7).unwrap().is_empty());
    assert_eq!(lib.vocabulary().unwrap()[0].0, "street");

    let before = lib.photo(row).unwrap().unwrap();
    let pick = c1index::Accept {
        row,
        keywords: vec!["street".into(), "market".into()],
        headline: Some("Market square".into()),
        rating: Some(5),
        seen: c1index::Seen {
            caption: before.iptc.description.clone(),
            headline: before.iptc.headline.clone(),
            rating: before.rating,
        },
        ..c1index::Accept::default()
    };
    let accepted = lib.accept_suggestions(&[pick]).unwrap();
    assert!(accepted.changed.is_empty());
    let undo = accepted.undo;
    let photo = lib.photo(row).unwrap().unwrap();
    assert_eq!(photo.rating, Some(5));
    assert_eq!(photo.iptc.headline.as_deref(), Some("Market square"));
    assert_eq!(photo.iptc.description.as_deref(), Some("Rynek we Wrocławiu"), "the caption is left as it was");
    assert!(photo.keywords.contains(&KeywordPath::from_pipe("street")));
    assert!(photo.keywords.contains(&KeywordPath::from_pipe("market")));
    assert!(!undo.is_empty());
    assert!(lib.events(10).unwrap().iter().any(|e| e.origin == Origin::Ai));
    assert_eq!(lib.suggestion(row).unwrap().unwrap().status, Status::Accepted);
    assert!(lib.suggestions(Some(Status::Proposed)).unwrap().is_empty());
}

/// A caption written after the review showed the photo is not replaced by accepting: the
/// field is left, the rest is taken, and the suggestion stays to review again.
#[test]
fn accepting_leaves_a_field_that_changed_since_it_was_shown() {
    use c1index::ai::{Entry, Status};
    let (_d, lib, _s) = setup();
    let row = row_of(&lib, 7);
    let entry = Entry {
        at: 1,
        provider: "claude".into(),
        model: "claude-haiku-5-5".into(),
        prompt: 7,
        suggestion: c1ai::Suggestion::default(),
        warnings: vec![],
        cost: 0.0,
        status: Status::Proposed,
    };
    lib.put_suggestion(row, &entry, &c1ai::RunRecord::default()).unwrap();
    let before = lib.photo(row).unwrap().unwrap();
    assert_eq!(before.iptc.description.as_deref(), Some("Rynek we Wrocławiu"));
    // The card was shown when the photo had no caption, and no rating.
    let pick = c1index::Accept {
        row,
        keywords: vec!["market".into()],
        caption: Some("A market square at dusk.".into()),
        rating: Some(4),
        seen: c1index::Seen { headline: before.iptc.headline.clone(), ..c1index::Seen::default() },
        ..c1index::Accept::default()
    };
    let rated = before.rating.is_some();
    let accepted = lib.accept_suggestions(&[pick]).unwrap();
    assert_eq!(accepted.changed, vec![row]);
    let photo = lib.photo(row).unwrap().unwrap();
    assert_eq!(photo.iptc.description.as_deref(), Some("Rynek we Wrocławiu"), "the caption written since is kept");
    if rated {
        assert_eq!(photo.rating, before.rating, "the rating set since is kept");
    }
    assert!(photo.keywords.contains(&KeywordPath::from_pipe("market")), "the rest is taken");
    assert_eq!(lib.suggestion(row).unwrap().unwrap().status, Status::Proposed);

    // Shown as it is now, it is taken.
    let pick = c1index::Accept {
        row,
        caption: Some("A market square at dusk.".into()),
        seen: c1index::Seen {
            caption: photo.iptc.description.clone(),
            headline: photo.iptc.headline.clone(),
            rating: photo.rating,
        },
        ..c1index::Accept::default()
    };
    assert!(lib.accept_suggestions(&[pick]).unwrap().changed.is_empty());
    let photo = lib.photo(row).unwrap().unwrap();
    assert_eq!(photo.iptc.description.as_deref(), Some("A market square at dusk."));
    assert_eq!(lib.suggestion(row).unwrap().unwrap().status, Status::Accepted);
}
