// A model's place guess on a review card: which levels start ticked, and what accepting
// takes from them.

import type { Guess, PlaceGuess, PlaceNames } from "./api";

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

/** The place fields accepting takes: the ticked levels, with the country its code. */
export function takenPlace(g: PlaceGuess, ticks: PlaceTicks): Partial<PlaceNames> {
  const out: Partial<PlaceNames> = {};
  for (const level of LEVELS) {
    const guess = g[level];
    if (ticks[level] && guess) out[level] = guess.name;
  }
  if (out.country && g.country_code) out.country_code = g.country_code;
  return out;
}

/** The ticked levels whose name would replace another the photo has now. */
export function placeReplaces(g: PlaceGuess, ticks: PlaceTicks, now: PlaceNames | null): Level[] {
  return LEVELS.filter((level) => {
    const has = now?.[level];
    return ticks[level] && !blank(has) && g[level]?.name.trim() !== has?.trim();
  });
}

export const percent = (p: number) => `${Math.round(p * 100)}%`;
