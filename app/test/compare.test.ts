import { test } from "node:test";
import assert from "node:assert/strict";
import { COMPARE_MAX, FIT, clampView, compareKey, compareRows, dropRow, oneToOne, panBy, zoomAt } from "../src/lib/compare";

test("the focused photo is compared first, then the others in the order picked", () => {
  assert.deepEqual(compareRows([5, 9, 2], 2), { rows: [2, 5, 9], left: 0 });
  assert.deepEqual(compareRows([5, 9]), { rows: [5, 9], left: 0 });
  // A focus that is not picked is not compared.
  assert.deepEqual(compareRows([5, 9], 7), { rows: [5, 9], left: 0 });
});

test("at most four photos are compared; the rest are counted", () => {
  const { rows, left } = compareRows([1, 2, 3, 4, 5, 6], 6);
  assert.equal(rows.length, COMPARE_MAX);
  assert.deepEqual(rows, [6, 1, 2, 3]);
  assert.equal(left, 2);
});

test("a photo taken out leaves at least two; the next one becomes active", () => {
  assert.deepEqual(dropRow([1, 2, 3], 1), { rows: [1, 3], active: 1 });
  assert.deepEqual(dropRow([1, 2, 3, 4], 3), { rows: [1, 2, 3], active: 2 });
  assert.equal(dropRow([1, 2], 0), null);
  assert.equal(dropRow([1, 2, 3], 5), null);
});

const key = (k: string, more: object = {}) => ({ key: k, target: { tagName: "BODY" } as unknown as EventTarget, defaultPrevented: false, ...more });

test("the comparison's keys", () => {
  assert.deepEqual(compareKey(key("Escape"), false, 2), { kind: "close" });
  assert.deepEqual(compareKey(key("Tab"), false, 3), { kind: "active", delta: 1 });
  assert.deepEqual(compareKey(key("Tab", { shiftKey: true }), false, 3), { kind: "active", delta: -1 });
  assert.deepEqual(compareKey(key("ArrowLeft"), false, 2), { kind: "active", delta: -1 });
  assert.deepEqual(compareKey(key(" "), false, 2), { kind: "swap" });
  assert.deepEqual(compareKey(key("w"), false, 2), { kind: "mode", mode: "split" });
  assert.deepEqual(compareKey(key("D"), false, 2), { kind: "mode", mode: "difference" });
  assert.deepEqual(compareKey(key("z"), false, 4), { kind: "zoom" });
  assert.deepEqual(compareKey(key("l"), false, 4), { kind: "sync" });
});

test("modes over each other, Swap need two photos; dropping needs three", () => {
  assert.equal(compareKey(key("w"), false, 3), null);
  assert.equal(compareKey(key(" "), false, 3), null);
  assert.deepEqual(compareKey(key("s"), false, 3), { kind: "mode", mode: "side" });
  assert.equal(compareKey(key("x"), false, 2), null);
  assert.deepEqual(compareKey(key("x"), false, 3), { kind: "drop" });
});

test("no keys behind a dialog, in a field, with Ctrl, or handled already; edits pass through", () => {
  assert.equal(compareKey(key("Escape"), true, 2), null);
  assert.equal(compareKey({ ...key("w"), target: { tagName: "INPUT", type: "text" } as unknown as EventTarget }, false, 2), null);
  assert.equal(compareKey(key("z", { ctrlKey: true }), false, 2), null);
  assert.equal(compareKey(key("Escape", { defaultPrevented: true }), false, 2), null);
  for (const k of ["0", "3", "5", "-", "+", "*", "h", "i"]) assert.equal(compareKey(key(k), false, 2), null, k);
});

test("zoom keeps the point under the pointer still, and never below fit", () => {
  const v = zoomAt(FIT, 2, 100, 50, 400, 200);
  assert.deepEqual(v, { s: 2, x: -100, y: -50 });
  // The pane point (100, 50) shows the same image point as before: (p - t) / s.
  assert.equal((100 - v.x) / v.s, 100);
  assert.deepEqual(zoomAt(v, 0.1, 0, 0, 400, 200), FIT);
});

test("panning stops at the photo's edges", () => {
  const v = { s: 2, x: -100, y: -50 };
  assert.deepEqual(panBy(v, 500, 500, 400, 200), { s: 2, x: 0, y: 0 });
  assert.deepEqual(panBy(v, -1000, -1000, 400, 200), { s: 2, x: -400, y: -200 });
  assert.deepEqual(clampView({ s: 1, x: -30, y: 20 }, 400, 200), FIT);
});

test("1:1 is where a preview pixel is a screen pixel, at least 2", () => {
  // A 4000 px wide preview fit into 1000 px: 4×.
  assert.equal(oneToOne(4000, 2000, 1000, 800), 4);
  // On a 2× screen half of that.
  assert.equal(oneToOne(4000, 2000, 1000, 800, 2), 2);
  // A preview smaller than the pane still zooms.
  assert.equal(oneToOne(800, 600, 1000, 800), 2);
  assert.equal(oneToOne(0, 0, 1000, 800), 2);
});
