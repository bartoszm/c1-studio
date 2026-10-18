import { test } from "node:test";
import assert from "node:assert/strict";
import { hiddenNote, hides, withHidden } from "../src/lib/hidden";

test("hidden photos are let in only while shown", () => {
  assert.deepEqual(withHidden({ hues: ["red"] }, false), { hues: ["red"] });
  assert.deepEqual(withHidden({ hues: ["red"] }, true), { hues: ["red"], hidden: "Include" });
});

test("h hides, unless every picked photo is hidden already", () => {
  const hidden = new Set([2, 3]);
  assert.equal(hides([1], hidden), true);
  assert.equal(hides([1, 2], hidden), true, "a mix: hide them all");
  assert.equal(hides([2, 3], hidden), false, "all hidden: show them again");
  assert.equal(hides([], hidden), true);
});

test("the note counts the photos", () => {
  assert.equal(hiddenNote(1, true), "1 photo hidden");
  assert.equal(hiddenNote(2, false), "2 photos shown again");
});
