import { test } from "node:test";
import assert from "node:assert/strict";
import { allOfPicks, compile, criteriaCount, cycle, cyclePlace, placePick, ratingFor, tidyPlaces, ratingParts, ratingText } from "../src/lib/criteria.ts";

test("cards set to all ask for every pick; any is left as it is", () => {
  const f = { hues: ["red", "blue"], light: ["dark", "clipped highlights", "low contrast"] };
  assert.deepEqual(allOfPicks(f, {}), f);
  assert.deepEqual(allOfPicks(f, { hues: true }), {
    light: ["dark", "clipped highlights", "low contrast"],
    all_of: [{ hues: ["red"] }, { hues: ["blue"] }],
  });
  assert.deepEqual(allOfPicks(f, { problems: true }), {
    hues: ["red", "blue"],
    light: ["dark"],
    all_of: [{ light: ["clipped highlights"] }, { light: ["low contrast"] }],
  });
  // One pick: nothing to combine.
  assert.deepEqual(allOfPicks({ hues: ["red"] }, { hues: true }), { hues: ["red"] });
  // Alternatives made earlier (Match: any) are kept.
  const any = { hues: ["red", "blue"], all_of: [{ any_of: [{ rating: "Any" as const }] }] };
  assert.equal(allOfPicks(any, { hues: true }).all_of?.length, 3);
});

test("a value cycles through included, excluded and off", () => {
  let s = cycle(undefined, undefined, "a");
  assert.deepEqual(s, { inc: ["a"], exc: undefined });
  s = cycle(s.inc, s.exc, "b");
  assert.deepEqual(s.inc, ["a", "b"]);
  s = cycle(s.inc, s.exc, "a");
  assert.deepEqual(s, { inc: ["b"], exc: ["a"] });
  s = cycle(s.inc, s.exc, "a");
  assert.deepEqual(s, { inc: ["b"], exc: undefined });
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

test("all together leaves the filter alone", () => {
  const f = { rating: ratingFor("≥", 4), keywords_under: ["a", "b"] };
  assert.equal(compile(f, "all"), f);
});

test("any one turns the includes into alternatives and keeps the excludes", () => {
  const f = { rating: ratingFor("≥", 4), keywords_under: ["a", "b"], keywords_under_not: ["c"], year: 2024 };
  const out = compile(f, "any");
  assert.deepEqual(out, {
    keywords_under_not: ["c"],
    year: 2024,
    all_of: [{ any_of: [{ rating: ratingFor("≥", 4) }, { keywords_under: ["a"] }, { keywords_under: ["b"] }] }],
  });
});

test("'none' picks are alternatives too, and one alone is no choice", () => {
  const out = compile({ unkeyworded: true, rating: ratingFor("≥", 4) }, "any");
  assert.deepEqual(out.all_of?.[0].any_of, [{ rating: ratingFor("≥", 4) }, { unkeyworded: true }]);
  const one = { unkeyworded: true };
  assert.equal(compile(one, "any"), one);
  assert.equal(criteriaCount({ unkeyworded: true, collections: ["1", "2"], cameras: ["x"] }), 4);
});

test("places are alternatives: one in a picked place is that place", () => {
  const PL = "Poland";
  const KR = "Poland|Małopolskie|Kraków";
  // Off → in, beside another country.
  assert.deepEqual(cyclePlace(["Germany"], undefined, PL), { inc: ["Germany", PL], exc: undefined });
  // Kraków, then Poland: Poland.
  assert.deepEqual(cyclePlace([KR], undefined, PL), { inc: [PL], exc: undefined });
  // A city's "(no location)" is in it too.
  assert.deepEqual(cyclePlace([`${KR}|—`], undefined, KR), { inc: [KR], exc: undefined });
  // In → out takes what was picked inside it along.
  assert.deepEqual(cyclePlace([PL, "Germany"], [KR], PL), { inc: ["Germany"], exc: [PL] });
  // Out → off.
  assert.deepEqual(cyclePlace(undefined, [PL], PL), { inc: undefined, exc: undefined });
});

test("the deepest pick decides: a place in an included one is left out, in an excluded one taken in", () => {
  const PL = "Poland";
  const KR = "Poland|Małopolskie|Kraków";
  const WA = `${KR}|Wawel`;
  assert.equal(placePick([PL], undefined, KR), "under-in");
  assert.deepEqual(cyclePlace([PL], undefined, KR), { inc: [PL], exc: [KR] });
  // Poland except Kraków keeps the exception when Poland is picked again.
  assert.deepEqual(cyclePlace(undefined, [KR], PL), { inc: [PL], exc: [KR] });
  // Out → off: back to being in Poland.
  assert.deepEqual(cyclePlace([PL], [KR], KR), { inc: [PL], exc: undefined });
  // Poland, except Kraków, but Wawel; a click on Wawel again lets it go with Kraków.
  assert.equal(placePick([PL], [KR], WA), "under-out");
  assert.deepEqual(cyclePlace([PL], [KR], WA), { inc: [PL, WA], exc: [KR] });
  assert.deepEqual(cyclePlace([PL, WA], [KR], WA), { inc: [PL], exc: [KR] });
  // Not Poland, but Kraków.
  assert.deepEqual(cyclePlace(undefined, [PL], KR), { inc: [KR], exc: [PL] });
  // Picks only drop what a pick of their kind says with none of the other between.
  assert.deepEqual(tidyPlaces([PL, WA], [KR]), [PL, WA]);
  assert.deepEqual(tidyPlaces([PL, WA, PL], []), [PL]);
  // The nearest picked place decides; a name that only starts like another is not in it.
  assert.equal(placePick([PL, KR], ["Poland|Małopolskie"], `${KR}|Wawel`), "under-in");
  assert.equal(placePick([PL, KR], ["Poland|Małopolskie"], "Poland|Małopolskie|Tarnów"), "under-out");
  assert.equal(placePick(["Poland|—|Kra"], undefined, "Poland|—|Kraków"), null);
});
