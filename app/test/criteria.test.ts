import { test } from "node:test";
import assert from "node:assert/strict";
import {
  combinePicks, flipJoin, joinOf, keptCombine, modeOf, pick, pickPlace, placePick, ratingFor, tidyPlaces, ratingParts, ratingText,
} from "../src/lib/criteria.ts";

test("cards set to all ask for every pick; any is left as it is", () => {
  const f = { hues: ["red", "blue"], light: ["dark", "clipped highlights", "low contrast"] };
  assert.deepEqual(combinePicks(f, {}), f);
  assert.deepEqual(combinePicks(f, { hues: "all" }), {
    light: ["dark", "clipped highlights", "low contrast"],
    all_of: [{ hues: ["red"] }, { hues: ["blue"] }],
  });
  assert.deepEqual(combinePicks(f, { problems: "all" }), {
    hues: ["red", "blue"],
    light: ["dark"],
    all_of: [{ light: ["clipped highlights"] }, { light: ["low contrast"] }],
  });
  // One pick: nothing to combine.
  assert.deepEqual(combinePicks({ hues: ["red"] }, { hues: "all" }), { hues: ["red"] });
  // Groups made earlier are kept.
  const earlier = { hues: ["red", "blue"], all_of: [{ any_of: [{ rating: "Any" as const }] }] };
  assert.equal(combinePicks(earlier, { hues: "all" }).all_of?.length, 3);
});

test("keywords and albums ask for every pick unless switched to any", () => {
  const f = { keywords_under: ["Food", "Kraków"], keywords_under_not: ["Work"], collections: ["1", "2"], year: 2024 };
  // The index takes several of each as "every one".
  assert.deepEqual(combinePicks(f, {}), f);
  assert.deepEqual(combinePicks(f, { keywords: "any" }), {
    keywords_under_not: ["Work"],
    collections: ["1", "2"],
    year: 2024,
    all_of: [{ any_of: [{ keywords_under: ["Food"] }, { keywords_under: ["Kraków"] }] }],
  });
  assert.deepEqual(combinePicks(f, { albums: "any" }).all_of, [{ any_of: [{ collections: ["1"] }, { collections: ["2"] }] }]);
  // One keyword: no choice to make.
  const one = { keywords_under: ["Food"] };
  assert.equal(combinePicks(one, { keywords: "any" }), one);
});

test("a section's join flips, and goes back to its default under two picks", () => {
  assert.equal(joinOf({}, "keywords"), "all");
  assert.equal(joinOf({}, "hues"), "any");
  const c = flipJoin({}, "keywords");
  assert.deepEqual(c, { keywords: "any" });
  assert.deepEqual(flipJoin(c, "keywords"), {});
  assert.deepEqual(flipJoin({}, "hues"), { hues: "all" });
  const both = { keywords: "any" as const, albums: "any" as const };
  assert.equal(keptCombine({ keywords_under: ["a", "b"], collections: ["1", "2"] }, both), both);
  assert.deepEqual(keptCombine({ keywords_under: ["a", "b"], collections: ["1"] }, both), { keywords: "any" });
});

test("a click picks only the value; again lets go", () => {
  let s = pick(undefined, undefined, "a", "only");
  assert.deepEqual(s, { inc: ["a"], exc: undefined });
  s = pick(s.inc, s.exc, "b", "only");
  assert.deepEqual(s, { inc: ["b"], exc: undefined });
  s = pick(s.inc, s.exc, "b", "only");
  assert.deepEqual(s, { inc: undefined, exc: undefined });
  // Picked among others, or with some left out: a click narrows to it, not off.
  assert.deepEqual(pick(["a", "b"], undefined, "a", "only"), { inc: ["a"], exc: undefined });
  assert.deepEqual(pick(["a"], ["c"], "a", "only"), { inc: ["a"], exc: undefined });
  assert.deepEqual(pick(undefined, ["a"], "a", "only"), { inc: ["a"], exc: undefined });
});

test("Ctrl+click adds or takes away a value; Alt+click leaves it out or lets it back", () => {
  let s = pick(["a"], undefined, "b", "add");
  assert.deepEqual(s, { inc: ["a", "b"], exc: undefined });
  s = pick(s.inc, s.exc, "a", "add");
  assert.deepEqual(s, { inc: ["b"], exc: undefined });
  s = pick(s.inc, s.exc, "b", "out");
  assert.deepEqual(s, { inc: undefined, exc: ["b"] });
  s = pick(s.inc, s.exc, "b", "add");
  assert.deepEqual(s, { inc: ["b"], exc: undefined });
  s = pick(s.inc, s.exc, "c", "out");
  assert.deepEqual(s, { inc: ["b"], exc: ["c"] });
  s = pick(s.inc, s.exc, "c", "out");
  assert.deepEqual(s, { inc: ["b"], exc: undefined });
  assert.equal(modeOf({ ctrlKey: true, metaKey: false, altKey: false }), "add");
  assert.equal(modeOf({ ctrlKey: false, metaKey: true, altKey: false }), "add");
  assert.equal(modeOf({ ctrlKey: false, metaKey: false, altKey: true }), "out");
  assert.equal(modeOf({ ctrlKey: false, metaKey: false, altKey: false }), "only");
});

