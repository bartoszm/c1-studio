// AI Locate's guesses on the map: a pin per photo (or per scene) not yet reviewed, with the
// circle its position may be off by, and what accepting one from the map takes.

import type { Accept, SuggestionRow } from "./api";
import { guessNames, memberPick, placeTicks, preciseOffered, takenPlace } from "./aiPlace";

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
    by.set(key, [...(by.get(key) ?? []), r]);
  }
  return [...by.values()].map(([lead, ...members]) => {
    const pt = lead.suggestion.place!.point!;
    return { lead, members, lat: pt.lat, lon: pt.lon, radius_km: pt.radius_km, p: pt.p };
  });
}

/** The photo has a position of its own: a guessed one is never given. */
export const ownPosition = (r: SuggestionRow) => !!r.current?.placed && !r.current?.approx;

/** A circle `km` round a point, as a polygon ring of lon, lat pairs (closed). */
export function circle(lat: number, lon: number, km: number, steps = 64): [number, number][] {
  const r = Math.max(km, 0.05) / 6371;
  const [φ1, λ1] = [(lat * Math.PI) / 180, (lon * Math.PI) / 180];
  const ring: [number, number][] = [];
  for (let i = 0; i <= steps; i++) {
    const θ = (2 * Math.PI * i) / steps;
    const φ2 = Math.asin(Math.sin(φ1) * Math.cos(r) + Math.cos(φ1) * Math.sin(r) * Math.cos(θ));
    const λ2 = λ1 + Math.atan2(Math.sin(θ) * Math.sin(r) * Math.cos(φ1), Math.cos(r) - Math.sin(φ1) * Math.sin(φ2));
    ring.push([(((λ2 * 180) / Math.PI + 540) % 360) - 180, (φ2 * 180) / Math.PI]);
  }
  return ring;
}

/** Whether the pin's position starts as precise (the photo's own, not ≈). */
export const precisePin = (pin: GuessPin, precise: number) =>
  !!pin.lead.suggestion.place && preciseOffered(pin.lead.suggestion.place, precise);

/** What accepting a pin takes: the levels likely enough where the photo has none, and the
 *  position, `at` (moved by the user: then it is the photo's own) or the guess's; a scene's
 *  other photos the same, each keeping names and a position of their own. `tick` and
 *  `precise` are fractions (0..1). */
export function pinPicks(pin: GuessPin, at: [number, number] | null, tick: number, precise: boolean): Accept[] {
  const r = pin.lead;
  const g = r.suggestion.place!;
  const now = r.current?.place ?? null;
  const ticks = placeTicks(g, now, ownPosition(r), tick);
  const seen = (row: SuggestionRow) => ({
    caption: row.current?.caption ?? null,
    headline: row.current?.headline ?? null,
    rating: row.current?.rating ?? null,
    place: row.current?.place ?? { country: null, country_code: null, state: null, city: null, location: null },
  });
  const point: [number, number] | undefined = ownPosition(r) ? undefined : (at ?? [pin.lat, pin.lon]);
  const lead: Accept = {
    row: r.row,
    keywords: [],
    place: takenPlace(g, ticks, guessNames(g)),
    point,
    precise: !!point && (precise || !!at),
    seen: seen(r),
  };
  return [lead, ...pin.members.map((m) => memberPick(lead, m.row, m.current?.place ?? null, ownPosition(m), seen(m)))];
}
