//! Trips: stretches of days away from home, found from when and where photos were taken.
//!
//! Each day with photos is classed by where its placed photos were taken: home, away, or
//! unknown (no photo of it has a place). Days follow one another into a stretch while the
//! gap between photos stays under [`TripSettings::gap_hours`] (or a few days, when the
//! country on either side is the same); a day at home ends it. A stretch with a day away
//! is a trip. One with no place at all, but busy enough, is suggested: trips nobody
//! tagged look just like that, and so do some weekends at home.
//!
//! Detection runs again whenever the library changes. What the user did to a trip
//! (named, kept, merged, split, dismissed) is stored and stays; only the trips nobody
//! touched are found afresh.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, bail};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

const TRIPS: TableDefinition<u64, &[u8]> = TableDefinition::new("trips");
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");
const SETTINGS: &str = "settings";
const NEXT_ID: &str = "next_id";

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
/// A photo with a position and no place is at home this close to it, away beyond `AWAY_KM`.
const HOME_KM: f64 = 30.0;
const AWAY_KM: f64 = 80.0;
/// Places at least this share of a trip's placed photos name it, up to `LABEL_PLACES`.
const LABEL_SHARE: f64 = 0.15;
const LABEL_PLACES: usize = 3;

/// Where home is: photos taken there are never part of a trip. With no city, the whole
/// country is home and only trips abroad are found.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Home {
    pub country: Option<String>,
    pub city: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TripSettings {
    /// None until the user picks one; trips are found only then.
    pub home: Option<Home>,
    /// A longer gap between photos starts a new trip.
    pub gap_hours: f64,
    /// ... unless the photos on both sides were taken in the same country and the gap is
    /// no longer than this: a few days without shooting in the middle of a trip.
    pub bridge_days: f64,
    /// Trips with fewer photos are dropped: a stray photo with a wrong clock or place.
    pub min_photos: usize,
    /// A stretch with no place is suggested when it spans this many days ...
    pub suggest_days: usize,
    /// ... and holds this many photos.
    pub suggest_photos: usize,
}

impl Default for TripSettings {
    fn default() -> Self {
        Self { home: None, gap_hours: 36.0, bridge_days: 4.0, min_photos: 10, suggest_days: 2, suggest_photos: 100 }
    }
}

/// What detection needs of a photo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shot {
    /// Capture time, milliseconds since the epoch.
    pub at: i64,
    /// Local day number of `at` (any count of days, as long as it is the same for one day).
    pub day: i64,
    pub country: Option<String>,
    pub city: Option<String>,
    pub geo: Option<(f64, f64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Found, with photos placed away from home.
    Found,
    /// A busy stretch with no place: maybe a trip.
    Suggested,
    /// Touched by the user (named, kept, merged, split or made by hand): detection leaves
    /// it as it is, only stretching it over photos added at its ends.
    Kept,
    /// Not a trip, the user said: detection finds nothing over it again.
    Dismissed,
}

impl Kind {
    fn fixed(self) -> bool {
        matches!(self, Kind::Kept | Kind::Dismissed)
    }
}

/// A trip as stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trip {
    pub id: u64,
    /// First and last photo, milliseconds, inclusive.
    pub start: i64,
    pub end: i64,
    /// The user's name for it; otherwise it is called after its places.
    pub name: Option<String>,
    pub kind: Kind,
}

impl Trip {
    fn overlaps(&self, start: i64, end: i64) -> bool {
        self.start <= end && start <= self.end
    }
}

/// A stretch detection found.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub start: i64,
    pub end: i64,
    pub kind: Kind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Home,
    Away,
    Unknown,
}

fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

fn km((lat1, lon1): (f64, f64), (lat2, lon2): (f64, f64)) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let a = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * ((lon2 - lon1).to_radians() / 2.0).sin().powi(2);
    12_742.0 * a.sqrt().asin()
}

