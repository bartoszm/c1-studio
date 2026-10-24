import { test } from "node:test";
import assert from "node:assert/strict";
import type { Merged, Selection } from "../src/lib/api";
import { followFilter, isWorkList } from "../src/lib/follow";
import { Trail, type Shown } from "../src/lib/navigation";

const none = <T>(): Merged<T> => ({ same: null, values: 0, empty: 2, top: [] });
const one = <T>(v: T): Merged<T> => ({ same: v, values: 1, empty: 0, top: [] });
const place = (country: string, state: string | null, city: string | null, location: string | null = null) =>
  ({ country, state, city, location, country_code: null }) as unknown as Selection["places"][number][0];

function sel(over: Partial<Selection>): Selection {
  return {
    keywords: [], albums: [], places: [], rating: none(), color: none(), geo: none(),
    iptc: {} as Selection["iptc"], ...over,
  };
}

test("a place filter follows two photos moved to another city", () => {
  const s = sel({ places: [[place("Poland", "Mazowieckie", "Warszawa"), [1, 2]]] });
  const f = { place_under: "Poland|Małopolskie|Kraków" };
  assert.deepEqual(followFilter(f, s, 2), { place_under: "Poland|Mazowieckie|Warszawa" });
});

test("a place filter keeps its depth, and only the levels the photos share", () => {
  const s = sel({
    places: [
      [place("Poland", "Mazowieckie", "Warszawa", "Rynek"), [1]],
      [place("Poland", "Mazowieckie", "Radom"), [2]],
    ],
  });
  assert.deepEqual(followFilter({ place_under: "Poland|Małopolskie|Kraków" }, s, 2), { place_under: "Poland|Mazowieckie" });
  // A country picked stays a country.
  const abroad = sel({ places: [[place("Italy", "Lazio", "Roma"), [1, 2]]] });
  assert.deepEqual(followFilter({ place_under: "Poland" }, abroad, 2), { place_under: "Italy" });
});

test("a place cleared, or nothing in common, leaves the filter", () => {
  const cleared = sel({ places: [[place("", null, null) , [1, 2]]] });
  assert.equal(followFilter({ place_under: "Poland|Małopolskie" }, cleared, 2), null);
  const apart = sel({ places: [[place("Poland", null, null), [1]], [place("Italy", null, null), [2]]] });
  assert.equal(followFilter({ place_under: "Poland|Małopolskie" }, apart, 2), null);
});

test("a rating filter follows the stars given, keeping its operator", () => {
  assert.deepEqual(followFilter({ rating: { Values: { min: 4 } } }, sel({ rating: one(2) }), 2), { rating: { Values: { min: 2 } } });
  assert.deepEqual(followFilter({ rating: { Values: { min: 4, max: 4 } } }, sel({ rating: one(3) }), 2), { rating: { Values: { min: 3, max: 3 } } });
  // Two ratings: the range holding both.
  const two = sel({ rating: { same: null, values: 2, empty: 0, top: [[1, 1], [3, 1]] } });
  assert.deepEqual(followFilter({ rating: { Values: { min: 4 } } }, two, 2), { rating: { Values: { min: 1, max: 3 } } });
  // Stars taken off: unrated.
  assert.deepEqual(followFilter({ rating: { Values: { min: 4 } } }, sel({}), 2), { rating: "None" });
});

test("a colour filter follows the colour given", () => {
  assert.deepEqual(followFilter({ color: { Values: ["Red"] } }, sel({ color: one("Green") }), 2), { color: { Values: ["Green"] } });
  assert.deepEqual(followFilter({ color: "None" }, sel({ color: one("Blue") }), 2), { color: { Values: ["Blue"] } });
});

test("a keyword filter follows only to keywords the edit added", () => {
  const s = sel({ keywords: [["animal|cat", 2]] });
  assert.deepEqual(
    followFilter({ keywords_under: ["animal|dog"] }, s, 2, { AddKeywords: ["animal|cat"] }),
    { keywords_under: ["animal|cat"] },
  );
  // Removed and nothing added: the empty filter is what was asked for.
  assert.equal(followFilter({ keywords_under: ["animal|dog"] }, s, 2, { RemoveKeywords: ["animal|dog"] }), null);
});

test("an album filter follows photos put into another album", () => {
  const s = sel({ albums: [["b", 2]] });
  assert.deepEqual(followFilter({ collections: ["a"] }, s, 2, { AddToAlbum: "b" }), { collections: ["b"] });
  assert.equal(followFilter({ collections: ["a"] }, sel({}), 2, { RemoveFromAlbum: "a" }), null);
});

test("work lists, Match any and fields that still match never follow", () => {
  const s = sel({ places: [[place("Poland", "Mazowieckie", "Warszawa"), [1, 2]]], rating: one(5) });
  assert.equal(followFilter({ place_under: "Poland|Małopolskie", unkeyworded: true }, s, 2), null);
  assert.equal(followFilter({ place_under: "Poland|Małopolskie" }, s, 2, undefined, true), null);
  assert.equal(followFilter({ rating: { Values: { min: 4 } } }, s, 2), null);
  assert.equal(followFilter({ place_under: "Poland|Małopolskie" }, s, 0), null);
  for (const f of [{ placed: false }, { located: false }, { gps: false }, { text: "x" }, { keywords_under_not: ["a"] }]) {
    assert.equal(isWorkList(f), true, JSON.stringify(f));
  }
  assert.equal(isWorkList({ placed: true, rating: "None" }), false);
});

const shown = (text: string, picks: number[] = []): Shown =>
  ({ filter: text ? { text } : {}, sortField: "date", sortReversed: false, match: "all", picks });

test("back and forward walk the filters left, with their picks", () => {
  const t = new Trail();
  t.leave(shown(""), shown("a"));
  t.leave(shown("a", [7]), shown("b"));
  assert.deepEqual(t.goBack(shown("b")), shown("a", [7]));
  assert.deepEqual(t.goBack(shown("a")), shown(""));
  assert.equal(t.goBack(shown("")), null);
  assert.deepEqual(t.goForward(shown("")), shown("a"));
  // A new filter forgets what was ahead.
  t.leave(shown("a"), shown("c"));
  assert.equal(t.goForward(shown("c")), null);
});

test("a run that shows the same filter is not a step", () => {
  const t = new Trail();
  t.leave(shown("a", [1]), shown("a", [2]));
  assert.equal(t.back.length, 0);
});
