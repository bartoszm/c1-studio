// What the map shows: where a place found by name goes, the area the grid follows, and when
// a trip's route is fitted again.
import type { BBox, Filter } from "./api";

/** A place found by name: its point, and Nominatim's box (west, south, east, north),
 *  rank and kind of place. */
export type Hit = {
  name: string;
  lat: number;
  lon: number;
  box: [number, number, number, number] | null;
  rank?: number;
  type?: string;
};

/** A row of Nominatim's `format=jsonv2` answer, which carries `place_rank` and `addresstype`. */
export type NominatimRow = {
  display_name: string;
  lat: string;
  lon: string;
  boundingbox?: string[];
  place_rank?: number;
  addresstype?: string;
};

export function parseHits(list: NominatimRow[]): Hit[] {
  return list.map((h) => ({
    name: h.display_name,
    lat: +h.lat,
    lon: +h.lon,
    // Nominatim's order is south, north, west, east.
    box: h.boundingbox ? [+h.boundingbox[2], +h.boundingbox[0], +h.boundingbox[3], +h.boundingbox[1]] : null,
    rank: h.place_rank,
    type: h.addresstype,
  }));
}

/** The zoom that suits a kind of place. */
const TYPE_ZOOM: Record<string, number> = {
  continent: 3,
  country: 5,
  state: 7, region: 7, province: 7,
  state_district: 9, county: 9, district: 9,
  city: 10, municipality: 10,
  town: 12, borough: 12,
  village: 13, suburb: 13,
  quarter: 14, hamlet: 14, neighbourhood: 14, city_block: 14,
};
/** By Nominatim's rank, when the kind is not one of those: [highest rank, zoom]. */
const RANK_ZOOM: [number, number][] = [[4, 5], [9, 7], [12, 9], [16, 10], [18, 12], [20, 13], [22, 14], [25, 15]];
/** Roads, buildings and anything finer. */
const FINE_ZOOM = 16;
const MAX_ZOOM = 17;
/** The world is this many pixels wide at zoom 0. */
const TILE = 512;

const mercatorY = (lat: number) => {
  const s = Math.sin((Math.max(-85, Math.min(85, lat)) * Math.PI) / 180);
  return 0.5 - Math.log((1 + s) / (1 - s)) / (4 * Math.PI);
};

/** The zoom for a hit's kind, or rank; null when Nominatim gave neither. */
export function placeZoom(hit: Pick<Hit, "rank" | "type">): number | null {
  if (hit.type && hit.type in TYPE_ZOOM) return TYPE_ZOOM[hit.type];
  if (hit.rank !== undefined) return RANK_ZOOM.find(([rank]) => hit.rank! <= rank)?.[1] ?? FINE_ZOOM;
  return hit.type ? FINE_ZOOM : null;
}

/**
 * Where the map goes for a place found: always its own point. The box sets the zoom only
 * when the point lies well inside it and it doesn't reach across ±180: a place's box covers
 * all of it, far-off islands included (Tokyo's runs to Minamitorishima, France's to the
 * Pacific, Fiji's round the world), so its middle can be far from the place.
 */
export function hitView(hit: Hit, size: { width: number; height: number }, padding = 40): { center: [number, number]; zoom: number } {
  const center: [number, number] = [hit.lon, hit.lat];
  const zoom = boxZoom(hit, size, padding) ?? placeZoom(hit) ?? 14;
  return { center, zoom };
}

function boxZoom(hit: Hit, size: { width: number; height: number }, padding: number): number | null {
  if (!hit.box) return null;
  const [west, south, east, north] = hit.box;
  if (!(east > west) || east - west >= 180 || north < south) return null;
  const inside = (v: number, lo: number, hi: number) => hi === lo ? v === lo : (v - lo) / (hi - lo) >= 0.2 && (v - lo) / (hi - lo) <= 0.8;
  if (!inside(hit.lon, west, east) || !inside(hit.lat, south, north)) return null;
  // Centred on the point, the box must still fit: twice its farther side, each way.
  const dx = (2 * Math.max(hit.lon - west, east - hit.lon)) / 360;
  const y = mercatorY(hit.lat);
  const dy = 2 * Math.max(mercatorY(south) - y, y - mercatorY(north));
  const fit = (px: number, part: number) => (part > 0 ? Math.log2(Math.max(1, px - 2 * padding) / (TILE * part)) : MAX_ZOOM);
  return Math.max(0, Math.min(MAX_ZOOM, fit(size.width, dx), fit(size.height, dy)));
}

/** A longitude in -180..180. */
export const wrapLon = (lon: number) => (lon >= -180 && lon < 180 ? lon : ((((lon + 180) % 360) + 360) % 360) - 180);

/**
 * The area the map shows as one box. Panned round the world, maplibre's edges run past
 * ±180 (west 422, east 578): they are wrapped. A view across ±180 cannot be one box, so it
 * takes every longitude in its latitudes.
 */
export function viewBox(west: number, south: number, east: number, north: number): BBox {
  const lat = { min_lat: Math.max(-90, south), max_lat: Math.min(90, north) };
  const w = wrapLon(west);
  const e = w + (east - west);
  if (east - west >= 360 || e > 180) return { ...lat, min_lon: -180, max_lon: 180 };
  return { ...lat, min_lon: w, max_lon: e };
}

/**
 * A trip's route read again: only another trip is fitted, played from its end, and stops
 * the playback; the same one (after an edit) keeps the view and the playhead, within its
 * new length.
 */
export function routeStep(shown: string | null, trip: string | null, along: number, length: number): { fresh: boolean; along: number } {
  const end = Math.max(0, length - 1);
  if (trip !== shown) return { fresh: true, along: end };
  return { fresh: false, along: Math.min(Math.max(0, along), end) };
}

/**
 * Whether the grid should take the map's view now: following is on, the Map tab is open,
 * no tile is picked, the grid doesn't show photos the map can't place (no GPS, or only
 * approximate), and it isn't held to an area already.
 */
export function shouldFollow(s: { follow: boolean; mapOn: boolean; tile: boolean; filter: Pick<Filter, "gps" | "approx" | "within"> }): boolean {
  return s.follow && s.mapOn && !s.tile && s.filter.gps !== false && !s.filter.approx && !s.filter.within;
}
