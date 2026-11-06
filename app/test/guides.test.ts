import { test } from "node:test";
import assert from "node:assert/strict";
import { GUIDES, fitted, guideLines, nextGuide, storedGuide } from "../src/lib/guides";

test("the grids go round, both ways", () => {
  assert.equal(nextGuide("off"), "thirds");
  assert.equal(nextGuide(GUIDES[GUIDES.length - 1]), "off");
  assert.equal(nextGuide("off", true), GUIDES[GUIDES.length - 1]);
  assert.equal(nextGuide("thirds", true), "off");
});

test("a stored grid is read back; anything else is none", () => {
  assert.equal(storedGuide("golden"), "golden");
  assert.equal(storedGuide(null), "off");
  assert.equal(storedGuide("spiral"), "off");
});

test("thirds, golden ratio and quarters split both sides", () => {
  assert.deepEqual(guideLines("off", 300, 200), []);
  assert.deepEqual(guideLines("thirds", 300, 300), [
    [100, 0, 100, 300], [200, 0, 200, 300], [0, 100, 300, 100], [0, 200, 300, 200],
  ]);
  const golden = guideLines("golden", 1000, 1000).map((l) => Math.round(l[0]));
  assert.deepEqual(golden.slice(0, 2), [382, 618]);
  assert.equal(guideLines("quarters", 400, 400).length, 6);
});

test("the diagonals' perpendiculars meet them at right angles", () => {
  const [w, h] = [300, 200];
  const lines = guideLines("diagonals", w, h);
  assert.equal(lines.length, 6);
  const dot = (a: number[], b: number[]) =>
    (a[2] - a[0]) * (b[2] - b[0]) + (a[3] - a[1]) * (b[3] - b[1]);
  assert.ok(Math.abs(dot(lines[0], lines[2])) < 1e-6);
  assert.ok(Math.abs(dot(lines[0], lines[3])) < 1e-6);
  assert.ok(Math.abs(dot(lines[1], lines[4])) < 1e-6);
  assert.ok(Math.abs(dot(lines[1], lines[5])) < 1e-6);
  // Each foot lies on its diagonal.
  for (const [i, d] of [[2, 0], [3, 0], [4, 1], [5, 1]]) {
    const [, , x, y] = lines[i];
    const [x1, y1, x2, y2] = lines[d];
    assert.ok(Math.abs((x2 - x1) * (y - y1) - (y2 - y1) * (x - x1)) < 1e-6);
  }
});

test("a fitted image is centred on its longer free side", () => {
  assert.deepEqual(fitted(3000, 2000, 600, 600), { left: 0, top: 100, width: 600, height: 400 });
  assert.deepEqual(fitted(2000, 3000, 600, 600), { left: 100, top: 0, width: 400, height: 600 });
  assert.equal(fitted(0, 0, 600, 600), null);
});
