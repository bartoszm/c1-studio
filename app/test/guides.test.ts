import { test } from "node:test";
import assert from "node:assert/strict";
import { GUIDES, PHI, fitted, frameFor, guideLines, guidePath, nextGuide, nextTurn, spiral, storedGuide, storedTurn } from "../src/lib/guides";

test("the grids go round, both ways", () => {
  assert.equal(nextGuide("off"), "thirds");
  assert.equal(nextGuide(GUIDES[GUIDES.length - 1]), "off");
  assert.equal(nextGuide("off", true), GUIDES[GUIDES.length - 1]);
  assert.equal(nextGuide("thirds", true), "off");
});

test("a stored grid is read back; anything else is none", () => {
  assert.equal(storedGuide("golden"), "golden");
  assert.equal(storedGuide(null), "off");
  assert.equal(storedGuide("spirals"), "off");
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
  // Each perpendicular meets its diagonal square, at a foot on it.
  for (const [i, d] of [[1, 0], [2, 0], [4, 3], [5, 3]]) {
    assert.ok(Math.abs(dot(lines[d], lines[i])) < 1e-6);
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

test("a stored turn is read back; anything else is 0", () => {
  assert.equal(storedTurn("3"), 3);
  assert.equal(storedTurn("4"), 0);
  assert.equal(storedTurn("1.5"), 0);
  assert.equal(storedTurn(null), 0);
});

test("golden triangles are one diagonal and its two perpendiculars, either way", () => {
  const all = guideLines("diagonals", 300, 200);
  assert.deepEqual(guideLines("triangles", 300, 200, 0), all.slice(0, 3));
  assert.deepEqual(guideLines("triangles", 300, 200, 1), all.slice(3));
  assert.deepEqual(guideLines("triangles", 300, 200, 3), all.slice(3));
});

const near = (a: number, b: number) => Math.abs(a - b) < 1e-6;

test("the spiral's arcs run on from each other, start in a corner and stay in the frame", () => {
  for (const [w, h] of [[300, 200], [200, 300], [400, 400]]) {
    for (let turn = 0; turn < 4; turn++) {
      const { lines, arcs } = spiral(w, h, turn);
      assert.equal(lines.length, arcs.length);
      const [x0, y0] = [arcs[0].x1, arcs[0].y1];
      assert.ok((near(x0, 0) || near(x0, w)) && (near(y0, 0) || near(y0, h)), `corner ${w}x${h} ${turn}`);
      for (let i = 1; i < arcs.length; i++) {
        assert.ok(near(arcs[i].x1, arcs[i - 1].x2) && near(arcs[i].y1, arcs[i - 1].y2));
      }
      for (const a of arcs) {
        for (const [x, y] of [[a.x1, a.y1], [a.x2, a.y2]]) assert.ok(x > -1e-6 && x < w + 1e-6 && y > -1e-6 && y < h + 1e-6);
        // A quarter: the ends are a radius apart along each axis.
        assert.ok(near(Math.abs(a.x2 - a.x1), a.rx) && near(Math.abs(a.y2 - a.y1), a.ry));
      }
    }
  }
});

test("the spiral's first cut is at the golden ratio, along the long side", () => {
  const wide = spiral(1000, 500).lines[0];
  assert.ok(near(wide[0], 1000 / PHI) && near(wide[2], 1000 / PHI));
  const tall = spiral(500, 1000).lines[0];
  assert.ok(near(tall[1], 1000 / PHI) && near(tall[3], 1000 / PHI));
});

test("turning the spiral mirrors it and its winding", () => {
  const a = spiral(300, 200, 0).arcs[0];
  const b = spiral(300, 200, 1).arcs[0];
  assert.ok(near(b.x1, 300 - a.x1) && near(b.y1, a.y1) && b.cw !== a.cw);
  const c = spiral(300, 200, 2).arcs[0];
  assert.ok(near(c.y1, 200 - a.y1) && c.cw !== a.cw);
  assert.equal(spiral(300, 200, 3).arcs[0].cw, a.cw);
});

test("a guide is one path; none draws nothing", () => {
  assert.equal(guidePath("off", 300, 200), "");
  assert.equal((guidePath("thirds", 300, 200).match(/M/g) ?? []).length, 4);
  assert.equal((guidePath("spiral", 300, 200).match(/A/g) ?? []).length, 10);
  assert.equal(guidePath("spiral", 0, 200), "");
});

test("each spiral arc bends round the corner on the spiral's side", () => {
  for (const [w, h] of [[300, 200], [200, 300], [1000, 300]]) {
    for (let turn = 0; turn < 4; turn++) {
      const { arcs } = spiral(w, h, turn);
      const eye = [arcs[arcs.length - 1].x2, arcs[arcs.length - 1].y2];
      for (const a of arcs.slice(0, 6)) {
        // The two corners a quarter ellipse can turn round; the sweep picks the one it
        // goes round clockwise (on screen, y down: a positive cross product) or not.
        const clockwise = (p: number[]) => (a.x1 - p[0]) * (a.y2 - p[1]) - (a.y1 - p[1]) * (a.x2 - p[0]) > 0;
        const [p, q] = [[a.x1, a.y2], [a.x2, a.y1]];
        const [c, other] = clockwise(p) === a.cw ? [p, q] : [q, p];
        const d = (p: number[]) => Math.hypot(p[0] - eye[0], p[1] - eye[1]);
        assert.ok(d(c) < d(other), `${w}x${h} turn ${turn}`);
      }
    }
  }
});

test("o takes the spiral round the corners; the triangles flip between two", () => {
  const corner = (t: number) => { const a = spiral(300, 200, t).arcs[0]; return `${a.x1},${a.y1}`; };
  const seen = [0];
  for (let i = 0; i < 3; i++) seen.push(nextTurn("spiral", seen[seen.length - 1]));
  assert.deepEqual(seen.map(corner), ["0,200", "300,200", "300,0", "0,0"]);
  assert.equal(nextTurn("spiral", 2), 0);
  assert.equal(nextTurn("triangles", 2), 3);
  assert.equal(nextTurn("triangles", 3), 2);
  assert.equal(nextTurn("thirds", 1), 1);
});

test("the guides sit on the image fitted inside the stage's inset", () => {
  assert.equal(frameFor(null, 600, 400, 12), null);
  assert.deepEqual(frameFor([3000, 2000], 624, 624, 12), { left: 12, top: 112, width: 600, height: 400 });
});

test("golden triangles draw three lines", () => {
  assert.equal((guidePath("triangles", 300, 200).match(/M/g) ?? []).length, 3);
});
