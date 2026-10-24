import { test } from "node:test";
import assert from "node:assert/strict";
import type { PlaceNames, Review, Suggestion } from "../src/lib/api";
import { aboveOf, guessNames, levelOptions, takenPlace } from "../src/lib/aiPlace";
import { changes, chipText, digest, shows } from "../src/lib/reviews";

const none: PlaceNames = { country: null, country_code: null, state: null, city: null, location: null };
const suggestion: Suggestion = {
  caption_en: "A tram at dusk.", caption_pl: null, headline: "Tram", subject: "tram",
  keywords: ["transport|tram", "street"], keywords_new: ["dusk"], rating: 4, quality: 4, composition: 3, confidence: 0.8,
  place: {
    country: { name: "Poland", p: 0.9 }, state: { name: "Lesser Poland", p: 0.7 }, city: { name: "Kraków", p: 0.8 },
    location: null, country_code: "PL", point: null,
  },
};
const review = (over: Partial<Review>): Review => ({
  seq: 1, row: 7, at: 0, key: "k", model: "m", prompt: 1, status: "Accepted", suggestion, taken: null, ...over,
});

test("corrections: a place renamed, keywords added and taken off, a caption and a rating changed", () => {
  const r = review({
    taken: {
      keywords: ["transport|tram", "night"], caption: "A tram turns at dusk.", headline: "Tram", rating: 3,
      place: { ...none, country: "Poland", state: "Małopolskie" }, point: false,
    },
  });
  const list = changes(r);
  assert.deepEqual(list.map(chipText), [
    "state: Lesser Poland → Małopolskie", "+ night", "− street", "caption rewritten", "★4 → ★3",
  ]);
  assert.equal(
    digest(r),
    `- state "Lesser Poland" changed to "Małopolskie"; keyword "night" added; keyword "street" taken off; ` +
      `caption "A tram at dusk." rewritten as "A tram turns at dusk."; rating 4 changed to 3`,
  );
  assert.equal(shows(r, "edited"), true);
});

test("accepted as it was is no correction; rejected says what was turned down", () => {
  const same = review({
    taken: { keywords: ["transport|tram", "street"], caption: "A tram at dusk.", headline: null, rating: null, place: { ...none, city: "Kraków" }, point: false },
  });
  assert.deepEqual(changes(same), []);
  assert.equal(digest(same), null);
  assert.equal(shows(same, "unchanged"), true);
  const rejected = review({ status: "Rejected" });
  assert.match(digest(rejected)!, /^- Rejected as a whole: headline "Tram"; caption "A tram at dusk."; keywords tram, street, dusk; place Poland › Lesser Poland › Kraków; rating 4$/);
});

test("a card takes its edited names, with the code of the country named", () => {
  const g = suggestion.place!;
  const ticks = { point: false, country: true, state: true, city: false, location: false };
  assert.deepEqual(takenPlace(g, ticks, { ...guessNames(g), state: " Małopolskie " }), {
    country: "Poland", country_code: "PL", state: "Małopolskie",
  });
  // Another country: its own code, or none when it is not known.
  assert.equal(takenPlace(g, ticks, { ...guessNames(g), country: "Czechia" }).country_code, "CZ");
  assert.equal(takenPlace(g, ticks, { ...guessNames(g), country: "Narnia" }).country_code, undefined);
});

const keys: [string, number][] = [
  ["Poland", 90], ["Poland|Małopolskie", 60], ["Poland|Mazowieckie", 30], ["Italy", 10], ["Italy|Lazio", 10],
  ["Poland|Małopolskie|Kraków", 50],
];

test("the catalog's names at a level, those under the levels above first", () => {
  assert.deepEqual(levelOptions(keys, "state", { country: "Italy" }), ["Lazio", "Małopolskie", "Mazowieckie"]);
  assert.deepEqual(levelOptions(keys, "state", {}), ["Małopolskie", "Mazowieckie", "Lazio"]);
  assert.deepEqual(aboveOf(keys, "city", "krakow"), { country: "Poland", state: "Małopolskie" });
  assert.equal(aboveOf(keys, "city", "Rome"), null);
});
