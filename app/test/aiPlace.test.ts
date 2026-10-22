import { test } from "node:test";
import assert from "node:assert/strict";
import { placeReplaces, placeTicks, takenPlace } from "../src/lib/aiPlace";
import type { PlaceGuess, PlaceNames } from "../src/lib/api";

const guess: PlaceGuess = {
  country: { name: "Poland", p: 0.95 },
  state: { name: "Lesser Poland", p: 0.7 },
  city: { name: "Kraków", p: 0.85 },
  location: null,
  country_code: "PL",
  point: { lat: 50.06, lon: 19.94, p: 0.8, radius_km: 5 },
};
const none: PlaceNames = { country: null, country_code: null, state: null, city: null, location: null };

test("levels likely enough start ticked where the photo has none", () => {
  const ticks = placeTicks(guess, { ...none, country: "Polska" }, false, 0.8);
  assert.deepEqual(ticks, { point: true, country: false, state: false, city: true, location: false });
  assert.equal(placeTicks(guess, none, true, 0.8).point, false, "never over a position");
});

test("accepting takes the ticked levels, the country with its code", () => {
  const ticks = { point: false, country: true, state: false, city: true, location: false };
  assert.deepEqual(takenPlace(guess, ticks), { country: "Poland", country_code: "PL", city: "Kraków" });
  const city = { ...ticks, country: false };
  assert.deepEqual(takenPlace(guess, city), { city: "Kraków" });
});

test("a ticked level over another name is told", () => {
  const ticks = { point: false, country: true, state: false, city: true, location: false };
  assert.deepEqual(placeReplaces(guess, ticks, { ...none, country: "Poland", city: "Wieliczka" }), ["city"]);
});