test("ratings compare", () => {
  assert.deepEqual(ratingFor("≥", 4), { Values: { min: 4 } });
  assert.deepEqual(ratingFor("≤", 2), { Values: { max: 2 } });
  assert.deepEqual(ratingFor("=", 3), { Values: { min: 3, max: 3 } });
  assert.equal(ratingText(ratingFor("≥", 4)), "★ ≥ 4");
  assert.equal(ratingText(ratingFor("≤", 2)), "★ ≤ 2");
  assert.equal(ratingText(ratingFor("=", 3)), "★ = 3");
  assert.deepEqual(ratingParts(ratingFor("≤", 2)), { op: "≤", n: 2 });
  assert.equal(ratingParts("None"), null);
});

test("places are alternatives: one in a picked place is that place", () => {
  const PL = "Poland";
  const KR = "Poland|Małopolskie|Kraków";
  // A click: only that place.
  assert.deepEqual(pickPlace(["Germany"], [KR], PL, "only"), { inc: [PL], exc: undefined });
  // Ctrl+click adds it beside another country.
  assert.deepEqual(pickPlace(["Germany"], undefined, PL, "add"), { inc: ["Germany", PL], exc: undefined });
  // Kraków, then Poland added: Poland.
  assert.deepEqual(pickPlace([KR], undefined, PL, "add"), { inc: [PL], exc: undefined });
  // A city's "(no location)" is in it too.
  assert.deepEqual(pickPlace([`${KR}|—`], undefined, KR, "add"), { inc: [KR], exc: undefined });
  // Leaving out a picked place takes what was left out inside it along.
  assert.deepEqual(pickPlace([PL, "Germany"], [KR], PL, "out"), { inc: ["Germany"], exc: [PL] });
  // Out, and Alt+click again: off.
  assert.deepEqual(pickPlace(undefined, [PL], PL, "out"), { inc: undefined, exc: undefined });
});

test("the deepest pick decides: a place in an included one is left out, in an excluded one taken in", () => {
  const PL = "Poland";
  const KR = "Poland|Małopolskie|Kraków";
  const WA = `${KR}|Wawel`;
  assert.equal(placePick([PL], undefined, KR), "under-in");
  // Poland except Kraków.
  assert.deepEqual(pickPlace([PL], undefined, KR, "out"), { inc: [PL], exc: [KR] });
  // Poland except Kraków keeps the exception when Poland is added.
  assert.deepEqual(pickPlace(undefined, [KR], PL, "add"), { inc: [PL], exc: [KR] });
  // Kraków let back: Poland whole again.
  assert.deepEqual(pickPlace([PL], [KR], KR, "out"), { inc: [PL], exc: undefined });
  // Poland, except Kraków, but Wawel; Wawel added again lets it go with Kraków.
  assert.equal(placePick([PL], [KR], WA), "under-out");
  assert.deepEqual(pickPlace([PL], [KR], WA, "add"), { inc: [PL, WA], exc: [KR] });
  assert.deepEqual(pickPlace([PL, WA], [KR], WA, "add"), { inc: [PL], exc: [KR] });
  // Not Poland, but Kraków.
  assert.deepEqual(pickPlace(undefined, [PL], KR, "add"), { inc: [KR], exc: [PL] });
  // Picks only drop what a pick of their kind says with none of the other between.
  assert.deepEqual(tidyPlaces([PL, WA], [KR]), [PL, WA]);
  assert.deepEqual(tidyPlaces([PL, WA, PL], []), [PL]);
  // The nearest picked place decides; a name that only starts like another is not in it.
  assert.equal(placePick([PL, KR], ["Poland|Małopolskie"], `${KR}|Wawel`), "under-in");
  assert.equal(placePick([PL, KR], ["Poland|Małopolskie"], "Poland|Małopolskie|Tarnów"), "under-out");
  assert.equal(placePick(["Poland|—|Kra"], undefined, "Poland|—|Kraków"), null);
});

test("Shift+click adds as an alternative, and keeps a value already shown", () => {
  assert.equal(modeOf({ ctrlKey: false, metaKey: false, altKey: false, shiftKey: true }), "either");
  assert.deepEqual(pick(["a"], undefined, "b", "either"), { inc: ["a", "b"], exc: undefined });
  assert.deepEqual(pick(["a"], undefined, "a", "either"), { inc: ["a"], exc: undefined });
  assert.deepEqual(pick(undefined, ["a"], "a", "either"), { inc: ["a"], exc: undefined });
  // A place covered by a picked one stays covered.
  assert.deepEqual(pickPlace(["Poland"], undefined, "Poland|Małopolskie|Kraków", "either"), { inc: ["Poland"], exc: undefined });
});
