//! Photos without a position on the map: by their place's name, from their neighbours in
//! time, and places filled in from a position.

use c1index::{BBox, Filter, Library, MapView, NeighbourSettings, PlaceLookup, QueryOptions, Sort};
use c1model::memory::MemorySource;
use c1model::*;

const T0: i64 = 1_767_225_600_000; // 2026-01-01 00:00 UTC
const MIN: i64 = 60_000;

fn place(country: &str, state: &str, city: &str) -> Place {
    let some = |s: &str| (!s.is_empty()).then(|| s.to_owned());
    Place { country: some(country), state: some(state), city: some(city), ..Place::default() }
}

fn photo(n: u32, minutes: i64, place: Place, geo: Option<(f64, f64)>) -> Photo {
    let source = SourceId("mem".into());
    Photo {
        id: PhotoId::new(&source, n.to_string()),
        asset: AssetId(n.to_string()),
        variant_index: 0,
        file: FileInfo { name: format!("P{n:04}.RAF"), ext: "raf".into(), path: None, size: None },
        captured_at: Some(T0 + minutes * MIN),
        imported_at: None,
        exif: Exif::default(),
        rating: None,
        color: None,
        keywords: vec![],
        iptc: Iptc::default(),
        place,
        geo: geo.map(|(lat, lon)| GeoPoint { lat, lon, alt: None, origin: GeoOrigin::Exif }),
        collections: vec![],
        edit: EditInfo::default(),
        trashed: false,
    }
}

fn krakow() -> Place {
    Place { country_code: Some("PL".into()), ..place("Poland", "Malopolska", "Kraków") }
}

/// 1 and 3 have a position, 2 is between them in time; 4 is in Kraków by name, 5 in
/// Poland, 6 nowhere known; 7 and 8 have nothing, 8 is taken just after 4; 9 has a
/// position and no place.
fn setup() -> (tempfile::TempDir, Library) {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(dir.path()).unwrap();
    let photos = vec![
        photo(1, 0, Place::default(), Some((50.06, 19.93))),
        photo(2, 10, Place::default(), None),
        photo(3, 20, Place::default(), Some((50.08, 19.95))),
        photo(4, 1000, krakow(), None),
        photo(5, 3000, place("Poland", "", ""), None),
        photo(6, 5000, place("Atlantis", "", "Xyzzyville"), None),
        photo(7, 7000, Place::default(), None),
        photo(8, 1005, Place::default(), None),
        photo(9, 9000, Place::default(), Some((50.0614, 19.9366))),
    ];
    library.sync(&MemorySource::new("mem", photos, vec![])).unwrap();
    (dir, library)
}

fn row(library: &Library, n: u32) -> u64 {
    let all = library.query(&Filter::default(), &QueryOptions::default()).unwrap();
    all.rows.into_iter().find(|r| library.photo_id(*r).unwrap().ext == n.to_string()).unwrap()
}

fn numbers(library: &Library, filter: &Filter) -> Vec<u32> {
    let rows = library.query(filter, &QueryOptions { sort: Sort::Row, ..QueryOptions::default() }).unwrap().rows;
    let mut out: Vec<u32> = rows.iter().map(|r| library.photo_id(*r).unwrap().ext.parse().unwrap()).collect();
    out.sort();
    out
}

fn view(zoom: f64) -> MapView {
    MapView { bounds: BBox { min_lat: -85.0, max_lat: 85.0, min_lon: -180.0, max_lon: 180.0 }, zoom }
}

#[test]
fn photos_with_a_place_or_without() {
    let (_d, lib) = setup();
    assert_eq!(numbers(&lib, &Filter { placed: Some(true), ..Filter::default() }), [4, 5, 6]);
    assert_eq!(numbers(&lib, &Filter { placed: Some(false), gps: Some(false), ..Filter::default() }), [2, 7, 8]);
}