/// Home or away by the photo's place; None when its place does not say.
fn by_place(shot: &Shot, home: &Home) -> Option<Class> {
    let country = shot.country.as_deref().filter(|s| !s.trim().is_empty());
    let city = shot.city.as_deref().filter(|s| !s.trim().is_empty());
    let home_country = home.country.as_deref();
    let other_country = matches!((country, home_country), (Some(c), Some(h)) if !same(c, h));
    match home.city.as_deref() {
        Some(home_city) => match city {
            Some(c) if same(c, home_city) && !other_country => Some(Class::Home),
            Some(_) => Some(Class::Away),
            None if other_country => Some(Class::Away),
            None => None,
        },
        None => match country {
            Some(_) if other_country => Some(Class::Away),
            Some(_) => Some(Class::Home),
            None => None,
        },
    }
}

/// Where home is on the map: the middle of the photos placed at home that have a position.
fn home_point(shots: &[Shot], home: &Home) -> Option<(f64, f64)> {
    let mut lats = Vec::new();
    let mut lons = Vec::new();
    for s in shots {
        if let Some((lat, lon)) = s.geo
            && by_place(s, home) == Some(Class::Home)
        {
            lats.push(lat);
            lons.push(lon);
        }
    }
    if lats.len() < 3 {
        return None;
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    Some((median(&mut lats), median(&mut lons)))
}

fn classify(shot: &Shot, home: &Home, point: Option<(f64, f64)>) -> Class {
    if let Some(class) = by_place(shot, home) {
        return class;
    }
    match (shot.geo, point) {
        (Some(geo), Some(home)) => match km(geo, home) {
            d if d <= HOME_KM => Class::Home,
            d if d >= AWAY_KM => Class::Away,
            _ => Class::Unknown,
        },
        _ => Class::Unknown,
    }
}

struct Day {
    first: i64,
    last: i64,
    photos: usize,
    class: Class,
    /// The country most of its photos away were taken in.
    country: Option<String>,
}

/// The stretches of `shots` (sorted by time) that are trips, or might be.
pub fn detect(shots: &[Shot], settings: &TripSettings) -> Vec<Found> {
    let Some(home) = &settings.home else { return Vec::new() };
    let point = home_point(shots, home);
    let mut days: Vec<Day> = Vec::new();
    let mut tally: (usize, usize, HashMap<String, usize>) = (0, 0, HashMap::new());
    let close = |day: &mut Day, (home, away, countries): &mut (usize, usize, HashMap<String, usize>)| {
        day.class = if *home > *away {
            Class::Home
        } else if *away > 0 {
            Class::Away
        } else {
            Class::Unknown
        };
        day.country = countries.drain().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0))).map(|(c, _)| c);
        *home = 0;
        *away = 0;
    };
    let mut day_no = None;
    for shot in shots {
        if day_no != Some(shot.day) {
            if let Some(day) = days.last_mut() {
                close(day, &mut tally);
            }
            days.push(Day { first: shot.at, last: shot.at, photos: 0, class: Class::Unknown, country: None });
            day_no = Some(shot.day);
        }
        let day = days.last_mut().expect("a day was just pushed");
        day.last = shot.at;
        day.photos += 1;
        match classify(shot, home, point) {
            Class::Home => tally.0 += 1,
            Class::Away => {
                tally.1 += 1;
                if let Some(c) = shot.country.as_deref().filter(|c| !c.trim().is_empty()) {
                    *tally.2.entry(c.trim().to_owned()).or_default() += 1;
                }
            }
            Class::Unknown => {}
        }
    }
    if let Some(day) = days.last_mut() {
        close(day, &mut tally);
    }

    let gap = (settings.gap_hours * HOUR_MS as f64) as i64;
    let bridge = (settings.bridge_days * DAY_MS as f64) as i64;
    let mut stretches: Vec<Vec<&Day>> = Vec::new();
    let mut current: Vec<&Day> = Vec::new();
    let mut last_country: Option<&str> = None;
    for day in &days {
        if day.class == Class::Home {
            stretches.push(std::mem::take(&mut current));
            last_country = None;
            continue;
        }
        if let Some(prev) = current.last() {
            let apart = day.first - prev.last;
            let bridged = apart <= bridge && last_country.is_some() && day.country.as_deref() == last_country;
            if apart > gap && !bridged {
                stretches.push(std::mem::take(&mut current));
                last_country = None;
            }
        }
        if day.country.is_some() {
            last_country = day.country.as_deref();
        }
        current.push(day);
    }
    stretches.push(current);

    let mut out = Vec::new();
    for stretch in stretches.into_iter().filter(|s| !s.is_empty()) {
        let photos: usize = stretch.iter().map(|d| d.photos).sum();
        let away = stretch.iter().any(|d| d.class == Class::Away);
        let kind = if away && photos >= settings.min_photos {
            Kind::Found
        } else if !away && stretch.len() >= settings.suggest_days && photos >= settings.suggest_photos {
            Kind::Suggested
        } else {
            continue;
        };
        out.push(Found { start: stretch[0].first, end: stretch[stretch.len() - 1].last, kind });
    }
    out
}

