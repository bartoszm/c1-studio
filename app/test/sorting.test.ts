import { test } from "node:test";
import assert from "node:assert/strict";
import { FIELDS, effectiveField, matchAvailable, restore, toSort, words } from "../src/lib/sorting";

test("every field has both directions, each a different sort, and says what it does", () => {
  const all = FIELDS.flatMap((f) => f.sorts);
  assert.equal(new Set(all).size, all.length, "no sort is used twice");
  for (const f of FIELDS) {
    assert.notEqual(f.first, f.second);
    assert.equal(toSort(f.key, false), f.sorts[0]);
    assert.equal(toSort(f.key, true), f.sorts[1]);
  }
  assert.equal(words("date", false), "Newest first");
  assert.equal(words("rating", true), "Lowest first");
  assert.equal(words("match", false), "Best match first");
});

test("the usual direction is the one a person expects for the field", () => {
  assert.equal(toSort("date", false), "DateDesc");
  assert.equal(toSort("rating", false), "RatingDesc");
  assert.equal(toSort("name", false), "Filename");
  assert.equal(toSort("sharpness", false), "SharpnessDesc");
});

test("best match needs a colour, light or focus pick", () => {
  assert.equal(matchAvailable({}), false);
  assert.equal(matchAvailable({ hues: [] }), false);
  assert.equal(matchAvailable({ hues: ["muted"] }), false);
  assert.equal(matchAvailable({ shape: ["square"] }), false);
  assert.equal(matchAvailable({ hues: ["red", "green"] }), true);
  assert.equal(matchAvailable({ light: ["dark"] }), true);
  assert.equal(matchAvailable({ focus: ["sharp"] }), true);
});

test("without a pick, best match shows as the date; other fields stay", () => {
  assert.equal(effectiveField("match", {}), "date");
  assert.equal(effectiveField("match", { hues: ["red"] }), "match");
  assert.equal(effectiveField("rating", {}), "rating");
});

test("a stored choice comes back; the old dropdown's value is migrated; junk is the default", () => {
  assert.deepEqual(restore(JSON.stringify({ field: "sharpness", reversed: true }), null), { field: "sharpness", reversed: true });
  assert.deepEqual(restore(null, "Date"), { field: "date", reversed: true });
  assert.deepEqual(restore(null, "SharpnessDesc"), { field: "sharpness", reversed: false });
  assert.deepEqual(restore(null, "Brightness"), { field: "brightness", reversed: true });
  assert.deepEqual(restore("{not json", "RatingDesc"), { field: "rating", reversed: false });
  assert.deepEqual(restore(JSON.stringify({ field: "nope" }), "junk"), { field: "date", reversed: false });
  assert.deepEqual(restore(null, null), { field: "date", reversed: false });
});
