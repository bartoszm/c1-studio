//! The map: the photos a filter selects that have a position, gathered into clusters that
//! suit the zoom (what `supercluster` does, in Web Mercator pixels).

use std::collections::HashMap;

use c1geo::Level;
use serde::{Deserialize, Serialize};

use crate::filter::BBox;

/// A photo with a position, and what picks the best of a cluster.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Located {
    pub row: u64,
    pub lat: f64,
    pub lon: f64,
    pub rating: u64,
    pub sharpness: f64,
}

/// One marker: a single photo (`count` 1) or a cluster of them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Cluster {
    pub lat: f64,
    pub lon: f64,
    pub count: usize,
    /// The photo shown for it: highest rating, then sharpest.
    pub best: u64,
    /// The area its photos cover, to zoom to or filter by.
    pub bounds: BBox,
    /// When there are several photos all at one spot (a stack), all of them, so they can be
    /// moved together; empty otherwise.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapView {
    pub bounds: BBox,
    /// Map zoom as MapLibre counts it (a fraction while zooming).
    pub zoom: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MapResult {
    /// Photos with a position among those selected, seen or not.
    pub located: usize,
    /// Of them, the ones inside the view.
    pub in_view: usize,
    pub clusters: Vec<Cluster>,
    /// Photos placed only by their place's name (city, state, country), or given that
    /// place's position: shown apart, and only while the map is zoomed out to about as
    /// far as that name reaches.
    pub approx: Vec<PlaceCluster>,
    /// Photos with such a position among those selected, seen or not.
    pub approx_located: usize,
    /// Of them, the ones inside the view.
    pub approx_in_view: usize,
    /// Of those, the ones not shown: the map is zoomed in closer than their place.
    pub approx_hidden: usize,
    /// Photos with a place and no position of their own.
    pub place_only: usize,
    /// Of them, those whose place no position could be found for (yet).
    pub unresolved: usize,
    /// Photos with neither a position nor a place.
    pub unplaced: usize,
    /// Photos given a position by AI Locate, accepted as approximate: clustered apart from
    /// positions of their own, so the map can show them in their own colour, or not at all.
    pub ai: Vec<Cluster>,
    /// Photos with such a position among those selected, seen or not.
    pub ai_located: usize,
    /// Of them, the ones inside the view.
    pub ai_in_view: usize,
}

/// Where some photos are, for the map to show them all: the box their positions span,
/// those of their own and those from their places' names.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Area {
    /// None: none of them has a position, or a place a position is known for. The shorter
    /// way round the world: across the 180th meridian it runs west to east from
    /// `min_lon` to `max_lon`, so `min_lon > max_lon` (Fiji at 179.9 and -179.9).
    pub bounds: Option<BBox>,
    /// The least precise place a position came from (None: all are their own), so the
    /// map stays zoomed out enough to show it.
    pub coarsest: Option<Level>,
    /// Photos with a position of their own.
    pub exact: usize,
    /// Photos placed by their place's name, or given that place's position.
    pub approx: usize,
    /// Photos with neither, or with a place no position is known for.
    pub unknown: usize,
    /// The longitudes added, for [`Area::wrapped`] to find the shorter way round.
    #[serde(skip)]
    pub(crate) lons: Vec<f64>,
}

impl Area {
    /// A photo at a position: its own (no level), or its place's. The box grows as on a
    /// flat map; [`Area::wrapped`] then takes the shorter way round.
    pub(crate) fn add(&mut self, lat: f64, lon: f64, level: Option<Level>) {
        self.lons.push(lon);
        self.bounds = Some(match self.bounds {
            None => BBox { min_lat: lat, max_lat: lat, min_lon: lon, max_lon: lon },
            Some(b) => BBox {
                min_lat: b.min_lat.min(lat),
                max_lat: b.max_lat.max(lat),
                min_lon: b.min_lon.min(lon),
                max_lon: b.max_lon.max(lon),
            },
        });
        match level {
            Some(l) => {
                self.approx += 1;
                self.coarsest = Some(self.coarsest.map_or(l, |c| c.min(l)));
            }
            None => self.exact += 1,
        }
    }

    /// The area with its longitudes the shorter way round: the widest gap between them is
    /// what is left out, and when that gap is not the one across the 180th meridian the
    /// box crosses it (`min_lon > max_lon`).
    pub(crate) fn wrapped(mut self) -> Self {
        let mut lons = std::mem::take(&mut self.lons);
        let Some(b) = self.bounds.as_mut() else { return self };
        lons.sort_by(f64::total_cmp);
        let (Some(&first), Some(&last)) = (lons.first(), lons.last()) else { return self };
        // The gap across the meridian, from the most easterly round to the most westerly.
        let mut widest = first + 360.0 - last;
        for pair in lons.windows(2) {
            let gap = pair[1] - pair[0];
            if gap > widest {
                widest = gap;
                (b.min_lon, b.max_lon) = (pair[1], pair[0]);
            }
        }
        self
    }
}

