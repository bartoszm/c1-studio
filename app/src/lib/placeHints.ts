// Where location hints come from besides the catalog: the places of photos near a point,
// and the address OpenStreetMap gives for it.

import * as api from "./api";
import { fromNominatim, type PlacePath } from "./places";

const nearbyAsked = new Map<string, Promise<[string, number][]>>();

/** The places of the photos within about `km` of a point, as `place_tree` keys with
 *  counts. `version` is the library's: an edit since asks again. */
export function nearbyPlaces(lat: number, lon: number, version: number, km = 3): Promise<[string, number][]> {
  const key = `${lat.toFixed(3)},${lon.toFixed(3)},${version}`;
  let asked = nearbyAsked.get(key);
  if (!asked) {
    const dLat = km / 111;
    const dLon = km / (111 * Math.max(0.05, Math.cos((lat * Math.PI) / 180)));
    const within = { min_lat: lat - dLat, max_lat: lat + dLat, min_lon: lon - dLon, max_lon: lon + dLon };
    asked = api
      .query({ within }, { limit: 0, facets: ["place_tree"] })
      .then((r) => r.facets.place_tree ?? [])
      .catch(() => []);
    nearbyAsked.set(key, asked);
  }
  return asked;
}

const hereAsked = new Map<string, Promise<PlacePath | null>>();
let nextAsk = 0;

/** The address at a point, from OpenStreetMap's Nominatim; null offline or nowhere.
 *  Asked once per spot, at most one a second, as Nominatim's policy wants. */
export function placeAt(lat: number, lon: number): Promise<PlacePath | null> {
  const key = `${lat.toFixed(4)},${lon.toFixed(4)}`;
  let asked = hereAsked.get(key);
  if (!asked) {
    const wait = Math.max(0, nextAsk - Date.now());
    nextAsk = Date.now() + wait + 1100;
    asked = new Promise((ok) => setTimeout(ok, wait))
      .then(() => fetch(`https://nominatim.openstreetmap.org/reverse?format=jsonv2&zoom=18&addressdetails=1&lat=${lat}&lon=${lon}`))
      .then((res) => (res.ok ? res.json() : null))
      .then(fromNominatim)
      .catch(() => {
        hereAsked.delete(key);
        return null;
      });
    hereAsked.set(key, asked);
  }
  return asked;
}
