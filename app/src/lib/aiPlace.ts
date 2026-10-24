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

/** The names the catalog has at `level` (from `place_tree` keys with counts), those under
 *  the levels named above first, then the rest; most used first within each. */
export function levelOptions(keys: [string, number][], level: Level, above: Partial<Names>): string[] {
  const depth = LEVELS.indexOf(level) + 1;
  const parents = LEVELS.slice(0, depth - 1);
  const counts = new Map<string, { n: number; under: boolean }>();
  for (const [key, n] of keys) {
    if (key.split("|").length !== depth) continue;
    const path = parsePath(key);
    const name = path[level];
    if (!name) continue;
    const under = parents.every((l) => !above[l]?.trim() || looseKey(above[l]!) === looseKey(path[l]));
    const was = counts.get(name);
    counts.set(name, { n: (was?.n ?? 0) + n, under: !!was?.under || under });
  }
  return [...counts]
    .sort(([a, x], [b, y]) => Number(y.under) - Number(x.under) || y.n - x.n || a.localeCompare(b))
    .map(([name]) => name);
}

/** The levels above `name` at `level` in the catalog's most used place with it: picking a
 *  state fills its country. Null when the catalog has no such name there. */
export function aboveOf(keys: [string, number][], level: Level, name: string): Partial<Names> | null {
  const depth = LEVELS.indexOf(level) + 1;
  let best: [string, number] | null = null;
  for (const [key, n] of keys) {
    if (key.split("|").length !== depth || looseKey(parsePath(key)[level]) !== looseKey(name)) continue;
    if (!best || n > best[1]) best = [key, n];
  }
  if (!best) return null;
  const path = parsePath(best[0]);
  return Object.fromEntries(LEVELS.slice(0, depth - 1).filter((l) => path[l]).map((l) => [l, path[l]]));
}

/** The ticked levels whose name (as on the card) would replace another the photo has now. */
export function placeReplaces(g: PlaceGuess, ticks: PlaceTicks, now: PlaceNames | null, names: Names = guessNames(g)): Level[] {
  return LEVELS.filter((level) => {
    const has = now?.[level];
    return ticks[level] && !blank(has) && names[level]?.trim() !== has?.trim();
  });
}

export const percent = (p: number) => `${Math.round(p * 100)}%`;
