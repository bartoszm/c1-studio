//! Places without the network: where a country, state or city named on a photo lies, and
//! which place a position is in. From GeoNames (CC BY 4.0, https://www.geonames.org/):
//! every country, its first-level divisions and every place of 1000 people or more, built
//! by `tools/gazetteer/build.py`.

mod coords;
mod fold;
mod pluscode;

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use c1model::Place;
use serde::{Deserialize, Serialize};

pub use coords::parse_point;
pub use fold::fold;

/// How precisely a position stands for a photo's place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Level {
    Country,
    State,
    City,
    /// The place inside a city (`ZIMAGE_LOCATION`): only an online lookup finds these.
    Location,
}

impl Level {
    /// The finest level a place's text names, if any.
    pub fn named(place: &Place) -> Option<Level> {
        let has = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.trim().is_empty());
        if has(&place.location) {
            Some(Level::Location)
        } else if has(&place.city) {
            Some(Level::City)
        } else if has(&place.state) {
            Some(Level::State)
        } else if has(&place.country) || has(&place.country_code) {
            Some(Level::Country)
        } else {
            None
        }
    }
}

/// Where a place is, as precisely as it could be found.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Approx {
    pub lat: f64,
    pub lon: f64,
    pub level: Level,
}

/// What a place's names were found as, by index into the gazetteer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Resolved {
    pub country: Option<u32>,
    pub state: Option<u32>,
    pub city: Option<u32>,
}

/// The place a position is in.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Found {
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: String,
    pub country_code: String,
    /// What it was found as, to match against the names a catalog already uses.
    #[serde(skip)]
    pub ids: Resolved,
}

struct Country {
    code: String,
    name: String,
    lat: f64,
    lon: f64,
}

struct State {
    name: String,
    lat: f64,
    lon: f64,
    /// Every name, folded.
    names: Vec<String>,
}

struct City {
    country: u32,
    state: Option<u32>,
    population: u32,
    lat: f64,
    lon: f64,
    name: String,
    /// A district of a bigger city near it: found by name, never as what a position is in.
    part: bool,
}

pub struct Gazetteer {
    countries: Vec<Country>,
    country_by_code: HashMap<String, u32>,
    country_by_name: HashMap<String, u32>,
    states: Vec<State>,
    /// Per country, its states.
    states_of: Vec<Vec<u32>>,
    cities: Vec<City>,
    city_by_name: HashMap<String, Vec<u32>>,
    /// Whole cities (no districts) by 1° cell, for finding what a position is in.
    grid: HashMap<(i32, i32), Vec<u32>>,
}

static DATA: &[u8] = include_bytes!("../data/gazetteer.tsv.gz");
static GAZETTEER: OnceLock<Gazetteer> = OnceLock::new();

/// The gazetteer, read on first use (a fraction of a second).
pub fn gazetteer() -> &'static Gazetteer {
    GAZETTEER.get_or_init(|| {
        let mut text = String::new();
        flate2::read::GzDecoder::new(DATA).read_to_string(&mut text).expect("the gazetteer is built in");
        Gazetteer::parse(&text)
    })
}

/// How far a place of this many people reaches, roughly: Kraków (800k) about 24 km.
fn reach_km(population: u32) -> f64 {
    1.0 + (population as f64).sqrt() / 40.0
}

fn km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let dlat = (lat2 - lat1).to_radians();
    let mut dlon = lon2 - lon1;
    if dlon > 180.0 {
        dlon -= 360.0;
    } else if dlon < -180.0 {
        dlon += 360.0;
    }
    let dlon = dlon.to_radians() * ((lat1 + lat2) / 2.0).to_radians().cos();
    6371.0 * dlat.hypot(dlon)
}

/// How far `reverse` looks: past the reach of the biggest cities (Moscow, about 96 km).
const SEARCH_KM: f64 = 100.0;

fn cell(lat: f64, lon: f64) -> (i32, i32) {
    (lat.floor() as i32, lon.floor() as i32)
}

