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
        file: FileInfo { name: format!("P{n:04}.RAF"), ext: "raf".into(), path: None, size: None, offline: false },
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
fn a_place_given_its_position_shows_under_its_place_as_edited() {
    let (_d, lib) = setup();
    let four = row(&lib, 4);
    lib.promote(&Filter::default(), &["Poland|Malopolska|Kraków".to_owned()]).unwrap();
    let wieliczka = place("Poland", "Malopolska", "Wieliczka");
    lib.edit(&[(four, Op::SetPlace { base: krakow(), value: wieliczka.clone() })]).unwrap();
    let map = lib.map(&Filter::default(), &view(9.0)).unwrap();
    assert_eq!(
        map.approx.iter().map(|c| c.place.as_deref()).collect::<Vec<_>>(),
        [Some("Poland|Malopolska|Wieliczka")]
    );
    // Its place cleared, it is a position like any other, as the tray shows it.
    lib.edit(&[(four, Op::SetPlace { base: wieliczka, value: Place::default() })]).unwrap();
    let map = lib.map(&Filter::default(), &view(9.0)).unwrap();
    assert_eq!((map.located, map.approx_located, map.place_only), (4, 1, 2));
    assert_eq!(numbers(&lib, &Filter { placed: Some(true), approx: Some(true), ..Filter::default() }), [5, 6]);
}

#[test]
fn a_photo_moved_since_it_was_given_its_place_loses_its_mark_when_opened_again() {
    let (d, lib) = setup();
    let (four, five) = (row(&lib, 4), row(&lib, 5));
    let undo = lib.promote(&Filter::default(), &["Poland|Malopolska|Kraków".to_owned(), "Poland".to_owned()]).unwrap();
    let given = |r| lib.photo(r).unwrap().unwrap().geo.unwrap();
    let (at_four, at_five) = (given(four), given(five));
    let by_hand =
        |lat, lon| Op::SetGeo { base: None, value: Some(GeoPoint { lat, lon, alt: None, origin: GeoOrigin::Manual }) };
    // 4 moved by hand; 5 undone, so it has no position.
    lib.edit(&[(four, by_hand(50.0, 19.9))]).unwrap();
    lib.edit(&undo.into_iter().filter(|(r, _)| *r == five).collect::<Vec<_>>()).unwrap();
    drop(lib);
    let lib = Library::open(d.path()).unwrap();
    // Both put back: 4 by hand, a position of its own now; 5 redone, its place's again.
    lib.edit(&[(four, by_hand(at_four.lat, at_four.lon)), (five, Op::SetGeo { base: None, value: Some(at_five) })])
        .unwrap();
    assert_eq!(numbers(&lib, &Filter { gps: Some(true), approx: Some(true), ..Filter::default() }), [5]);
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
    assert_eq!((plan.unpositioned, plan.positions, plan.unlocated, plan.places), (6, 1, 9, 1));
    assert_eq!(plan.sample[0].0, row(&lib, 2));
    assert!((plan.sample[0].1 - 50.07).abs() < 1e-9, "halfway between 1 and 3");
    assert_eq!(plan.place_counts, [(String::new(), "Poland|Malopolska|Kraków".to_owned(), 1)]);
    lib.neighbour_apply(&Filter::default(), &settings).unwrap();
    assert!(lib.photo(row(&lib, 2)).unwrap().unwrap().geo.is_some());
    assert_eq!(lib.photo(row(&lib, 8)).unwrap().unwrap().place, krakow());
    let only_close = NeighbourSettings { position_gap_minutes: 1, ..settings };
    assert_eq!(lib.neighbour_plan(&Filter::default(), &only_close).unwrap().positions, 0);
    // A huge gap is two days, not an overflow.
    let huge = NeighbourSettings { position_gap_minutes: i64::MAX, place_gap_minutes: i64::MAX, ..settings };
    assert!(lib.neighbour_plan(&Filter::default(), &huge).is_ok());
}