/// The closest zoom a position from a place's name is shown at: further in, the map
/// shows streets the photo may not be in.
pub fn max_zoom(level: Level) -> f64 {
    match level {
        Level::Country => 5.0,
        Level::State => 8.0,
        Level::City => 11.0,
        Level::Location => 14.0,
    }
}

/// Whether a position this precise is shown at `zoom` (zoom 5.9 still counts as 5).
pub fn shown(level: Level, zoom: f64) -> bool {
    zoom < max_zoom(level) + 1.0
}

/// The photos of one place that have only its position.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacePoint {
    /// The place, as [`crate::schema::place_tree`] writes it: `Poland|Malopolska|Kraków`.
    pub path: String,
    pub lat: f64,
    pub lon: f64,
    pub level: Level,
    pub count: usize,
    pub best: Located,
}

/// A marker for photos placed by their place's name: one place or several near.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlaceCluster {
    pub lat: f64,
    pub lon: f64,
    pub count: usize,
    pub best: u64,
    pub bounds: BBox,
    /// The coarsest of its places' levels.
    pub level: Level,
    /// When all its places sit at one point (one place, or a city and spots in it known
    /// no better): what they have in common, `Poland|Malopolska|Kraków`.
    pub place: Option<String>,
    /// Those places, then; empty otherwise.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    pub places: usize,
}

/// The levels two place paths share: `A|B|C` and `A|B|D` → `A|B`.
fn common_path(a: &str, b: &str) -> String {
    a.split('|').zip(b.split('|')).take_while(|(x, y)| x == y).map(|(x, _)| x).collect::<Vec<_>>().join("|")
}

/// The clusters of the places inside `view` that are precise enough for its zoom; also
/// how many photos are in view, and how many of those are not shown.
pub fn cluster_places(places: &[PlacePoint], view: &MapView) -> (Vec<PlaceCluster>, usize, usize) {
    let b = &view.bounds;
    let (dlat, dlon) = ((b.max_lat - b.min_lat) * 0.1, (b.max_lon - b.min_lon) * 0.1);
    let mut cells: HashMap<(i64, i64), (PlaceCluster, Located, f64, f64)> = HashMap::new();
    let (mut in_view, mut hidden) = (0, 0);
    for p in places {
        let seen = p.lat >= b.min_lat && p.lat <= b.max_lat && p.lon >= b.min_lon && p.lon <= b.max_lon;
        if seen {
            in_view += p.count;
        }
        if !shown(p.level, view.zoom) {
            if seen {
                hidden += p.count;
            }
            continue;
        }
        if p.lat < b.min_lat - dlat || p.lat > b.max_lat + dlat || p.lon < b.min_lon - dlon || p.lon > b.max_lon + dlon
        {
            continue;
        }
        let (x, y) = world_px(p.lat, p.lon, view.zoom);
        let key = ((x / CELL_PX).floor() as i64, (y / CELL_PX).floor() as i64);
        let (cell, best, lat, lon) = cells.entry(key).or_insert_with(|| {
            let bounds = BBox { min_lat: p.lat, max_lat: p.lat, min_lon: p.lon, max_lon: p.lon };
            let cluster = PlaceCluster {
                lat: 0.0,
                lon: 0.0,
                count: 0,
                best: p.best.row,
                bounds,
                level: p.level,
                place: Some(p.path.clone()),
                paths: Vec::new(),
                places: 0,
            };
            (cluster, p.best, 0.0, 0.0)
        });
        cell.count += p.count;
        cell.places += 1;
        *lat += p.lat * p.count as f64;
        *lon += p.lon * p.count as f64;
        cell.level = cell.level.min(p.level);
        if better(&p.best, best) {
            *best = p.best;
        }
        cell.bounds.min_lat = cell.bounds.min_lat.min(p.lat);
        cell.bounds.max_lat = cell.bounds.max_lat.max(p.lat);
        cell.bounds.min_lon = cell.bounds.min_lon.min(p.lon);
        cell.bounds.max_lon = cell.bounds.max_lon.max(p.lon);
        let b = &cell.bounds;
        if b.max_lat - b.min_lat > SPOT || b.max_lon - b.min_lon > SPOT {
            cell.place = None;
            cell.paths.clear();
        } else if let Some(shared) = &cell.place {
            let shared = common_path(shared, &p.path);
            cell.place = (!shared.is_empty()).then_some(shared);
            cell.paths.push(p.path.clone());
        }
    }
    let mut out: Vec<PlaceCluster> = cells
        .into_values()
        .map(|(mut c, best, lat, lon)| {
            c.lat = lat / c.count as f64;
            c.lon = lon / c.count as f64;
            c.best = best.row;
            c
        })
        .collect();
    out.sort_by_key(|c| c.best);
    (out, in_view, hidden)
}

