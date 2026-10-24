// A model's place guess on a review card: which levels start ticked, and what accepting
// takes from them.

import type { Guess, PlaceGuess, PlaceNames } from "./api";
import { codeFor, looseKey } from "./countries";
import { parsePath } from "./places";

export const LEVELS = ["country", "state", "city", "location"] as const;
export type Level = (typeof LEVELS)[number];
export type PlaceTicks = Record<Level | "point", boolean>;

const blank = (v: string | null | undefined) => !v || !v.trim();

/** The ticks a card starts with: a level at least `tick` (0..1) likely where the photo has
 *  none of it; the position likewise, and only for a photo with no position. */
export function placeTicks(g: PlaceGuess, now: PlaceNames | null, placed: boolean, tick: number): PlaceTicks {
  const on = (guess: Guess | null, has: boolean) => !!guess && guess.p >= tick && !has;
  const ticks = { point: on(g.point && { name: "", p: g.point.p }, placed) } as PlaceTicks;
  for (const level of LEVELS) ticks[level] = on(g[level], !blank(now?.[level]));
  return ticks;
}

/** A card's name for each level: the model's, until the user writes another. */
export type Names = Record<Level, string>;
export const guessNames = (g: PlaceGuess): Names =>
  Object.fromEntries(LEVELS.map((l) => [l, g[l]?.name ?? ""])) as Names;

/** The place fields accepting takes: the ticked levels, as named on the card (the model's
 *  names when none is given), with the country its code: the model's for its own country,
 *  else the one the name is known by. */
export function takenPlace(g: PlaceGuess, ticks: PlaceTicks, names: Names = guessNames(g)): Partial<PlaceNames> {
  const out: Partial<PlaceNames> = {};
  for (const level of LEVELS) {
    const name = names[level]?.trim();
    if (ticks[level] && name) out[level] = name;
  }
  if (out.country) {
    const code = g.country && looseKey(out.country) === looseKey(g.country.name) ? g.country_code : codeFor(out.country);
    if (code) out.country_code = code;
  }
  return out;
}

/** A place the catalog has, ending at a level: its names, the same compared loosely, and
 *  how many photos are there. */
type Known = { path: Names; loose: Names; n: number };
/** The catalog's places (`place_tree` keys with counts) by the level they end at, read once
 *  so a box's list is quick to make at each key typed. */
export type PlaceIndex = Record<Level, Known[]>;

export function placeIndex(keys: [string, number][]): PlaceIndex {
  const index = Object.fromEntries(LEVELS.map((l) => [l, []])) as unknown as PlaceIndex;
  for (const [key, n] of keys) {
    const level = LEVELS[key.split("|").length - 1];
    if (!level) continue;
    const path = parsePath(key);
    if (!path[level]) continue;
    const loose = Object.fromEntries(LEVELS.map((l) => [l, looseKey(path[l])])) as Names;
    index[level].push({ path, loose, n });
  }
  return index;
}

/** The names the catalog has at `level`, those under the levels named above first, then
 *  the rest; most used first within each. */
export function levelOptions(index: PlaceIndex, level: Level, above: Partial<Names>): string[] {
  const parents = LEVELS.slice(0, LEVELS.indexOf(level))
    .map((l) => [l, looseKey(above[l] ?? "")] as const)
    .filter(([, name]) => name);
  const counts = new Map<string, { n: number; under: boolean }>();
  for (const { path, loose, n } of index[level]) {
    const under = parents.every(([l, name]) => loose[l] === name);
    const was = counts.get(path[level]);
    counts.set(path[level], { n: (was?.n ?? 0) + n, under: !!was?.under || under });
  }
  return [...counts]
    .sort(([a, x], [b, y]) => Number(y.under) - Number(x.under) || y.n - x.n || a.localeCompare(b))
    .map(([name]) => name);
}

/** The catalog's most used place ending in `name` at `level` (compared loosely), or null. */
function placeNamed(index: PlaceIndex, level: Level, name: string): Names | null {
  const loose = looseKey(name);
  let best: Known | null = null;
  for (const known of index[level]) {
    if (known.loose[level] === loose && (!best || known.n > best.n)) best = known;
  }
  return best?.path ?? null;
}

/**
 * A name written at `level` and left: the catalog's spelling of it ("krakow" → "Kraków"),
 * and the levels above it from the catalog's most used place with it, filled only where the
 * card still has the model's name (`model`) or none: a name the user wrote stays. Says which
 * levels it filled. Null when the catalog has no such name there.
 */
export function chooseName(
  index: PlaceIndex, level: Level, name: string, names: Names, model: Names,
): { names: Names; filled: Level[] } | null {
  const path = placeNamed(index, level, name);
  if (!path) return null;
  const out = { ...names, [level]: path[level] };
  const filled: Level[] = [];
  for (const l of LEVELS.slice(0, LEVELS.indexOf(level))) {
    const mine = names[l].trim() && names[l].trim() !== model[l].trim();
    if (!path[l] || mine || names[l] === path[l]) continue;
    out[l] = path[l];
    filled.push(l);
  }
  return { names: out, filled };
}

/** The ticked levels whose name (as on the card) would replace another the photo has now. */
export function placeReplaces(g: PlaceGuess, ticks: PlaceTicks, now: PlaceNames | null, names: Names = guessNames(g)): Level[] {
  return LEVELS.filter((level) => {
    const has = now?.[level];
    return ticks[level] && !blank(has) && names[level]?.trim() !== has?.trim();
  });
}

export const percent = (p: number) => `${Math.round(p * 100)}%`;
