import { test } from "node:test";
import assert from "node:assert/strict";
import { circle, guessPins, pinPicks, pointTicked } from "../src/lib/aiGuesses";
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

test("a circle across the date line stays one ring; a tiny radius is drawn as the smallest told", () => {
  const ring = circle(-17, 179.99, 5);
  const lons = ring.map(([lon]) => lon);
  assert.ok(Math.max(...lons) > 180 && Math.min(...lons) > 179, "runs on past 180, no jump to -180");
  const [pin] = guessPins([{ ...row(7, null), suggestion: { ...row(7, null).suggestion, place: { ...place, point: { ...place.point!, radius_km: 0 } } } }]);
  assert.equal(pin.radius_km, 0.05);
});

test("a position below the tick is left unless asked for; the scene's others get it past a first photo's own", () => {
  const low = { ...place, point: { ...place.point!, p: 0.5 } };
  const lowRow = (n: number, group: number | null, current = {}) => ({ ...row(n, group, current), suggestion: { ...row(n, group).suggestion, place: low } });
  const [pin] = guessPins([lowRow(1, null)]);
  assert.equal(pointTicked(pin, 0.8), false);
  assert.equal(pinPicks(pin, null, 0.8, false, false)[0].point, undefined);
  assert.deepEqual(pinPicks(pin, null, 0.8, false, true)[0].point, [50.3233, 19.4725], "ticked by hand");
  const [scene] = guessPins([row(1, 1, { placed: true }), row(2, 1)]);
  const [first, other] = pinPicks(scene, null, 0.8, true);
  assert.deepEqual([first.point, first.precise], [undefined, false]);
  assert.deepEqual([other.point, other.precise], [[50.3233, 19.4725], true]);
});