/// Cells this many pixels across hold one marker; a marker is a tile about as big.
pub const CELL_PX: f64 = 72.0;
/// From here on photos are only grouped when they sit at the same spot.
pub const SINGLE_ZOOM: f64 = 17.0;
/// Photos this close in degrees (a metre or so) are at one spot.
pub const SPOT: f64 = 1e-5;

const TILE: f64 = 512.0;

fn world_px(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
    let size = TILE * 2f64.powf(zoom);
    let x = (lon + 180.0) / 360.0 * size;
    let s = lat.clamp(-85.0511, 85.0511).to_radians().sin();
    let y = (0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * std::f64::consts::PI)) * size;
    (x, y)
}

fn better(a: &Located, b: &Located) -> bool {
    (a.rating, a.sharpness, std::cmp::Reverse(a.row)) > (b.rating, b.sharpness, std::cmp::Reverse(b.row))
}

struct Cell {
    count: usize,
    lat: f64,
    lon: f64,
    best: Located,
    bounds: BBox,
    /// Its photos while they are all at one spot; `None` once they are not.
    stack: Option<Vec<u64>>,
}

/// The clusters of the points inside `view`. Cells are laid on the whole world, so a
/// marker stays with its photos as the view pans.
pub fn cluster(points: &[Located], view: &MapView) -> MapResult {
    let b = &view.bounds;
    // A margin, so markers at the edge don't pop in as the map is dragged.
    let (dlat, dlon) = ((b.max_lat - b.min_lat) * 0.1, (b.max_lon - b.min_lon) * 0.1);
    let inside = |p: &Located| {
        p.lat >= b.min_lat - dlat && p.lat <= b.max_lat + dlat && p.lon >= b.min_lon - dlon && p.lon <= b.max_lon + dlon
    };
    let mut cells: HashMap<(i64, i64), Cell> = HashMap::new();
    let mut result = MapResult { located: points.len(), ..MapResult::default() };
    for p in points.iter().filter(|p| inside(p)) {
        if p.lat >= b.min_lat && p.lat <= b.max_lat && p.lon >= b.min_lon && p.lon <= b.max_lon {
            result.in_view += 1;
        }
        let key = if view.zoom >= SINGLE_ZOOM {
            // Same spot, to a metre or so.
            ((p.lat * 1e5).round() as i64, (p.lon * 1e5).round() as i64)
        } else {
            let (x, y) = world_px(p.lat, p.lon, view.zoom);
            ((x / CELL_PX).floor() as i64, (y / CELL_PX).floor() as i64)
        };
        let cell = cells.entry(key).or_insert(Cell {
            count: 0,
            lat: 0.0,
            lon: 0.0,
            best: *p,
            bounds: BBox { min_lat: p.lat, max_lat: p.lat, min_lon: p.lon, max_lon: p.lon },
            stack: Some(Vec::new()),
        });
        cell.count += 1;
        cell.lat += p.lat;
        cell.lon += p.lon;
        if better(p, &cell.best) {
            cell.best = *p;
        }
        cell.bounds.min_lat = cell.bounds.min_lat.min(p.lat);
        cell.bounds.max_lat = cell.bounds.max_lat.max(p.lat);
        cell.bounds.min_lon = cell.bounds.min_lon.min(p.lon);
        cell.bounds.max_lon = cell.bounds.max_lon.max(p.lon);
        let b = &cell.bounds;
        if b.max_lat - b.min_lat > SPOT || b.max_lon - b.min_lon > SPOT {
            cell.stack = None;
        } else if let Some(rows) = &mut cell.stack {
            rows.push(p.row);
        }
    }
    result.clusters = cells
        .into_values()
        .map(|c| Cluster {
            lat: c.lat / c.count as f64,
            lon: c.lon / c.count as f64,
            count: c.count,
            best: c.best.row,
            bounds: c.bounds,
            rows: match c.stack {
                Some(rows) if rows.len() > 1 => rows,
                _ => Vec::new(),
            },
        })
        .collect();
    // The same order every time, for a steady redraw.
    result.clusters.sort_by_key(|c| c.best);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(row: u64, lat: f64, lon: f64, rating: u64) -> Located {
        Located { row, lat, lon, rating, sharpness: 0.0 }
    }

    fn world() -> BBox {
        BBox { min_lat: -85.0, max_lat: 85.0, min_lon: -180.0, max_lon: 180.0 }
    }

    #[test]
    fn near_photos_join_when_far_out_and_part_when_close() {
        let points = [at(1, 50.0610, 19.9380, 0), at(2, 50.0620, 19.9390, 3), at(3, 52.2297, 21.0122, 1)];
        let far = cluster(&points, &MapView { bounds: world(), zoom: 3.0 });
        assert_eq!(far.clusters.len(), 2);
        let krakow = far.clusters.iter().find(|c| c.count == 2).unwrap();
        assert_eq!(krakow.best, 2, "the better rated photo stands for the cluster");
        assert_eq!(far.in_view, 3);
        let near = cluster(&points, &MapView { bounds: world(), zoom: 16.0 });
        assert_eq!(near.clusters.len(), 3);
    }

    #[test]
    fn photos_at_one_spot_stay_together_at_the_top_zoom() {
        let points = [at(1, 50.0, 19.0, 0), at(2, 50.0, 19.0, 0), at(3, 50.001, 19.0, 0)];
        let r = cluster(&points, &MapView { bounds: world(), zoom: 19.0 });
        assert_eq!(r.clusters.len(), 2);
        assert_eq!(r.clusters.iter().map(|c| c.count).max(), Some(2));
    }

    #[test]
    fn a_stack_lists_its_photos_and_a_spread_cluster_does_not() {
        let points = [at(1, 50.0, 19.0, 0), at(2, 50.0, 19.0, 0), at(3, 52.0, 21.0, 0)];
        let near = cluster(&points, &MapView { bounds: world(), zoom: 19.0 });
        let stack = near.clusters.iter().find(|c| c.count == 2).unwrap();
        let mut rows = stack.rows.clone();
        rows.sort();
        assert_eq!(rows, [1, 2]);
        assert!(near.clusters.iter().find(|c| c.count == 1).unwrap().rows.is_empty(), "one photo is no stack");
        let far = cluster(&points, &MapView { bounds: world(), zoom: 0.0 });
        assert_eq!(far.clusters.len(), 1);
        assert!(far.clusters[0].rows.is_empty(), "photos apart are not moved as one");
    }

    #[test]
    fn only_the_view_is_returned() {
        let points = [at(1, 50.0, 19.0, 0), at(2, -30.0, 140.0, 0)];
        let view = MapView { bounds: BBox { min_lat: 40.0, max_lat: 60.0, min_lon: 10.0, max_lon: 30.0 }, zoom: 5.0 };
        let r = cluster(&points, &view);
        assert_eq!((r.located, r.in_view, r.clusters.len()), (2, 1, 1));
    }

    fn area_of(lons: &[f64]) -> BBox {
        let mut area = Area::default();
        for &lon in lons {
            area.add(10.0, lon, None);
        }
        area.wrapped().bounds.unwrap()
    }

    #[test]
    fn an_area_takes_the_shorter_way_round() {
        // Either side of the 180th meridian: a box across it, west to east.
        let fiji = area_of(&[178.0, -178.0, 179.5]);
        assert_eq!((fiji.min_lon, fiji.max_lon), (178.0, -178.0));
        // Elsewhere, as on a flat map.
        let europe = area_of(&[19.0, -9.0, 2.0]);
        assert_eq!((europe.min_lon, europe.max_lon), (-9.0, 19.0));
        // Spread round the world: the widest gap is left out, wherever it is.
        let round = area_of(&[-170.0, -10.0, 10.0, 170.0, 100.0]);
        assert_eq!((round.min_lon, round.max_lon), (-10.0, -170.0));
        let pacific = area_of(&[-120.0, 120.0]);
        assert_eq!((pacific.min_lon, pacific.max_lon), (120.0, -120.0));
        assert_eq!(area_of(&[-90.0, 90.0]).min_lon, -90.0, "a tie stays on the flat map");
        let one = area_of(&[179.0]);
        assert_eq!((one.min_lon, one.max_lon), (179.0, 179.0));
    }

    #[test]
    fn bounds_cover_the_members() {
        let points = [at(1, 50.00, 19.00, 0), at(2, 50.01, 19.02, 0)];
        let r = cluster(&points, &MapView { bounds: world(), zoom: 2.0 });
        assert_eq!(r.clusters.len(), 1);
        let b = r.clusters[0].bounds;
        assert_eq!((b.min_lat, b.max_lat, b.min_lon, b.max_lon), (50.0, 50.01, 19.0, 19.02));
    }
}