#[test]
fn guesses_from_neighbours_place_no_more_photos() {
    let (_d, lib) = setup();
    // Photo 2 gets a position between 1 and 3; theirs gone, it must not give them one.
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

fn by_hand(lat: f64, lon: f64) -> Op {
    Op::SetGeo { base: None, value: Some(GeoPoint { lat, lon, alt: None, origin: GeoOrigin::Manual }) }
}

#[test]
fn neighbours_refine_a_place_to_a_more_specific_one() {
    let (_d, lib) = setup();
    let wawel = Place { location: Some("Wawel".into()), ..krakow() };
    lib.edit(&[(row(&lib, 8), Op::SetPlace { base: Place::default(), value: wawel.clone() })]).unwrap();
    let settings = NeighbourSettings::default();
    let plan = lib.neighbour_plan(&Filter::default(), &settings).unwrap();
    assert_eq!((plan.unlocated, plan.places), (8, 1));
    assert_eq!(
        plan.place_counts,
        [("Poland|Malopolska|Kraków".to_owned(), "Poland|Malopolska|Kraków|Wawel".to_owned(), 1)]
    );
    let only_close = NeighbourSettings { place_gap_minutes: 4, ..settings };
    assert_eq!(lib.neighbour_plan(&Filter::default(), &only_close).unwrap().places, 0);
    lib.neighbour_apply(&Filter::default(), &settings).unwrap();
    assert_eq!(lib.photo(row(&lib, 4)).unwrap().unwrap().place, wawel, "country code and all");
    assert_eq!(lib.neighbour_plan(&Filter::default(), &settings).unwrap().places, 0, "nothing more to say");
}

#[test]
fn neighbours_saying_different_things_refine_no_place() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let at = |location: &str| Place { location: Some(location.into()), ..krakow() };
    let photos = vec![photo(1, 0, at("Wawel"), None), photo(2, 5, krakow(), None), photo(3, 10, at("Barbican"), None)];
    lib.sync(&MemorySource::new("mem", photos, vec![])).unwrap();
    assert_eq!(lib.neighbour_plan(&Filter::default(), &NeighbourSettings::default()).unwrap().places, 0);
    lib.edit(&[(row(&lib, 3), Op::SetPlace { base: at("Barbican"), value: at("Wawel") })]).unwrap();
    assert_eq!(lib.neighbour_plan(&Filter::default(), &NeighbourSettings::default()).unwrap().places, 1, "they agree");
}

#[test]
fn a_place_is_refined_past_neighbours_saying_the_same_but_not_past_one_elsewhere() {
    // A city's photos, one of them at a named spot: the others need not be next to it.
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let wawel = Place { location: Some("Wawel".into()), ..krakow() };
    let photos = vec![
        photo(1, 0, wawel.clone(), None),
        photo(2, 3, krakow(), None),
        photo(3, 6, krakow(), None),
        photo(4, 9, place("Poland", "", "Kraków"), None),
    ];
    lib.sync(&MemorySource::new("mem", photos, vec![])).unwrap();
    let plan = lib.neighbour_plan(&Filter::default(), &NeighbourSettings::default()).unwrap();
    assert_eq!(plan.places, 3, "{plan:?}");
    // Somewhere else in between: 2 has Wawel before it but Warsaw after, 4 has Warsaw
    // before it, and Warsaw's 3 has Kraków either side. Nothing agrees.
    let warsaw = place("Poland", "Mazowieckie", "Warszawa");
    lib.edit(&[(row(&lib, 3), Op::SetPlace { base: krakow(), value: warsaw })]).unwrap();
    let plan = lib.neighbour_plan(&Filter::default(), &NeighbourSettings::default()).unwrap();
    assert_eq!(plan.places, 0, "{plan:?}");
    // Further away than the gap, it says nothing.
    let close = NeighbourSettings { place_gap_minutes: 3, ..NeighbourSettings::default() };
    lib.edit(&[(row(&lib, 4), Op::SetPlace { base: place("Poland", "", "Kraków"), value: krakow() })]).unwrap();
    let plan = lib.neighbour_plan(&Filter::default(), &close).unwrap();
    assert_eq!(plan.places, 0, "2 still has Warsaw 3 minutes after it: {plan:?}");
    lib.edit(&[(row(&lib, 3), Op::SetPlace { base: place("Poland", "Mazowieckie", "Warszawa"), value: krakow() })])
        .unwrap();
    assert_eq!(
        lib.neighbour_plan(&Filter::default(), &close).unwrap().places,
        1,
        "only 2 is within 3 minutes of Wawel"
    );
}

#[test]
fn neighbours_place_no_photo_when_the_filter_wants_those_with_a_location() {
    let (_d, lib) = setup();
    let plan = lib.neighbour_plan(&Filter { located: Some(true), ..Filter::default() }, &NeighbourSettings::default());
    let plan = plan.unwrap();
    assert_eq!((plan.unlocated, plan.places), (0, 0));
}

#[test]
fn neighbours_outside_the_filter_still_give_their_place() {
    let (_d, lib) = setup();
    // 8 is taken just after 4, in Kraków; the filter leaves 4 out.
    let plan = lib.neighbour_plan(&Filter { placed: Some(false), ..Filter::default() }, &NeighbourSettings::default());
    assert_eq!(plan.unwrap().place_counts, [(String::new(), "Poland|Malopolska|Kraków".to_owned(), 1)]);
}

