//! The map: the photos a filter selects that have a position, gathered into clusters that
//! suit the zoom (what `supercluster` does, in Web Mercator pixels).

use std::collections::HashMap;

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
}

/// Cells this many pixels across hold one marker; a marker is a tile about as big.
pub const CELL_PX: f64 = 72.0;
/// From here on photos are only grouped when they sit at the same spot.
pub const SINGLE_ZOOM: f64 = 17.0;

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
    }
    result.clusters = cells
        .into_values()
        .map(|c| Cluster {
            lat: c.lat / c.count as f64,
            lon: c.lon / c.count as f64,
            count: c.count,
            best: c.best.row,
            bounds: c.bounds,
        })
        .collect();
    // The same order every time, for a steady redraw.
    result.clusters.sort_by(|a, b| a.best.cmp(&b.best));
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
    fn only_the_view_is_returned() {
        let points = [at(1, 50.0, 19.0, 0), at(2, -30.0, 140.0, 0)];
        let view = MapView { bounds: BBox { min_lat: 40.0, max_lat: 60.0, min_lon: 10.0, max_lon: 30.0 }, zoom: 5.0 };
        let r = cluster(&points, &view);
        assert_eq!((r.located, r.in_view, r.clusters.len()), (2, 1, 1));
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