#[test]
fn place_names_show_on_the_map_while_zoomed_out_enough() {
    let (_d, lib) = setup();
    let far = lib.map(&Filter::default(), &view(3.0)).unwrap();
    assert_eq!((far.located, far.approx_located, far.unresolved, far.unplaced), (3, 2, 1, 3));
    assert_eq!(far.approx.iter().map(|c| c.count).sum::<usize>(), 2);
    // Kraków and Poland: one marker for the whole world, at the coarser level.
    let world = lib.map(&Filter::default(), &view(0.0)).unwrap();
    assert_eq!(world.approx.len(), 1);
    assert_eq!(
        (world.approx[0].count, world.approx[0].level, world.approx[0].place.as_deref()),
        (2, c1geo::Level::Country, None)
    );
    // The country is gone at zoom 9, the city at zoom 13.
    let region = lib.map(&Filter::default(), &view(9.0)).unwrap();
    assert_eq!(region.approx.len(), 1);
    assert_eq!(region.approx[0].place.as_deref(), Some("Poland|Malopolska|Kraków"));
    assert_eq!(region.approx_hidden, 1);
    let street = lib.map(&Filter::default(), &view(13.0)).unwrap();
    assert!(street.approx.is_empty());
    assert_eq!(street.approx_hidden, 2);
}

#[test]
fn places_the_gazetteer_lacks_are_looked_up_once() {
    let (_d, lib) = setup();
    let unknown = lib.unresolved_places(&Filter::default()).unwrap();
    assert_eq!(unknown.iter().map(|u| u.path.as_str()).collect::<Vec<_>>(), ["Atlantis|—|Xyzzyville"]);
    let found = c1geo::Approx { lat: 36.4, lon: 25.4, level: c1geo::Level::City };
    lib.set_place_points(&[PlaceLookup { path: unknown[0].path.clone(), found: Some(found) }]).unwrap();
    assert!(lib.unresolved_places(&Filter::default()).unwrap().is_empty());
    let map = lib.map(&Filter::default(), &view(9.0)).unwrap();
    assert_eq!((map.unresolved, map.approx_located), (0, 3));
}

#[test]
fn a_place_given_its_position_still_shows_as_approximate() {
    let (_d, lib) = setup();
    let undo = lib.promote(&Filter::default(), &["Poland|Malopolska|Kraków".to_owned()]).unwrap();
    assert_eq!(undo.len(), 1);
    let four = lib.photo(row(&lib, 4)).unwrap().unwrap();
    assert!(four.geo.is_some_and(|g| g.origin == GeoOrigin::Geocoded));
    let map = lib.map(&Filter::default(), &view(9.0)).unwrap();
    assert_eq!((map.located, map.approx_located), (3, 2), "counted with the places, not the positions");
    // Moved by hand, it is placed for real.
    lib.edit(&[(
        row(&lib, 4),
        Op::SetGeo { base: None, value: Some(GeoPoint { lat: 50.0, lon: 19.9, alt: None, origin: GeoOrigin::Manual }) },
    )])
    .unwrap();
    assert_eq!(lib.map(&Filter::default(), &view(9.0)).unwrap().located, 4);
}

#[test]
fn places_are_filled_from_positions_as_the_catalog_writes_them() {
    let (_d, lib) = setup();
    let plan = lib.fill_plan(&[row(&lib, 9), row(&lib, 1), row(&lib, 4)]).unwrap();
    assert_eq!(plan.len(), 1, "photo 4 has no position");
    assert_eq!(plan[0].rows.len(), 2);
    let p = &plan[0].place;
    assert_eq!(
        (p.country.as_deref(), p.country_code.as_deref(), p.state.as_deref(), p.city.as_deref()),
        (Some("Poland"), Some("PL"), Some("Malopolska"), Some("Kraków"))
    );
    lib.fill_places(&[row(&lib, 9)]).unwrap();
    assert_eq!(lib.photo(row(&lib, 9)).unwrap().unwrap().place.city.as_deref(), Some("Kraków"));
    // A photo that says it is elsewhere is left alone.
    lib.edit(&[(row(&lib, 1), Op::SetPlace { base: Place::default(), value: place("Germany", "", "") })]).unwrap();
    assert!(lib.fill_plan(&[row(&lib, 1)]).unwrap().is_empty());
}

