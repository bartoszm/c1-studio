import { test } from "node:test";
import assert from "node:assert/strict";
import type { Merged, Selection } from "../src/lib/api";
import { followFilter, isWorkList } from "../src/lib/follow";
import { KEPT, KEPT_PICKS, Trail, type Shown } from "../src/lib/navigation";
import { keptPicks, wantedRows } from "../src/lib/batch";

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

test("a rating filter follows the stars given, and only those", () => {
  // ≥ 4 set to 2 is = 2, not ≥ 2: every photo rated 2 to 5 is not the ones edited.
  assert.deepEqual(followFilter({ rating: { Values: { min: 4 } } }, sel({ rating: one(2) }), 2), { rating: { Values: { min: 2, max: 2 } } });
  assert.deepEqual(followFilter({ rating: { Values: { max: 1 } } }, sel({ rating: one(5) }), 2), { rating: { Values: { min: 5, max: 5 } } });
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

const shown = (text: string, picks: number[] = []): Shown => ({
  filter: text ? { text } : {}, sortField: "date", sortReversed: false, match: "all",
  bin: false, hidden: false, tile: null, follow: true, picks,
});

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

test("a rating with an exclusive bound, or a range, follows to the stars given", () => {
  // > 3 does not hold photos set to 3, though ≥ 3 would.
  const above = { rating: { Values: { min: 3, min_exclusive: true } } };
  assert.deepEqual(followFilter(above, sel({ rating: one(3) }), 2), { rating: { Values: { min: 3, max: 3 } } });
  assert.equal(followFilter(above, sel({ rating: one(4) }), 2), null);
  assert.deepEqual(followFilter({ rating: { Values: { min: 4, max: 5 } } }, sel({ rating: one(2) }), 2), { rating: { Values: { min: 2, max: 2 } } });
});

test("some photos with a value and some with none: no filter holds both", () => {
  const mixed = sel({ rating: { same: null, values: 1, empty: 1, top: [[2, 1]] } });
  assert.equal(followFilter({ rating: { Values: { min: 4 } } }, mixed, 2), null);
  const colours = sel({ color: { same: null, values: 1, empty: 3, top: [["Red", 1]] } });
  assert.equal(followFilter({ color: { Values: ["Green"] } }, colours, 4), null);
});

test("a place with a level left out (—) follows by its levels", () => {
  const s = sel({ places: [[place("Poland", null, "Warszawa"), [1, 2]]] });
  const to = followFilter({ place_under: "Poland|—|Kraków" }, s, 2);
  assert.deepEqual(to, { place_under: "Poland|—|Warszawa" });
  // Still under it: nothing to follow.
  const there = sel({ places: [[place("Poland", null, "Kraków"), [1, 2]]] });
  assert.equal(followFilter({ place_under: "Poland|—|Kraków" }, there, 2), null);
});

test("a country and a place picked together both follow", () => {
  const s = sel({ places: [[place("Italy", "Lazio", "Roma"), [1, 2]]] });
  assert.deepEqual(
    followFilter({ country: "Poland", place_under: "Poland|Małopolskie" }, s, 2),
    { country: "Italy", place_under: "Italy|Lazio" },
  );
});

test("of several keywords picked, only the one left follows", () => {
  const s = sel({ keywords: [["animal|cat", 2], ["trip|rome", 2]] });
  assert.deepEqual(
    followFilter({ keywords_under: ["animal|dog", "trip|rome"] }, s, 2, { AddKeywords: ["animal|cat"] }),
    { keywords_under: ["trip|rome", "animal|cat"] },
  );
});

test("a selection kept through a new filter", () => {
  // The focus stays on its photo; the others found stay picked; the rest are counted.
  assert.deepEqual(keptPicks({ picks: [1, 2, 3], focus: 2 }, [[3, 4], [2, 9]]), { kept: [3, 2], at: 9, lost: 1 });
  // The focus gone: the first of those found, by position.
  assert.deepEqual(keptPicks({ picks: [1, 2, 3], focus: 1 }, [[3, 7], [2, 5]]), { kept: [3, 2], at: 5, lost: 1 });
  // None found.
  assert.deepEqual(keptPicks({ picks: [1, 2], focus: 1 }, []), { kept: [], at: null, lost: 2 });
  // Only a focus, nothing picked: the focus is what is looked for.
  assert.deepEqual(wantedRows({ picks: [], focus: 8 }), [8]);
  assert.deepEqual(keptPicks({ picks: [], focus: 8 }, [[8, 0]]), { kept: [8], at: 0, lost: 0 });
  assert.deepEqual(wantedRows({ picks: [] }), []);
});

test("the trail keeps the last fifty filters", () => {
  const t = new Trail();
  for (let i = 0; i <= KEPT + 5; i++) t.leave(shown(`f${i}`), shown(`f${i + 1}`));
  assert.equal(t.back.length, KEPT);
  assert.equal(t.back[0].filter.text, "f6");
});

test("a huge selection is kept as its focus only", () => {
  const t = new Trail();
  const many = Array.from({ length: KEPT_PICKS + 1 }, (_, i) => i);
  t.leave({ ...shown("a", many), focus: 5 }, shown("b"));
  assert.deepEqual(t.back[0].picks, []);
  assert.equal(t.back[0].focus, 5);
});

test("a search typed, or tiles clicked, is one step back", () => {
  const t = new Trail();
  t.leave(shown(""), shown("k"), "search");
  t.leave(shown("k"), shown("kr"), "search");
  t.leave(shown("kr"), shown("kra"), "search");
  assert.deepEqual(t.back, [shown("")]);
  // Something else in between: the next search is a step of its own.
  t.leave(shown("kra"), shown("kra2"));
  t.leave(shown("kra2"), shown("x"), "search");
  assert.equal(t.back.length, 3);
  // Back and forward end a run of them.
  t.goBack(shown("x"));
  t.leave(shown("kra2"), shown("y"), "search");
  assert.equal(t.back.length, 3);
});

test("the trash or hidden photos shown is another view", () => {
  const t = new Trail();
  t.leave(shown("a"), { ...shown("a"), bin: true });
  t.leave({ ...shown("a"), bin: true }, { ...shown("a"), bin: true, hidden: true });
  assert.equal(t.back.length, 2);
});