/// The city photographed on the most days: a guess at home to offer.
pub fn guess_home(shots: &[Shot]) -> Option<Home> {
    let mut days: HashMap<(String, String), HashSet<i64>> = HashMap::new();
    for s in shots {
        if let (Some(country), Some(city)) = (&s.country, &s.city) {
            days.entry((country.trim().to_owned(), city.trim().to_owned())).or_default().insert(s.day);
        }
    }
    days.into_iter()
        .max_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| b.0.cmp(&a.0)))
        .map(|((country, city), _)| Home { country: Some(country), city: Some(city) })
}

/// What a trip is called when the user has not named it: the places most of its placed
/// photos away from home were taken in, `Tokyo, Hong Kong`.
pub fn label(shots: &[Shot], start: i64, end: i64, home: Option<&Home>) -> String {
    let from = shots.partition_point(|s| s.at < start);
    let to = shots.partition_point(|s| s.at <= end);
    let text = |v: &Option<String>| v.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned);
    // Name → (photos, position of the first), and each country's cities.
    let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
    let mut cities: HashMap<String, Vec<String>> = HashMap::new();
    let mut countries_only: Vec<(String, usize)> = Vec::new();
    for (i, s) in shots[from..to].iter().enumerate() {
        if home.is_some_and(|h| by_place(s, h) == Some(Class::Home)) {
            continue;
        }
        match (text(&s.country), text(&s.city)) {
            (country, Some(city)) => {
                counts.entry(city.clone()).or_insert((0, i)).0 += 1;
                if let Some(country) = country {
                    cities.entry(country.to_lowercase()).or_default().push(city);
                }
            }
            (Some(country), None) => countries_only.push((country, i)),
            (None, None) => {}
        }
    }
    // A photo placed only by its country counts for the trip's main city there, so a
    // trip is not called `Berlin, Germany`.
    for (country, i) in countries_only {
        let main = cities
            .get(&country.to_lowercase())
            .and_then(|list| list.iter().max_by_key(|c| counts.get(*c).map_or(0, |n| n.0)).cloned());
        counts.entry(main.unwrap_or(country)).or_insert((0, i)).0 += 1;
    }
    let placed: usize = counts.values().map(|(n, _)| n).sum();
    let mut names: Vec<(String, (usize, usize))> =
        counts.into_iter().filter(|(_, (n, _))| *n as f64 >= placed as f64 * LABEL_SHARE).collect();
    names.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
    names.truncate(LABEL_PLACES);
    // In the order they were visited.
    names.sort_by_key(|(_, (_, first))| *first);
    names.into_iter().map(|(n, _)| n).collect::<Vec<_>>().join(", ")
}

/// Detection's new findings merged into the stored trips: trips the user touched stay
/// (a kept one stretched over a finding that runs past it), findings over them are
/// dropped, and the rest replace the untouched trips, keeping the id of the one they
/// overlap so a selected trip stays selected.
pub fn reconcile(stored: &[Trip], found: &[Found], next_id: &mut u64) -> Vec<Trip> {
    let mut fixed: Vec<Trip> = stored.iter().filter(|t| t.kind.fixed()).cloned().collect();
    let mut old: Vec<&Trip> = stored.iter().filter(|t| !t.kind.fixed()).collect();
    let mut out = Vec::new();
    for f in found {
        let over: Vec<usize> = (0..fixed.len()).filter(|&i| fixed[i].overlaps(f.start, f.end)).collect();
        match over.as_slice() {
            [] => {
                let id = match old.iter().position(|t| t.overlaps(f.start, f.end)) {
                    Some(i) => old.remove(i).id,
                    None => {
                        *next_id += 1;
                        *next_id
                    }
                };
                out.push(Trip { id, start: f.start, end: f.end, name: None, kind: f.kind });
            }
            [i] if fixed[*i].kind == Kind::Kept => {
                let t = &mut fixed[*i];
                t.start = t.start.min(f.start);
                t.end = t.end.max(f.end);
            }
            _ => {}
        }
    }
    out.extend(fixed);
    out.sort_by_key(|t| (t.start, t.id));
    out
}

