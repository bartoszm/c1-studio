import { test } from "node:test";
import assert from "node:assert/strict";
import { STRIP_DEFAULT, clampStrip, dragged, storedStrip } from "../src/lib/splitter";

test("the strip stays between its least and what leaves the map room", () => {
  assert.equal(clampStrip(10, 900), 80);
  assert.equal(clampStrip(500, 900), 500);
  assert.equal(clampStrip(850, 900), 620);
  assert.equal(clampStrip(Number.NaN, 900), STRIP_DEFAULT);
  // A pane too small for both: the least wins.
  assert.equal(clampStrip(300, 250), 80);
});

test("dragging up makes the strip taller, down shorter", () => {
  assert.equal(dragged(190, 600, 500, 900), 290);
  assert.equal(dragged(190, 600, 700, 900), 90);
  assert.equal(dragged(190, 600, 0, 900), 620);
});

test("a stored height is used only when it is one", () => {
  assert.equal(storedStrip("320"), 320);
  assert.equal(storedStrip("5"), STRIP_DEFAULT);
  assert.equal(storedStrip("junk"), STRIP_DEFAULT);
  assert.equal(storedStrip(null), STRIP_DEFAULT);
});
