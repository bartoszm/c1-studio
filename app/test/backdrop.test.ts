import { test } from "node:test";
import assert from "node:assert/strict";
import { backdrop } from "../src/lib/backdrop";
import { neighbourGap } from "../src/lib/around";

const shade = {};
const inside = {};
const at = (target: object) => ({ target, currentTarget: shade }) as unknown as PointerEvent;

/** A press on `from` and a release on `to`: the browser clicks the element holding both,
 *  the backdrop whenever either is on it. */
function gesture(h: ReturnType<typeof backdrop>, from: object, to: object) {
  h.onpointerdown(at(from));
  h.onpointerup(at(to));
  if (from === shade || to === shade) h.onclick(at(shade));
}

test("a click wholly on the backdrop closes the dialog", () => {
  let closed = 0;
  const h = backdrop(() => closed++);
  gesture(h, shade, shade);
  assert.equal(closed, 1);
});

test("a drag between the dialog and the backdrop does not, either way", () => {
  let closed = 0;
  const h = backdrop(() => closed++);
  // Selecting a field's text and letting go outside.
  gesture(h, inside, shade);
  // Pressing outside and letting go inside.
  gesture(h, shade, inside);
  assert.equal(closed, 0);
  // A click with no press seen (the dialog opened under a held button) does not either.
  h.onclick(at(shade));
  assert.equal(closed, 0);
  // And a plain click after all that still closes it.
  gesture(h, shade, shade);
  assert.equal(closed, 1);
});

test("neighbour gaps are whole minutes up to two days", () => {
  assert.equal(neighbourGap("45"), 45);
  assert.equal(neighbourGap(" 12.6 "), 13);
  assert.equal(neighbourGap(2880), 2880);
  for (const bad of ["", "  ", "abc", "0", "-5", "2881", null, undefined, NaN, true]) {
    assert.equal(neighbourGap(bad), null, String(bad));
  }
});
