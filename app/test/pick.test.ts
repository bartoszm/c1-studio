import { test } from "node:test";
import assert from "node:assert/strict";
import { inverted } from "../src/lib/pick";

test("inverting picks the other photos, in the result's order", () => {
  assert.deepEqual(inverted([5, 3, 9, 1], new Set([3, 1])), [5, 9]);
});

test("with nothing picked but the focused photo, the rest is picked", () => {
  assert.deepEqual(inverted([5, 3, 9], new Set([9])), [5, 3]);
});

test("with every photo picked there is nothing to invert to", () => {
  assert.equal(inverted([5, 3], new Set([3, 5])), null);
  assert.equal(inverted([], new Set()), null);
});
