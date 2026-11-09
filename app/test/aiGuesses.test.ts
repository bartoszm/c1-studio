import { test } from "node:test";
import assert from "node:assert/strict";
import { circle, guessPins, pinPicks } from "../src/lib/aiGuesses";
import type { PlaceGuess, PlaceNames, SuggestionRow } from "../src/lib/api";

const none: PlaceNames = { country: null, country_code: null, state: null, city: null, location: null };
const place: PlaceGuess = {
  country: { name: "Poland", p: 0.99 },
  state: null,
  city: { name: "Laski", p: 0.96 },
  location: { name: "Piezometr", p: 0.9 },
  country_code: "PL",
  point: { lat: 50.3233, lon: 19.4725, p: 0.95, radius_km: 0.5 },
  basis: "landmark",
};
const row = (n: number, group: number | null, current: Partial<NonNullable<SuggestionRow["current"]>> = {}): SuggestionRow => ({
  row: n, at: 1, provider: "gemini", model: "m", warnings: [], cost: 0, status: "Proposed", group,
  suggestion: {
    caption_en: null, caption_pl: null, headline: null, subject: null, keywords: [], keywords_new: [],
    rating: null, quality: null, composition: null, confidence: 0.9, place,
  },
  current: { caption: null, headline: null, rating: null, keywords: [], place: none, placed: false, approx: false, ...current },
});

test("a scene's guesses make one pin, a photo's its own", () => {
  const pins = guessPins([row(1, 1), row(2, 1), row(3, null), { ...row(4, null), status: "Rejected" }]);
  assert.deepEqual(pins.map((p) => [p.lead.row, p.members.map((m) => m.row)]), [[1, [2]], [3, []]]);
  assert.equal(pins[0].radius_km, 0.5);
});

test("a circle lies its radius from the point", () => {
  const ring = circle(50, 20, 1);
  assert.deepEqual(ring[0], ring[ring.length - 1], "closed");
  for (const [lon, lat] of ring) {
    const dy = (lat - 50) * 111.2;
    const dx = (lon - 20) * 111.2 * Math.cos((50 * Math.PI) / 180);
    assert.ok(Math.abs(Math.hypot(dx, dy) - 1) < 0.01, `${lon}, ${lat}`);
  }
});

test("accepting a pin takes the likely levels and the position, the scene's photos too", () => {
  const [pin] = guessPins([row(1, 1), row(2, 1, { place: { ...none, city: "Kraków" } })]);
  const [lead, other] = pinPicks(pin, null, 0.8, false);
  assert.deepEqual(lead.place, { country: "Poland", country_code: "PL", city: "Laski", location: "Piezometr" });
  assert.deepEqual([lead.point, lead.precise], [[50.3233, 19.4725], false]);
  assert.deepEqual(other.place, { country: "Poland", country_code: "PL" }, "its own city stays, nothing under it");
  assert.deepEqual(other.point, [50.3233, 19.4725]);
});

test("a pin moved by hand is the photo's own position; one with its own GPS keeps it", () => {
  const [pin] = guessPins([row(1, null)]);
  const [moved] = pinPicks(pin, [50.32, 19.47], 0.8, false);
  assert.deepEqual([moved.point, moved.precise], [[50.32, 19.47], true]);
  const [own] = guessPins([row(5, null, { placed: true, approx: false })]);
  const [kept] = pinPicks(own, [50.32, 19.47], 0.8, true);
  assert.deepEqual([kept.point, kept.precise], [undefined, false]);
  const [approx] = guessPins([row(6, null, { placed: true, approx: true })]);
  assert.deepEqual(pinPicks(approx, null, 0.8, true)[0].point, [50.3233, 19.4725], "≈ only: replaced");
});
