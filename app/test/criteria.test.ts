import { test } from "node:test";
import assert from "node:assert/strict";
import { allOfPicks, compile, criteriaCount, cycle, ratingFor, ratingParts, ratingText } from "../src/lib/criteria.ts";

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
