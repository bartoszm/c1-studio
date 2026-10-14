// Where location hints come from: the catalog's places, the places of photos near a
// point, and the address OpenStreetMap gives for it.

import * as api from "./api";
import { fromNominatim, type PlacePath } from "./places";

/** A `place_tree` facet query's places, kept for the library's `version` only: an edit
 *  since asks again. One that failed is not kept. */
function facetCache() {
  let at = -1;
  const asked = new Map<string, Promise<[string, number][]>>();
  return (key: string, version: number, filter: api.Filter) => {
    if (version !== at) {
      asked.clear();
      at = version;
    }
    let places = asked.get(key);
    if (!places) {
      places = api
        .query(filter, { limit: 0, facets: ["place_tree"] })
        .then((r) => r.facets.place_tree ?? [])
        .catch(() => {
          asked.delete(key);
          return [];
        });
      asked.set(key, places);
    }
    return places;
  };
}

const catalogAsked = facetCache();
/** Every place in the catalog, whatever the view shows. Asked when a location field
 *  is entered, so edits elsewhere cost nothing. */
export const catalogPlaces = (version: number) => catalogAsked("all", version, {});

const nearbyAsked = facetCache();
/** The places of the photos within about `km` of a point, as `place_tree` keys with
 *  counts. */
export function nearbyPlaces(lat: number, lon: number, version: number, km = 3): Promise<[string, number][]> {
  const dLat = km / 111;
  const dLon = km / (111 * Math.max(0.05, Math.cos((lat * Math.PI) / 180)));
  const within = {
    min_lat: Math.max(-90, lat - dLat),
    max_lat: Math.min(90, lat + dLat),
    min_lon: Math.max(-180, lon - dLon),
    max_lon: Math.min(180, lon + dLon),
  };
  return nearbyAsked(`${lat.toFixed(3)},${lon.toFixed(3)}`, version, { within });
}

const hereAsked = new Map<string, Promise<PlacePath | null>>();
let nextAsk = 0;
let online = true;

/** Whether `placeAt` may ask Nominatim; off, it answers null without a request. */
export function allowPlaceLookups(on: boolean) {
  online = on;
}

/** Until when Nominatim told us to wait (429, 503). */
let blockedUntil = 0;
const sleep = (ms: number) => new Promise((ok) => setTimeout(ok, ms));

/** A Nominatim request, at most one a second (as its policy wants) across all of them;
 *  told to wait, none is sent for that long, those already queued included. One that
 *  hangs gives up after 15 s. */
async function nominatim(path: string): Promise<unknown> {
  const wait = Math.max(0, nextAsk - Date.now());
  nextAsk = Date.now() + wait + 1100;
  await sleep(wait);
  while (blockedUntil > Date.now()) await sleep(blockedUntil - Date.now());
  const res = await fetch(`https://nominatim.openstreetmap.org/${path}`, { signal: AbortSignal.timeout(15_000) });
  if (res.status === 429 || res.status === 503) {
    const after = Number(res.headers.get("Retry-After"));
    blockedUntil = Math.max(blockedUntil, Date.now() + (after > 0 ? after * 1000 : 60_000));
    nextAsk = Math.max(nextAsk, blockedUntil);
  }
  if (!res.ok) throw new Error(`Nominatim: ${res.status}`);
  return res.json();
}

const LEVELS: api.Level[] = ["Country", "State", "City", "Location"];

/** Where a place is, from Nominatim: tried with all its names, then without the finest
 *  one, down to the level already known (`have`). `null`: nothing found (worth keeping);
 *  `undefined`: not asked or the request failed (ask again later). */
export async function pointFor(place: api.Photo["place"], have: api.Level | null): Promise<api.Approx | null | undefined> {
  if (!online) return undefined;
  const levels = [place.country, place.state, place.city, place.location];
  const code = place.country_code?.trim().toLowerCase() ?? "";
  for (let level = 3; level >= 1; level--) {
    if (!levels[level]?.trim()) continue;
    if (have && LEVELS.indexOf(have) >= level) break;
    const q = levels.slice(0, level + 1).filter((v) => v?.trim()).reverse().join(", ");
    try {
      const list = await nominatim(
        `search?format=jsonv2&limit=1&q=${encodeURIComponent(q)}${/^[a-z]{2}$/.test(code) ? `&countrycodes=${code}` : ""}`,
      );
      if (!Array.isArray(list)) return undefined;
      if (!list.length) continue;
      const [lat, lon] = [Number(list[0]?.lat), Number(list[0]?.lon)];
      // Not a position: something went wrong, ask again another time.
      if (!Number.isFinite(lat) || !Number.isFinite(lon) || Math.abs(lat) > 90 || Math.abs(lon) > 180) return undefined;
      return { lat, lon, level: LEVELS[level] };
    } catch {
      return undefined;
    }
  }
  return null;
}

/** The address at a point, from OpenStreetMap's Nominatim; null offline (or not allowed online) or nowhere.
 *  Asked once per spot. A failure is not kept. */
export function placeAt(lat: number, lon: number): Promise<PlacePath | null> {
  if (!online) return Promise.resolve(null);
  const key = `${lat.toFixed(4)},${lon.toFixed(4)}`;
  let asked = hereAsked.get(key);
  if (!asked) {
    asked = nominatim(`reverse?format=jsonv2&zoom=18&addressdetails=1&lat=${lat}&lon=${lon}`)
      .then((json) => fromNominatim(json as Parameters<typeof fromNominatim>[0]))
      .catch(() => {
        hereAsked.delete(key);
        return null;
      });
    hereAsked.set(key, asked);
  }
  return asked;
}