impl Gazetteer {
    fn parse(text: &str) -> Self {
        let mut g = Gazetteer {
            countries: Vec::new(),
            country_by_code: HashMap::new(),
            country_by_name: HashMap::new(),
            states: Vec::new(),
            states_of: Vec::new(),
            cities: Vec::new(),
            city_by_name: HashMap::new(),
            grid: HashMap::new(),
        };
        let mut state_by_key: HashMap<&str, u32> = HashMap::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let number = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            match f.first() {
                Some(&"C") if f.len() > 4 => {
                    let id = g.countries.len() as u32;
                    g.country_by_code.insert(f[1].to_owned(), id);
                    for name in &f[4..] {
                        g.country_by_name.entry(fold(name)).or_insert(id);
                    }
                    g.countries.push(Country {
                        code: f[1].to_owned(),
                        name: f[4].to_owned(),
                        lat: number(2),
                        lon: number(3),
                    });
                    g.states_of.push(Vec::new());
                }
                Some(&"A") if f.len() > 4 => {
                    let Some(&country) = f[1].split('.').next().and_then(|c| g.country_by_code.get(c)) else {
                        continue;
                    };
                    let id = g.states.len() as u32;
                    state_by_key.insert(f[1], id);
                    let mut names: Vec<String> = f[4..].iter().map(|n| fold(n)).collect();
                    names.dedup();
                    g.states.push(State { name: f[4].to_owned(), lat: number(2), lon: number(3), names });
                    g.states_of[country as usize].push(id);
                }
                Some(&kind @ ("P" | "D")) if f.len() > 6 => {
                    let Some(&country) = g.country_by_code.get(f[1]) else { continue };
                    let id = g.cities.len() as u32;
                    let state = state_by_key.get(format!("{}.{}", f[1], f[2]).as_str()).copied();
                    let city = City {
                        country,
                        state,
                        population: f[3].parse().unwrap_or(0),
                        lat: number(4),
                        lon: number(5),
                        name: f[6].to_owned(),
                        part: kind == "D",
                    };
                    let mut seen: Vec<String> = Vec::new();
                    for name in &f[6..] {
                        let key = fold(name);
                        if !key.is_empty() && !seen.contains(&key) {
                            g.city_by_name.entry(key.clone()).or_default().push(id);
                            seen.push(key);
                        }
                    }
                    if !city.part {
                        g.grid.entry(cell(city.lat, city.lon)).or_default().push(id);
                    }
                    g.cities.push(city);
                }
                _ => {}
            }
        }
        g
    }

    pub fn country_name(&self, code: &str) -> Option<&str> {
        let id = self.country_by_code.get(&code.trim().to_ascii_uppercase())?;
        Some(&self.countries[*id as usize].name)
    }

    fn country(&self, place: &Place) -> Option<u32> {
        let code = place.country_code.as_deref().map(|c| c.trim().to_ascii_uppercase());
        if let Some(id) = code.and_then(|c| self.country_by_code.get(&c)) {
            return Some(*id);
        }
        let name = fold(place.country.as_deref()?);
        self.country_by_name.get(&name).copied().or_else(|| {
            // `United States of America`, `Polska Rzeczpospolita`: a name it starts with.
            self.country_by_name
                .iter()
                .filter(|(n, _)| fold::alike_start(n, &name))
                .max_by_key(|(n, _)| n.len())
                .map(|(_, id)| *id)
        })
    }

    fn state(&self, country: u32, text: &str) -> Option<u32> {
        let name = fold(text);
        if name.is_empty() {
            return None;
        }
        let states = &self.states_of[country as usize];
        if let Some(id) = states.iter().find(|&&s| self.states[s as usize].names.contains(&name)) {
            return Some(*id);
        }
        // `Malopolska` for `Małopolskie`, `Lower Silesian` for `Lower Silesia`: the state
        // with the longest name sharing most of its start.
        states
            .iter()
            .flat_map(|&s| self.states[s as usize].names.iter().map(move |n| (s, n)))
            .filter(|(_, n)| fold::alike_start(n, &name))
            .max_by_key(|(_, n)| n.chars().zip(name.chars()).take_while(|(a, b)| a == b).count())
            .map(|(s, _)| s)
    }

    /// The city named `text`: in `country` if known, preferably in `state`; a whole city
    /// before a district, then the biggest. `typos`: also a name one typo away.
    fn city(&self, country: Option<u32>, state: Option<u32>, text: &str, typos: bool) -> Option<u32> {
        let name = fold(text);
        if name.is_empty() {
            return None;
        }
        let fits = |id: &u32| country.is_none_or(|c| self.cities[*id as usize].country == c);
        let mut found: Vec<u32> = self.city_by_name.get(&name).into_iter().flatten().copied().filter(fits).collect();
        if found.is_empty() && typos && country.is_some() {
            found = self
                .city_by_name
                .iter()
                .filter(|(n, _)| fold::one_typo(n, &name))
                .flat_map(|(_, ids)| ids.iter().copied())
                .filter(fits)
                .collect();
        }
        found.into_iter().max_by_key(|&id| {
            let c = &self.cities[id as usize];
            (state.is_some() && c.state == state, !c.part, c.population)
        })
    }

    /// What a place's names are in the gazetteer. A city found without its country or
    /// state given brings its own.
    pub fn resolve(&self, place: &Place) -> Resolved {
        let mut r = Resolved { country: self.country(place), ..Resolved::default() };
        r.state = r.country.and_then(|c| self.state(c, place.state.as_deref().unwrap_or("")));
        r.city = place
            .city
            .as_deref()
            .and_then(|t| self.city(r.country, r.state, t, true))
            // `Germany, —, —, Berlin`: a city typed as the location.
            .or_else(|| place.location.as_deref().and_then(|t| self.city(r.country, r.state, t, false)));
        if let Some(c) = r.city.map(|id| &self.cities[id as usize]) {
            r.country.get_or_insert(c.country);
            if r.state.is_none() {
                r.state = c.state;
            }
        }
        r
    }

    /// Where a place is, at the finest level its names were found.
    pub fn locate(&self, place: &Place) -> Option<Approx> {
        let r = self.resolve(place);
        if let Some(c) = r.city.map(|id| &self.cities[id as usize]) {
            return Some(Approx { lat: c.lat, lon: c.lon, level: Level::City });
        }
        // Without a city, a state comes from the place's own text.
        if let Some(s) = r.state.map(|id| &self.states[id as usize]) {
            return Some(Approx { lat: s.lat, lon: s.lon, level: Level::State });
        }
        let c = &self.countries[r.country? as usize];
        Some(Approx { lat: c.lat, lon: c.lon, level: Level::Country })
    }

    /// The place `lat, lon` is in: the city whose reach covers it best, or, away from any
    /// city, the state and country of the nearest one within 60 km.
    pub fn reverse(&self, lat: f64, lon: f64) -> Option<Found> {
        let (cy, cx) = cell(lat, lon);
        let mut best: Option<(f64, u32)> = None;
        let mut nearest: Option<(f64, u32)> = None;
        // Cells as far as the biggest city reaches (about 100 km); a degree of longitude
        // shrinks towards the poles, so more of them there.
        let shrink = (lat.abs() + 1.0).min(89.0).to_radians().cos().max(0.005);
        let across = ((SEARCH_KM / (111.0 * shrink)).ceil() as i32).clamp(1, 179);
        for dy in -1..=1 {
            for dx in -across..=across {
                let x = (cx + dx + 180).rem_euclid(360) - 180;
                for &id in self.grid.get(&(cy + dy, x)).into_iter().flatten() {
                    let c = &self.cities[id as usize];
                    let d = km(lat, lon, c.lat, c.lon);
                    let reach = reach_km(c.population);
                    if d <= reach + 8.0 {
                        let score = d / reach;
                        if best.is_none_or(|(s, _)| score < s) {
                            best = Some((score, id));
                        }
                    }
                    if d <= 60.0 && nearest.is_none_or(|(n, _)| d < n) {
                        nearest = Some((d, id));
                    }
                }
            }
        }
        let (id, in_city) = match (best, nearest) {
            (Some((_, id)), _) => (id, true),
            (None, Some((_, id))) => (id, false),
            (None, None) => return None,
        };
        let c = &self.cities[id as usize];
        let country = &self.countries[c.country as usize];
        Some(Found {
            city: in_city.then(|| c.name.clone()),
            state: c.state.map(|s| self.states[s as usize].name.clone()),
            country: country.name.clone(),
            country_code: country.code.clone(),
            ids: Resolved { country: Some(c.country), state: c.state, city: in_city.then_some(id) },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(country: &str, code: &str, state: &str, city: &str, location: &str) -> Place {
        let some = |s: &str| (!s.is_empty()).then(|| s.to_owned());
        Place {
            country: some(country),
            country_code: some(code),
            state: some(state),
            city: some(city),
            location: some(location),
        }
    }

    fn near(a: Approx, lat: f64, lon: f64, level: Level) {
        assert_eq!(a.level, level);
        assert!(km(a.lat, a.lon, lat, lon) < 30.0, "{a:?} is not near {lat}, {lon}");
    }

    #[test]
    fn cities_by_any_of_their_names() {
        let g = gazetteer();
        for name in ["Kraków", "Krakow", "Cracow", "KRAKOW"] {
            near(g.locate(&place("Poland", "PL", "Malopolska", name, "")).unwrap(), 50.06, 19.94, Level::City);
        }
        near(g.locate(&place("Japan", "JP", "", "Tokio", "")).unwrap(), 35.69, 139.69, Level::City);
        near(g.locate(&place("Turkey", "TR", "", "Istambul", "")).unwrap(), 41.01, 28.95, Level::City);
        near(g.locate(&place("Portugal", "PT", "", "Lisboa", "")).unwrap(), 38.72, -9.13, Level::City);
        near(g.locate(&place("Poland", "PL", "Lower Silesian", "Walbrzych", "")).unwrap(), 50.77, 16.28, Level::City);
        near(g.locate(&place("Italy", "IT", "Sicily", "Aidone", "")).unwrap(), 37.41, 14.45, Level::City);
    }

    #[test]
    fn a_city_typed_as_the_location() {
        let berlin = gazetteer().locate(&place("Germany", "DE", "", "", "Berlin")).unwrap();
        near(berlin, 52.52, 13.40, Level::City);
    }

    #[test]
    fn countries_and_states_when_there_is_no_city() {
        let g = gazetteer();
        assert_eq!(g.locate(&place("Polska", "", "", "", "")).unwrap().level, Level::Country);
        assert_eq!(g.locate(&place("", "PL", "", "", "")).unwrap().level, Level::Country);
        assert_eq!(g.locate(&place("United  Kingdom", "", "", "", "")).map(|a| a.level), Some(Level::Country));
        let malopolska = g.locate(&place("Poland", "PL", "Malopolska", "", "Czorsztynski")).unwrap();
        assert_eq!(malopolska.level, Level::State);
        assert!(km(malopolska.lat, malopolska.lon, 50.0, 20.0) < 80.0);
        assert_eq!(g.locate(&place("", "", "", "", "")), None);
        assert_eq!(g.locate(&place("Atlantis", "", "", "", "")), None);
    }

    #[test]
    fn what_a_position_is_in() {
        let g = gazetteer();
        let found = g.reverse(50.0647, 19.945).unwrap();
        assert_eq!((found.city.as_deref(), found.country_code.as_str()), (Some("Kraków"), "PL"));
        assert_eq!(found.country, "Poland");
        assert!(found.state.is_some());
        // A district is never the answer.
        assert_eq!(g.reverse(50.0642, 19.9676).unwrap().city.as_deref(), Some("Kraków"));
        let krk = g.reverse(44.9667, 14.75).unwrap();
        assert_eq!(krk.country_code, "HR");
        // The middle of the Atlantic.
        assert_eq!(g.reverse(30.0, -40.0), None);
        // Far north a degree of longitude is short (38 km at 70°): the city 60 km away
        // is three cells off, and still found.
        let tromso = g.reverse(69.6496, 18.956).unwrap();
        let east = g.reverse(69.6496, 18.956 + 60.0 / (111.0 * 69.6496f64.to_radians().cos())).unwrap();
        assert_eq!((east.country_code.as_str(), east.state.as_deref()), ("NO", tromso.state.as_deref()));
    }

    #[test]
    fn names_resolve_the_same_as_found() {
        let g = gazetteer();
        let found = g.reverse(50.0614, 19.9366).unwrap();
        let typed = g.resolve(&place("Poland", "PL", "Malopolska", "Krakow", ""));
        assert_eq!(found.ids, typed);
    }
}