#[test]
fn neighbours_in_time_give_a_position_or_a_place() {
    let (_d, lib) = setup();
    let settings = NeighbourSettings::default();
    let plan = lib.neighbour_plan(&Filter::default(), &settings).unwrap();
    assert_eq!((plan.candidates, plan.positions, plan.places), (6, 1, 1));
    assert_eq!(plan.sample[0].0, row(&lib, 2));
    assert!((plan.sample[0].1 - 50.07).abs() < 1e-9, "halfway between 1 and 3");
    assert_eq!(plan.place_counts, [("Poland|Malopolska|Kraków".to_owned(), 1)]);
    lib.neighbour_apply(&Filter::default(), &settings).unwrap();
    assert!(lib.photo(row(&lib, 2)).unwrap().unwrap().geo.is_some());
    assert_eq!(lib.photo(row(&lib, 8)).unwrap().unwrap().place, krakow());
    let only_close = NeighbourSettings { max_gap_minutes: 1, ..settings };
    assert_eq!(lib.neighbour_plan(&Filter::default(), &only_close).unwrap().positions, 0);
    // A huge gap is two days, not an overflow.
    let huge = NeighbourSettings { max_gap_minutes: i64::MAX, ..settings };
    assert!(lib.neighbour_plan(&Filter::default(), &huge).is_ok());
}

#[test]
fn guesses_from_neighbours_place_no_more_photos() {
    let (_d, lib) = setup();
    // Photo 2 gets a position between 1 and 3; it must not then place photo 7 by itself.
    lib.neighbour_apply(&Filter::default(), &NeighbourSettings { places: false, ..NeighbourSettings::default() })
        .unwrap();
    lib.edit(&[
        (row(&lib, 1), Op::SetGeo { base: None, value: None }),
        (row(&lib, 3), Op::SetGeo { base: None, value: None }),
    ])
    .unwrap();
    let plan = lib.neighbour_plan(&Filter::default(), &NeighbourSettings::default()).unwrap();
    assert!(!plan.sample.iter().any(|(r, ..)| *r == row(&lib, 1) || *r == row(&lib, 3)), "{plan:?}");
}

#[test]
fn photos_without_a_position_of_their_own_include_those_given_their_place() {
    let (_d, lib) = setup();
    let approx = Filter { approx: Some(true), ..Filter::default() };
    assert_eq!(numbers(&lib, &approx), [2, 4, 5, 6, 7, 8]);
    lib.promote(&Filter::default(), &["Poland|Malopolska|Kraków".to_owned()]).unwrap();
    assert_eq!(numbers(&lib, &approx), [2, 4, 5, 6, 7, 8], "4 has a position now, not its own");
    assert_eq!(numbers(&lib, &Filter { approx: Some(false), ..Filter::default() }), [1, 3, 9]);
    // Still flagged after the library is opened again.
    let dir = _d.path().to_owned();
    drop(lib);
    let lib = Library::open(&dir).unwrap();
    assert_eq!(numbers(&lib, &approx), [2, 4, 5, 6, 7, 8]);
}

#[test]
fn lookups_that_are_no_position_are_not_kept() {
    let (_d, lib) = setup();
    let path = "Atlantis|—|Xyzzyville".to_owned();
    let bad = c1geo::Approx { lat: 95.0, lon: 10.0, level: c1geo::Level::City };
    lib.set_place_points(&[PlaceLookup { path: path.clone(), found: Some(bad) }]).unwrap();
    assert_eq!(lib.unresolved_places(&Filter::default()).unwrap().len(), 1, "asked again");
    lib.set_place_points(&[PlaceLookup { path, found: None }]).unwrap();
    assert!(lib.unresolved_places(&Filter::default()).unwrap().is_empty(), "nothing found is kept");
}
