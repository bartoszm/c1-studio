// Location hints. A place nests: the location in a city, the city in a state, the state
// in a country. Typing at one level suggests the places the catalog knows there, those
// near the photo and the one its GPS points at first, then those in the photo's own
// country and state; picking one fills the levels above it too.

import type { Photo, PlaceField } from "./api";
import { codeFor, looseKey, nameFor } from "./countries";

export const LEVELS = ["country", "state", "city", "location"] as const;
export type Level = (typeof LEVELS)[number];
/** A place's levels, "" where it has none, and its ISO code when known. */
export type PlacePath = Record<Level, string> & { code?: string };
export type Place = Photo["place"];
export type PlaceFields = Partial<Record<PlaceField, string>>;

/** Where a suggestion comes from, the strongest first. */
export type Source = "here" | "nearby" | "selection" | "catalog";
const RANK: Record<Source, number> = { here: 0, nearby: 1, selection: 2, catalog: 3 };

export type Suggestion = {
  /** The place up to the level typed in. */
  path: PlacePath;
  /** Its `place_tree` key, `Poland|—|Kraków`. */
  key: string;
  /** The value at the level: `Kraków`. */
  value: string;
  /** The levels above, nearest first: `Małopolskie › Poland`. */
  parents: string;
  source: Source;
  /** Photos there: in the catalog, or in the selection for `selection`. */
  n: number;
};

/** The facet's `—`: a level the place has no value for. */
const NONE = "—";

/** `Poland|—|Kraków` → country Poland, no state, city Kraków. */
export function parsePath(key: string): PlacePath {
  const parts = key.split("|");
  const path = { country: "", state: "", city: "", location: "" };
  LEVELS.forEach((level, i) => {
    const part = parts[i] ?? "";
    path[level] = part === NONE ? "" : part;
  });
  return path;
}

/** The `place_tree` key of a place's first `depth` levels, as the index writes it. */
export function pathKey(path: Record<Level, string | null>, depth: number): string {
  return LEVELS.slice(0, depth)
    .map((l) => (path[l] ?? "").trim().replaceAll("|", "/") || NONE)
    .join("|");
}

const depthOf = (key: string) => key.split("|").length;
const lastOf = (key: string) => key.slice(key.lastIndexOf("|") + 1);
const same = (a: string, b: string) => looseKey(a) === looseKey(b);

const deepestOf = (place: Record<Level, string | null>) =>
  LEVELS.reduce((d, l, i) => ((place[l] ?? "").trim() ? i + 1 : d), 0);

/** A place's `place_tree` key, down to its last level filled; "" for none. */
export const keyOf = (place: Record<Level, string | null>) => pathKey(place, deepestOf(place));

/** The `place_tree` keys a set of places makes, with how many photos each: every level
 *  down to the last one filled, as the index counts them. */
export function treeKeys(places: [Place, number][]): [string, number][] {
  const counts = new Map<string, number>();
  for (const [place, n] of places) {
    for (let depth = 1; depth <= deepestOf(place); depth++) {
      const key = pathKey(place, depth);
      counts.set(key, (counts.get(key) ?? 0) + n);
    }
  }
  return [...counts];
}

export type Sources = {
  /** Every place in the catalog (the `place_tree` facet). */
  catalog: [string, number][];
  /** Places of photos near this one. */
  nearby?: [string, number][];
  /** Places of the selected photos, counted in the selection. */
  selection?: [string, number][];
  /** The place at the photo's GPS, from the map's geocoder. */
  here?: PlacePath | null;
};

/** What to suggest for `level` with `typed` typed in, `context` being the levels the
 *  photo has filled already. */
export function suggest(level: Level, typed: string, context: Partial<Place>, sources: Sources, limit = 12): Suggestion[] {
  const depth = LEVELS.indexOf(level) + 1;
  const found = new Map<string, Suggestion>();
  const add = (key: string, source: Source, n: number, path?: PlacePath) => {
    if (depthOf(key) !== depth || lastOf(key) === NONE) return;
    const had = found.get(key);
    if (had && RANK[had.source] <= RANK[source]) return;
    path ??= parsePath(key);
    found.set(key, { path, key, value: path[level], parents: parentsText(path, depth), source, n });
  };
  for (const [key, n] of sources.catalog) add(key, "catalog", n);
  for (const [key, n] of sources.selection ?? []) add(key, "selection", n);
  if (sources.nearby?.length) {
    const counts = new Map(sources.catalog);
    for (const [key] of sources.nearby) add(key, "nearby", counts.get(key) ?? 0);
  }
  if (sources.here?.[level]) {
    const here = sources.here;
    const known = hereInCatalog(here, depth, sources.catalog);
    if (known.length) for (const [key, n] of known) add(key, "here", n);
    else add(...newPlaceHere(here, depth, context, sources.catalog));
  }

  const q = looseKey(typed);
  const inContext = (s: Suggestion) =>
    LEVELS.slice(0, depth - 1).every((l) => !(context[l] ?? "").trim() || same(s.path[l], context[l]!));
  return [...found.values()]
    .filter((s) => !q || looseKey(s.value).includes(q))
    .map((s) => ({ s, rank: [RANK[s.source], +!inContext(s), +!(q && looseKey(s.value).startsWith(q)), -s.n] }))
    .sort((a, b) => order(a.rank, b.rank) || a.s.value.localeCompare(b.s.value))
    .slice(0, limit)
    .map(({ s }) => s);
}

