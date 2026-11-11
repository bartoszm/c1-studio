import { test } from "node:test";
import assert from "node:assert/strict";
import { backdrop } from "../src/lib/backdrop";
import { neighbourGap } from "../src/lib/around";

const shade = {};
const inside = {};
const at = (target: object) => ({ target, currentTarget: shade }) as unknown as PointerEvent;

test("a click wholly on the backdrop closes the dialog", () => {
  let closed = 0;
  const h = backdrop(() => closed++);
  h.onpointerdown(at(shade));
  h.onclick(at(shade));
  assert.equal(closed, 1);
});

test("a drag from inside the dialog that ends on the backdrop does not", () => {
  let closed = 0;
  const h = backdrop(() => closed++);
  // Selecting a field's text and letting go outside: the click lands on the backdrop.
  h.onpointerdown(at(inside));
  h.onclick(at(shade));
  assert.equal(closed, 0);
  // Nor a click inside after a press on the backdrop, nor a click with no press seen.
  h.onpointerdown(at(shade));
  h.onclick(at(inside));
  h.onclick(at(shade));
  assert.equal(closed, 0);
});

test("neighbour gaps are whole minutes up to two days", () => {
  assert.equal(neighbourGap("45"), 45);
  assert.equal(neighbourGap(" 12.6 "), 13);
  assert.equal(neighbourGap(2880), 2880);
  for (const bad of ["", "  ", "abc", "0", "-5", "2881", null, undefined, NaN, true]) {
    assert.equal(neighbourGap(bad), null, String(bad));
  }
});
