import { test } from "node:test";
import assert from "node:assert/strict";
import { markTitle, opensVariants } from "../src/lib/variants";

test("v opens the variants of an image that has several", () => {
  assert.ok(opensVariants({ key: "v" }, 2));
  assert.ok(!opensVariants({ key: "v" }, 1), "an image with one variant has nothing to open");
  assert.ok(!opensVariants({ key: "v", repeat: true }, 3), "held, it fires once");
  assert.ok(!opensVariants({ key: "v", ctrlKey: true }, 3), "Ctrl+V pastes");
  assert.ok(!opensVariants({ key: "v", metaKey: true }, 3));
  assert.ok(!opensVariants({ key: "v", altKey: true }, 3));
  assert.ok(!opensVariants({ key: "V", shiftKey: true }, 3));
  assert.ok(!opensVariants({ key: "c" }, 3));
});

test("the mark says how many variants and whether the others are collapsed", () => {
  assert.equal(markTitle(3, false), "One of 3 variants of this image: click or press V to see them all");
  assert.match(markTitle(2, true), /^One of 2 variants of this image, the others collapsed:/);
});