/// What the user does to trips.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TripEdit {
    /// Names a trip (or takes its name away with None); it is kept from then on.
    Rename { id: u64, name: Option<String> },
    /// Keeps a found or suggested trip as it is.
    Keep { id: u64 },
    /// Not a trip.
    Dismiss { id: u64 },
    /// Forgets what was done to a trip: detection finds what it will over it again.
    Forget { id: u64 },
    /// One trip from the first photo of the first to the last of the last.
    Merge { ids: Vec<u64> },
    /// Two trips: up to just before `at`, and from `at` on.
    Split { id: u64, at: i64 },
    /// A trip made by hand over a span of time.
    Create { start: i64, end: i64, name: Option<String> },
}

/// Applies an edit to the stored trips; returns the id of the trip it leaves selected.
pub fn apply(trips: &mut Vec<Trip>, edit: TripEdit, next_id: &mut u64) -> anyhow::Result<Option<u64>> {
    let mut new_id = || {
        *next_id += 1;
        *next_id
    };
    let find =
        |trips: &Vec<Trip>, id: u64| trips.iter().position(|t| t.id == id).with_context(|| format!("no trip {id}"));
    Ok(match edit {
        TripEdit::Rename { id, name } => {
            let i = find(trips, id)?;
            trips[i].name = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
            trips[i].kind = Kind::Kept;
            Some(id)
        }
        TripEdit::Keep { id } => {
            let i = find(trips, id)?;
            trips[i].kind = Kind::Kept;
            Some(id)
        }
        TripEdit::Dismiss { id } => {
            let i = find(trips, id)?;
            trips[i].kind = Kind::Dismissed;
            None
        }
        TripEdit::Forget { id } => {
            let i = find(trips, id)?;
            trips.remove(i);
            None
        }
        TripEdit::Merge { ids } => {
            if ids.len() < 2 {
                bail!("pick two trips or more to merge");
            }
            let mut picked = Vec::new();
            for id in &ids {
                picked.push(trips.remove(find(trips, *id)?));
            }
            picked.sort_by_key(|t| t.start);
            let start = picked.iter().map(|t| t.start).min().unwrap_or_default();
            let end = picked.iter().map(|t| t.end).max().unwrap_or_default();
            let name = picked.iter().find_map(|t| t.name.clone());
            // Trips that fall inside the merged one become part of it.
            trips.retain(|t| !(t.start >= start && t.end <= end));
            let id = new_id();
            trips.push(Trip { id, start, end, name, kind: Kind::Kept });
            Some(id)
        }
        TripEdit::Split { id, at } => {
            let i = find(trips, id)?;
            let t = trips[i].clone();
            if at <= t.start || at > t.end {
                bail!("split inside the trip, after its first photo");
            }
            trips[i] = Trip { end: at - 1, kind: Kind::Kept, ..t.clone() };
            let id = new_id();
            trips.push(Trip { id, start: at, end: t.end, name: None, kind: Kind::Kept });
            Some(id)
        }
        TripEdit::Create { start, end, name } => {
            if end < start {
                bail!("a trip ends after it starts");
            }
            // What detection found over it gives way.
            trips.retain(|t| t.kind.fixed() || !t.overlaps(start, end));
            let id = new_id();
            let name = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
            trips.push(Trip { id, start, end, name, kind: Kind::Kept });
            Some(id)
        }
    })
}

/// The stored trips and trip settings, in their own file: they are the user's work and
/// outlive the store, which is wiped when the model changes.
pub struct TripBook {
    db: Database,
}

