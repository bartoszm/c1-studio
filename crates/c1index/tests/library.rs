use c1index::{Dim, Filter, Library, Presence, QueryOptions, Range, Sort, TimeRange, Trashed, parse_rating};
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
    let all = library.query(&Filter { trashed: Trashed::Include, ..Filter::default() }, &QueryOptions::default());
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
    assert_eq!(events[0].change.op, Op::AddKeywords { paths: vec![KeywordPath::from_pipe("night")] });
    assert_eq!(events[1].origin, c1index::Origin::App);
    assert_eq!(events[1].change.op, Op::SetRating { base: Some(2), value: Some(5) });
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
    let plan = source.plan_after(&remaps.iter().map(|(_, q)| q.remap.clone()).collect::<Vec<_>>(), &changes).unwrap();
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
