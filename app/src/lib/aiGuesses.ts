// AI Locate's guesses on the map: a pin per photo (or per scene) not yet reviewed, with the
// circle its position may be off by, and what accepting one from the map takes.

import type { Accept, SuggestionRow } from "./api";
import { guessNames, placeTicks, preciseOffered, scenePicks, seenOf, takenPlace } from "./aiPlace";

/** A guess to show: the card's photo, its scene's others, and where it puts them. */
export type GuessPin = {
  lead: SuggestionRow;
  members: SuggestionRow[];
  lat: number;
  lon: number;
  radius_km: number;
  p: number;
};

/** The guesses with a position, one per scene: those of a scene share the first listed. */
export function guessPins(rows: SuggestionRow[]): GuessPin[] {
  const by = new Map<string, SuggestionRow[]>();
  for (const r of rows) {
    if (r.status !== "Proposed" || !r.suggestion.place?.point) continue;
    const key = r.group == null ? `row:${r.row}` : `group:${r.group}`;
    const list = by.get(key);
    if (list) list.push(r);
    else by.set(key, [r]);
  }
  return [...by.values()].map(([lead, ...members]) => {
    const pt = lead.suggestion.place!.point!;
    return { lead, members, lat: pt.lat, lon: pt.lon, radius_km: Math.max(pt.radius_km, MIN_RADIUS_KM), p: pt.p };
  });
}

/** The smallest circle drawn and told of: closer than this a guess is not. */
export const MIN_RADIUS_KM = 0.05;

/** The position starts ticked when it is as likely as `tick` (0..1) asks. */
export function pointTicked(pin: GuessPin, tick: number): boolean {
  return pin.p >= tick;
}

/** The photo has a position of its own: a guessed one is never given. */
export const ownPosition = (r: SuggestionRow) => !!r.current?.placed && !r.current?.approx;

/** A circle `km` round a point, as a polygon ring of lon, lat pairs (closed). Longitudes
 *  run on past ±180 rather than wrap, so a circle across the date line stays one ring. */
export function circle(lat: number, lon: number, km: number, steps = 64): [number, number][] {
  const r = Math.max(km, MIN_RADIUS_KM) / 6371;
  const [φ1, λ1] = [(lat * Math.PI) / 180, (lon * Math.PI) / 180];
  const ring: [number, number][] = [];
  for (let i = 0; i <= steps; i++) {
    const θ = (2 * Math.PI * i) / steps;
    const φ2 = Math.asin(Math.sin(φ1) * Math.cos(r) + Math.cos(φ1) * Math.sin(r) * Math.cos(θ));
    const λ2 = λ1 + Math.atan2(Math.sin(θ) * Math.sin(r) * Math.cos(φ1), Math.cos(r) - Math.sin(φ1) * Math.sin(φ2));
    ring.push([(λ2 * 180) / Math.PI, (φ2 * 180) / Math.PI]);
  }
  return ring;
}

/** Whether the pin's position starts as precise (the photo's own, not ≈). */
export const precisePin = (pin: GuessPin, precise: number) =>
  !!pin.lead.suggestion.place && preciseOffered(pin.lead.suggestion.place, precise);

/** What accepting a pin takes: the levels likely enough (`tick`, 0..1) where the photo has
 *  none, and, when `takePoint` or moved, the position: `at` (moved by the user: then the
 *  photo's own) or the guess's, as precise if asked. A scene's other photos the same, each
 *  keeping names and a position of its own, whether or not the first photo has one. */
export function pinPicks(
  pin: GuessPin, at: [number, number] | null, tick: number, precise: boolean, takePoint = true,
): Accept[] {
  const r = pin.lead;
  const g = r.suggestion.place!;
  const now = r.current?.place ?? null;
  const ticks = placeTicks(g, now, ownPosition(r), tick);
  const chosen: [number, number] | undefined = at ?? (takePoint ? [pin.lat, pin.lon] : undefined);
  const asOwn = !!chosen && (precise || !!at);
  const lead: Accept = {
    row: r.row,
    keywords: [],
    place: takenPlace(g, ticks, guessNames(g)),
    point: ownPosition(r) ? undefined : chosen,
    precise: !ownPosition(r) && asOwn,
    seen: seenOf(r),
  };
  // The scene's others take the position even when the first photo keeps one of its own.
  return scenePicks(lead, r, pin.members, ownPosition, { ...lead, point: chosen, precise: asOwn });
}