impl TripBook {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let db = Database::create(path).with_context(|| format!("opening trips {}", path.display()))?;
        let tx = db.begin_write()?;
        tx.open_table(TRIPS)?;
        tx.open_table(META)?;
        tx.commit()?;
        Ok(Self { db })
    }

    pub fn settings(&self) -> anyhow::Result<TripSettings> {
        let tx = self.db.begin_read()?;
        let meta = tx.open_table(META)?;
        Ok(match meta.get(SETTINGS)? {
            Some(json) => serde_json::from_str(json.value()).unwrap_or_default(),
            None => TripSettings::default(),
        })
    }

    pub fn set_settings(&self, settings: &TripSettings) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        tx.open_table(META)?.insert(SETTINGS, serde_json::to_string(settings)?.as_str())?;
        tx.commit()?;
        Ok(())
    }

    /// Every stored trip, and the last id given out.
    pub fn load(&self) -> anyhow::Result<(Vec<Trip>, u64)> {
        let tx = self.db.begin_read()?;
        let mut trips = Vec::new();
        for entry in tx.open_table(TRIPS)?.iter()? {
            let (_, bytes) = entry?;
            // An entry this build cannot read is left out rather than failing every view.
            if let Ok(trip) = serde_json::from_slice(bytes.value()) {
                trips.push(trip);
            }
        }
        let next = tx.open_table(META)?.get(NEXT_ID)?.and_then(|v| v.value().parse().ok()).unwrap_or(0);
        Ok((trips, next))
    }

    /// Replaces the stored trips.
    pub fn save(&self, trips: &[Trip], next_id: u64) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(TRIPS)?;
            table.retain(|_, _| false)?;
            for t in trips {
                table.insert(t.id, serde_json::to_vec(t)?.as_slice())?;
            }
            tx.open_table(META)?.insert(NEXT_ID, next_id.to_string().as_str())?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_767_225_600_000; // 2026-01-01 00:00 UTC

    fn shot(day: i64, hour: i64, country: Option<&str>, city: Option<&str>) -> Shot {
        let at = T0 + day * DAY_MS + hour * HOUR_MS;
        Shot { at, day, country: country.map(Into::into), city: city.map(Into::into), geo: None }
    }

    fn krakow() -> TripSettings {
        TripSettings {
            home: Some(Home { country: Some("Poland".into()), city: Some("Kraków".into()) }),
            min_photos: 1,
            suggest_photos: 3,
            ..TripSettings::default()
        }
    }

    /// `n` photos a day over `days`, each at the given place.
    fn days(days: std::ops::Range<i64>, n: i64, country: Option<&str>, city: Option<&str>) -> Vec<Shot> {
        days.flat_map(|d| (0..n).map(move |h| shot(d, 8 + h, country, city))).collect()
    }

    fn spans(found: &[Found]) -> Vec<(i64, i64, Kind)> {
        found.iter().map(|f| ((f.start - T0) / DAY_MS, (f.end - T0) / DAY_MS, f.kind)).collect()
    }

    #[test]
    fn nothing_is_found_until_home_is_set() {
        let shots = days(0..3, 2, Some("Japan"), Some("Tokyo"));
        assert!(detect(&shots, &TripSettings::default()).is_empty());
    }

    #[test]
    fn days_away_between_days_at_home_are_a_trip() {
        let mut shots = days(0..2, 2, Some("Poland"), Some("Kraków"));
        shots.extend(days(2..5, 2, Some("Japan"), Some("Tokyo")));
        shots.extend(days(5..7, 2, Some("Poland"), Some("Kraków")));
        assert_eq!(spans(&detect(&shots, &krakow())), [(2, 4, Kind::Found)]);
    }

    #[test]
    fn untagged_days_inside_a_trip_belong_to_it() {
        let mut shots = days(0..1, 2, Some("Japan"), Some("Tokyo"));
        shots.extend(days(1..3, 2, None, None));
        shots.extend(days(3..4, 2, Some("China"), Some("Hong Kong")));
        assert_eq!(spans(&detect(&shots, &krakow())), [(0, 3, Kind::Found)]);
    }

    #[test]
    fn a_long_gap_splits_unless_the_country_is_the_same() {
        let mut shots = days(0..2, 2, Some("Italy"), Some("Enna"));
        // Three days without photos, then still in Italy: one trip.
        shots.extend(days(5..7, 2, Some("Italy"), Some("Trapani")));
        // Five days more, then Germany: another.
        shots.extend(days(12..14, 2, Some("Germany"), Some("Berlin")));
        assert_eq!(spans(&detect(&shots, &krakow())), [(0, 6, Kind::Found), (12, 13, Kind::Found)]);
    }

    #[test]
    fn a_stray_home_photo_does_not_end_a_trip() {
        let mut shots = days(0..3, 3, Some("Japan"), Some("Tokyo"));
        // One photo of day 1 says Kraków; the day is still away.
        shots[3] = shot(1, 8, Some("Poland"), Some("Kraków"));
        assert_eq!(spans(&detect(&shots, &krakow())), [(0, 2, Kind::Found)]);
    }

    #[test]
    fn busy_stretches_with_no_place_are_suggested() {
        let mut shots = days(0..3, 2, None, None);
        // One day alone is too short to suggest.
        shots.extend(days(10..11, 5, None, None));
        assert_eq!(spans(&detect(&shots, &krakow())), [(0, 2, Kind::Suggested)]);
    }

    #[test]
    fn another_city_at_home_is_a_trip_and_the_country_alone_is_not() {
        let mut shots = days(0..2, 2, Some("Poland"), Some("Grabarka"));
        shots.extend(days(10..12, 2, Some("Poland"), None));
        let s = TripSettings { suggest_photos: 1000, ..krakow() };
        assert_eq!(spans(&detect(&shots, &s)), [(0, 1, Kind::Found)]);
    }

    #[test]
    fn a_country_as_home_finds_only_trips_abroad() {
        let mut shots = days(0..2, 2, Some("Poland"), Some("Grabarka"));
        shots.extend(days(5..7, 2, Some("Germany"), Some("Berlin")));
        let s = TripSettings { home: Some(Home { country: Some("Poland".into()), city: None }), ..krakow() };
        assert_eq!(spans(&detect(&shots, &s)), [(5, 6, Kind::Found)]);
    }

    #[test]
    fn positions_count_when_there_is_no_place() {
        let at_home = |d| Shot { geo: Some((50.06, 19.94)), ..shot(d, 8, Some("Poland"), Some("Kraków")) };
        let mut shots: Vec<Shot> = (0..3).map(at_home).collect();
        // Vienna, with no place: away. Kraków's Rynek, with no place: home.
        shots.push(Shot { geo: Some((48.21, 16.37)), ..shot(3, 8, None, None) });
        shots.push(Shot { geo: Some((48.21, 16.37)), ..shot(4, 8, None, None) });
        shots.push(Shot { geo: Some((50.0614, 19.9366)), ..shot(5, 8, None, None) });
        assert_eq!(spans(&detect(&shots, &krakow())), [(3, 4, Kind::Found)]);
    }

    #[test]
    fn trips_are_named_after_their_main_places_in_visiting_order() {
        let mut shots = days(0..2, 4, Some("Japan"), Some("Tokyo"));
        shots.extend(days(2..3, 3, Some("China"), Some("Hong Kong")));
        shots.push(shot(3, 8, Some("Spain"), Some("Barcelona")));
        shots.push(shot(3, 9, Some("Poland"), Some("Kraków")));
        let home = krakow().home;
        assert_eq!(label(&shots, T0, T0 + 4 * DAY_MS, home.as_ref()), "Tokyo, Hong Kong");
        assert_eq!(label(&shots, T0 + 2 * DAY_MS, T0 + 3 * DAY_MS, home.as_ref()), "Hong Kong");
    }

    #[test]
    fn photos_placed_only_by_country_count_for_its_main_city() {
        let mut shots = days(0..1, 3, Some("Germany"), Some("Berlin"));
        shots.extend(days(1..2, 6, Some("Germany"), None));
        shots.extend(days(2..3, 3, Some("Austria"), None));
        assert_eq!(label(&shots, T0, T0 + 3 * DAY_MS, None), "Berlin, Austria");
    }

    #[test]
    fn the_city_seen_on_most_days_is_the_guess_at_home() {
        let mut shots = days(0..5, 1, Some("Poland"), Some("Kraków"));
        shots.extend(days(5..7, 50, Some("China"), Some("Hong Kong")));
        assert_eq!(guess_home(&shots), Some(Home { country: Some("Poland".into()), city: Some("Kraków".into()) }));
    }

    fn found(start: i64, end: i64, kind: Kind) -> Found {
        Found { start, end, kind }
    }

    fn trip(id: u64, start: i64, end: i64, kind: Kind) -> Trip {
        Trip { id, start, end, name: None, kind }
    }

    #[test]
    fn findings_replace_untouched_trips_and_keep_their_ids() {
        let stored = [trip(1, 0, 10, Kind::Found), trip(2, 20, 30, Kind::Suggested)];
        let mut next = 2;
        let out = reconcile(&stored, &[found(0, 12, Kind::Found), found(40, 50, Kind::Found)], &mut next);
        assert_eq!(out, [trip(1, 0, 12, Kind::Found), trip(3, 40, 50, Kind::Found)]);
    }

    #[test]
    fn touched_trips_stay_and_a_kept_one_grows_over_new_photos() {
        let stored = [trip(1, 0, 10, Kind::Kept), trip(2, 20, 30, Kind::Dismissed)];
        let mut next = 2;
        let out = reconcile(&stored, &[found(0, 15, Kind::Found), found(18, 31, Kind::Suggested)], &mut next);
        assert_eq!(out, [trip(1, 0, 15, Kind::Kept), trip(2, 20, 30, Kind::Dismissed)]);
    }

    #[test]
    fn a_split_trip_is_not_joined_again() {
        let stored = [trip(1, 0, 4, Kind::Kept), trip(2, 5, 10, Kind::Kept)];
        let mut next = 2;
        assert_eq!(reconcile(&stored, &[found(0, 10, Kind::Found)], &mut next), stored);
    }

    #[test]
    fn edits_merge_split_and_create() {
        let mut trips = vec![trip(1, 0, 10, Kind::Found), trip(2, 20, 30, Kind::Found), trip(3, 40, 50, Kind::Found)];
        let mut next = 3;
        let merged = apply(&mut trips, TripEdit::Merge { ids: vec![2, 1] }, &mut next).unwrap();
        assert_eq!(merged, Some(4));
        assert!(trips.contains(&trip(4, 0, 30, Kind::Kept)));
        let second = apply(&mut trips, TripEdit::Split { id: 4, at: 15 }, &mut next).unwrap();
        assert_eq!(second, Some(5));
        assert!(trips.contains(&trip(4, 0, 14, Kind::Kept)) && trips.contains(&trip(5, 15, 30, Kind::Kept)));
        assert!(apply(&mut trips, TripEdit::Split { id: 5, at: 15 }, &mut next).is_err(), "nothing before it");
        // A trip made by hand takes the place of what was found under it.
        apply(&mut trips, TripEdit::Create { start: 35, end: 60, name: Some(" Sicily ".into()) }, &mut next).unwrap();
        assert!(!trips.iter().any(|t| t.id == 3));
        assert_eq!(trips.iter().find(|t| t.id == 6).unwrap().name.as_deref(), Some("Sicily"));
    }

    #[test]
    fn renaming_keeps_and_dismissing_sticks() {
        let mut trips = vec![trip(1, 0, 10, Kind::Suggested)];
        let mut next = 1;
        apply(&mut trips, TripEdit::Rename { id: 1, name: Some("Iceland".into()) }, &mut next).unwrap();
        assert_eq!((trips[0].kind, trips[0].name.as_deref()), (Kind::Kept, Some("Iceland")));
        apply(&mut trips, TripEdit::Dismiss { id: 1 }, &mut next).unwrap();
        assert_eq!(reconcile(&trips, &[found(0, 10, Kind::Suggested)], &mut next), trips);
        apply(&mut trips, TripEdit::Forget { id: 1 }, &mut next).unwrap();
        assert!(trips.is_empty());
    }

    #[test]
    fn the_book_keeps_trips_and_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trips.redb");
        let book = TripBook::open(&path).unwrap();
        assert_eq!(book.settings().unwrap(), TripSettings::default());
        let settings = krakow();
        book.set_settings(&settings).unwrap();
        book.save(&[trip(7, 0, 10, Kind::Kept)], 7).unwrap();
        drop(book);
        let book = TripBook::open(&path).unwrap();
        assert_eq!(book.settings().unwrap(), settings);
        assert_eq!(book.load().unwrap(), (vec![trip(7, 0, 10, Kind::Kept)], 7));
    }
}
