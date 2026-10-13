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

/** The address at a point, from OpenStreetMap's Nominatim; null offline or nowhere.
 *  Asked once per spot, at most one a second, as Nominatim's policy wants; told to
 *  wait (429, 503), it is not asked again for that long. A failure is not kept. */
export function placeAt(lat: number, lon: number): Promise<PlacePath | null> {
  const key = `${lat.toFixed(4)},${lon.toFixed(4)}`;
  let asked = hereAsked.get(key);
  if (!asked) {
    const wait = Math.max(0, nextAsk - Date.now());
    nextAsk = Date.now() + wait + 1100;
    asked = new Promise((ok) => setTimeout(ok, wait))
      .then(() => fetch(`https://nominatim.openstreetmap.org/reverse?format=jsonv2&zoom=18&addressdetails=1&lat=${lat}&lon=${lon}`))
      .then((res) => {
        if (res.status === 429 || res.status === 503) {
          const after = Number(res.headers.get("Retry-After"));
          nextAsk = Math.max(nextAsk, Date.now() + (after > 0 ? after * 1000 : 60_000));
        }
        if (!res.ok) throw new Error(`Nominatim: ${res.status}`);
        return res.json();
      })
      .then(fromNominatim)
      .catch(() => {
        hereAsked.delete(key);
        return null;
      });
    hereAsked.set(key, asked);
  }
  return asked;
}