#[test]
fn a_photo_given_a_position_and_a_place_is_undone_whole() {
    let (_d, lib) = setup();
    let eight = row(&lib, 8);
    lib.edit(&[(row(&lib, 4), by_hand(50.054, 19.9354))]).unwrap();
    let undo = lib.neighbour_apply(&Filter::default(), &NeighbourSettings::default()).unwrap();
    let given = lib.photo(eight).unwrap().unwrap();
    assert_eq!((given.geo.map(|g| (g.lat, g.lon)), given.place), (Some((50.054, 19.9354)), krakow()));
    lib.edit(&undo).unwrap();
    let back = lib.photo(eight).unwrap().unwrap();
    assert_eq!((back.geo, back.place), (None, Place::default()));
}

#[test]
fn neighbours_move_an_approximate_position_to_their_gps() {
    let (_d, lib) = setup();
    let (four, eight) = (row(&lib, 4), row(&lib, 8));
    lib.promote(&Filter::default(), &["Poland|Malopolska|Kraków".to_owned()]).unwrap();
    lib.edit(&[(eight, by_hand(50.054, 19.9354))]).unwrap();
    let positions = NeighbourSettings { places: false, ..NeighbourSettings::default() };
    let given = |s: &NeighbourSettings, r: u64| {
        lib.neighbour_plan(&Filter::default(), s).unwrap().sample.iter().any(|(at, ..)| *at == r)
    };
    let tight = NeighbourSettings { position_gap_minutes: 8, ..positions };
    assert!(!given(&tight, four), "a single neighbour five minutes apart, more than half of 8");
    assert!(given(&positions, four));
    lib.neighbour_apply(&Filter::default(), &positions).unwrap();
    let geo = lib.photo(four).unwrap().unwrap().geo.unwrap();
    assert_eq!((geo.lat, geo.lon, geo.origin), (50.054, 19.9354, GeoOrigin::Manual));
    assert!(numbers(&lib, &Filter { gps: Some(true), approx: Some(true), ..Filter::default() }).is_empty());
    // A guess: with 8's position gone, 4 does not give it one back.
    lib.edit(&[(eight, Op::SetGeo { base: None, value: None })]).unwrap();
    assert!(!given(&positions, eight));
}

#[test]
fn a_position_outside_the_photos_place_is_not_given() {
    let (_d, lib) = setup();
    let four = row(&lib, 4);
    lib.edit(&[(row(&lib, 8), by_hand(52.52, 13.405))]).unwrap();
    let given = || {
        let plan = lib.neighbour_plan(&Filter::default(), &NeighbourSettings::default()).unwrap();
        plan.sample.iter().any(|(at, ..)| *at == four)
    };
    assert!(!given(), "Berlin is not in Kraków");
    lib.promote(&Filter::default(), &["Poland|Malopolska|Kraków".to_owned()]).unwrap();
    assert!(!given(), "nor for its ≈ position");
}

#[test]
fn a_position_for_a_city_the_gazetteer_lacks_is_not_given() {
    let (_d, lib) = setup();
    let four = row(&lib, 4);
    let unknown = place("Poland", "Malopolska", "Xyzzyville");
    lib.edit(&[(four, Op::SetPlace { base: krakow(), value: unknown })]).unwrap();
    lib.promote(&Filter::default(), &["Poland|Malopolska|Xyzzyville".to_owned()]).unwrap();
    lib.edit(&[(row(&lib, 8), by_hand(50.054, 19.9354))]).unwrap();
    let plan = lib.neighbour_plan(&Filter::default(), &NeighbourSettings::default()).unwrap();
    assert!(!plan.sample.iter().any(|(at, ..)| *at == four), "Kraków is in Malopolska, but is it Xyzzyville?");
}

#[test]
fn photos_with_a_location_or_without() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let at = |location: &str, place: Place| Place { location: Some(location.into()), ..place };
    let photos = vec![
        photo(1, 0, krakow(), None),
        photo(2, 10, at("Rynek", krakow()), None),
        photo(3, 20, Place::default(), None),
        // Levels above left empty: still a location.
        photo(4, 30, at("Beach", place("Poland", "", "")), None),
    ];
    lib.sync(&MemorySource::new("mem", photos, vec![])).unwrap();
    assert_eq!(numbers(&lib, &Filter { located: Some(true), ..Filter::default() }), [2, 4]);
    assert_eq!(numbers(&lib, &Filter { located: Some(false), ..Filter::default() }), [1, 3]);
}