/** The catalog's places at `depth` that stand for the geocoder's: the same name in the
 *  same country, the most used first. Its names may not be the catalog's (Lesser
 *  Poland, Małopolskie), so the levels between are not compared. */
function hereInCatalog(here: PlacePath, depth: number, catalog: [string, number][]): [string, number][] {
  const level = LEVELS[depth - 1];
  const sameCountry = (country: string) =>
    !here.country || !country || same(country, here.country) || (!!here.code && codeFor(country) === here.code);
  return catalog
    .filter(([key]) => {
      if (depthOf(key) !== depth) return false;
      const path = parsePath(key);
      return (level === "country" || same(path[level], here[level])) && sameCountry(path.country);
    })
    .sort((a, b) => b[1] - a[1]);
}

/** The geocoder's place at `depth`, which the catalog does not have yet, with the levels
 *  above named as the catalog names them where it has the nearest one (Stare Miasto in
 *  the catalog's Kraków, Małopolskie, Poland); else as the photo has them; else as the
 *  geocoder does. Picking it then never renames a level the photo or catalog has. */
function newPlaceHere(here: PlacePath, depth: number, context: Partial<Place>, catalog: [string, number][]): [string, Source, number, PlacePath] {
  const path: PlacePath = { country: "", state: "", city: "", location: "" };
  path[LEVELS[depth - 1]] = here[LEVELS[depth - 1]];
  let named = 0;
  for (let d = depth - 1; d >= 1 && !named; d--) {
    const [parent] = here[LEVELS[d - 1]] ? hereInCatalog(here, d, catalog) : [];
    if (!parent) continue;
    const known = parsePath(parent[0]);
    for (const l of LEVELS.slice(0, d)) path[l] = known[l];
    named = d;
  }
  for (const l of LEVELS.slice(named, depth - 1)) path[l] = (context[l] ?? "").trim() || here[l];
  if (here.code && (!path.country || same(path.country, here.country) || codeFor(path.country) === here.code)) path.code = here.code;
  return [pathKey(path, depth), "here", 0, path];
}

function order(a: number[], b: number[]): number {
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return a[i] - b[i];
  return 0;
}

/** `Małopolskie › Poland`: the levels above the one at `depth`, nearest first. */
function parentsText(path: PlacePath, depth: number): string {
  return LEVELS.slice(0, depth - 1)
    .map((l) => path[l])
    .filter(Boolean)
    .reverse()
    .join(" › ");
}

// Country and ISO code go together: a country fills its code (and clearing it clears
// the code); a code typed where there is no country fills the country.
export function withCode(field: PlaceField, value: string, place: Partial<Place>): PlaceFields {
  const fields: PlaceFields = { [field]: value };
  if (field === "country") {
    const code = value.trim() ? codeFor(value) : "";
    if (code !== null) fields.country_code = code;
    // A name the app does not know: the old country's code would now be wrong. A code
    // that never matched the country (set by hand) is left alone.
    else if (place.country_code && codeFor(place.country ?? "") === place.country_code) fields.country_code = "";
  } else if (field === "country_code" && value.trim() && !place.country) {
    const name = nameFor(value);
    if (name) fields.country = name;
  }
  return fields;
}

/** The fields picking `path` at `level` sets: that level, and those above it the place
 *  names, with the country's code. A level the place leaves empty, and those below,
 *  stay as each photo has them: picking a location never clears the photo's city. */
export function fillFor(path: PlacePath, level: Level, place: Partial<Place>): PlaceFields {
  const fields: PlaceFields = { [level]: path[level] };
  for (const l of LEVELS.slice(0, LEVELS.indexOf(level))) if (path[l]) fields[l] = path[l];
  if (fields.country) {
    const code = path.code ?? withCode("country", fields.country, place).country_code;
    if (code !== undefined) fields.country_code = code;
  }
  return fields;
}

/** Every field of a place, "" for those it lacks: to put the whole place on photos. */
export function allFields(place: Place): PlaceFields {
  return {
    country: place.country ?? "",
    country_code: place.country_code ?? "",
    state: place.state ?? "",
    city: place.city ?? "",
    location: place.location ?? "",
  };
}

export const isEmpty = (place: Place) => Object.values(place).every((v) => !(v ?? "").trim());

/** What one field holds across a selection: each value with its photo count, and how
 *  many photos leave it empty. */
export function fieldValues(places: [Place, number][], field: PlaceField): { values: [string, number][]; empty: number } {
  const values = new Map<string, number>();
  let empty = 0;
  for (const [place, n] of places) {
    const v = (place[field] ?? "").trim();
    if (v) values.set(v, (values.get(v) ?? 0) + n);
    else empty += n;
  }
  return { values: [...values].sort((a, b) => b[1] - a[1]), empty };
}

/** The address OpenStreetMap's reverse geocoder gives for a point, as a place. */
export function fromNominatim(reply: { address?: Record<string, string> } | null): PlacePath | null {
  const a = reply?.address;
  if (!a) return null;
  const first = (...keys: string[]) => keys.map((k) => a[k]).find((v) => v?.trim())?.trim() ?? "";
  const path: PlacePath = {
    country: first("country"),
    state: first("state", "province", "region", "state_district"),
    city: first("city", "town", "village", "municipality", "hamlet"),
    location: first("suburb", "neighbourhood", "quarter", "city_district"),
  };
  const code = first("country_code").toUpperCase();
  if (code) path.code = code;
  return LEVELS.some((l) => path[l]) ? path : null;
}
