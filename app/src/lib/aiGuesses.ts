// AI Locate's guesses on the map: a pin per photo (or per scene) not yet reviewed, with the
// circle its position may be off by, and what accepting one from the map takes.

import type { Accept, PlaceGuess, SuggestionRow, SuggestionStatus } from "./api";
import { guessNames, placeTicks, preciseOffered, radiusText, scenePicks, seenOf, takenPlace } from "./aiPlace";

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

/** The pin a photo's guess is drawn as: its own, or its scene's (any of the scene's photos). */
export function pinOf(pins: GuessPin[], row: number): GuessPin | undefined {
  return pins.find((p) => p.lead.row === row || p.members.some((m) => m.row === row));
}

/** The box a circle (a pin's, or a guess's position) fills, `[[west, south], [east, north]]`,
 *  to fit the map to. */
export function circleBounds(c: { lat: number; lon: number; radius_km: number }): [[number, number], [number, number]] {
  const ring = circle(c.lat, c.lon, c.radius_km);
  const lons = ring.map(([lon]) => lon);
  const lats = ring.map(([, lat]) => lat);
  return [[Math.min(...lons), Math.min(...lats)], [Math.max(...lons), Math.max(...lats)]];
}

/** Where *Show on map* on a locate card takes the map: the guess's pin, when it is one not
 *  reviewed yet (its card opens there); the circle its position may be off by, from the
 *  card itself, when it has a position but no pin (accepted or rejected: no card to open);
 *  or, with no position, the place it names. */
export type GuessAim =
  | { kind: "pin"; pin: GuessPin }
  | { kind: "circle"; bounds: [[number, number], [number, number]]; radius_km: number }
  | { kind: "place" };

export function guessAim(pins: GuessPin[], row: number, place: PlaceGuess | null): GuessAim {
  const pin = pinOf(pins, row);
  if (pin) return { kind: "pin", pin };
  const pt = place?.point;
  if (!pt) return { kind: "place" };
  const radius_km = Math.max(pt.radius_km, MIN_RADIUS_KM);
  return { kind: "circle", bounds: circleBounds({ lat: pt.lat, lon: pt.lon, radius_km }), radius_km };
}

/** What the map's note says of a guess shown from its card: what is shown, and that the
 *  card's photos are picked (*Put them here* and dragging place them). `place` is for a
 *  guess with no position: "found" (the map is at the place it names), "unknown" (the
 *  gazetteer does not know it), "failed" (it could not be looked up) or "none" (no place). */
export function guessNote(
  aim: GuessAim, status: SuggestionStatus, count: number, place: "found" | "unknown" | "failed" | "none" = "none",
): string {
  const picked = count > 1 ? `the card's ${count.toLocaleString()} photos are picked` : "the card's photo is picked";
  switch (aim.kind) {
    case "pin":
      return `The AI's position, with the circle it may be off by and its card open · ${picked}`;
    case "circle": {
      const area = `the area of the AI's position (within ${radiusText(aim.radius_km)})`;
      if (status === "Accepted") {
        const own = count > 1 ? "the photos' own pins show where they are now" : "the photo's own pin shows where it is now";
        return `Accepted: ${area}; ${own} · ${picked}`;
      }
      if (status === "Rejected") return `Rejected: ${area}, not drawn · ${picked}`;
      return `${area[0].toUpperCase()}${area.slice(1)} · ${picked}`;
    }
    case "place": {
      const where = { found: ", the map is at the place it names", unknown: " and its place is not in the gazetteer", failed: " and its place could not be looked up", none: "" }[place];
      return `No AI position${where}: drag the picked photos onto the map, or find a place and put them there · ${picked}`;
    }
  }
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
