import { test } from "node:test";
import assert from "node:assert/strict";
import { SECTION_KEYS, mergeSections, move } from "../src/lib/sections";

const keys = (list: { key: string }[]) => list.map((s) => s.key);

test("nothing stored: every section, in the default order, shown", () => {
  for (const stored of [undefined, null, "junk", {}]) {
    const list = mergeSections(stored);
    assert.deepEqual(keys(list), [...SECTION_KEYS]);
    assert.ok(list.every((s) => !s.hidden));
  }
});

test("the stored order and hidden sections are kept; unknown keys and repeats dropped", () => {
  const stored = [
    { key: "keywords", hidden: false },
    { key: "albums", hidden: true },
    { key: "lenses", hidden: false },
    { key: "keywords", hidden: true },
    ...SECTION_KEYS.filter((k) => k !== "keywords" && k !== "albums").map((key) => ({ key, hidden: false })),
  ];
  const list = mergeSections(stored);
  assert.deepEqual(keys(list).slice(0, 2), ["keywords", "albums"]);
  assert.equal(list.length, SECTION_KEYS.length);
  assert.equal(list.find((s) => s.key === "albums")?.hidden, true);
  assert.equal(list.find((s) => s.key === "keywords")?.hidden, false);
});

test("a section added since goes in after the one it follows by default", () => {
  // Saved before "hue" existed, with "color" moved to the top.
  const stored = ["color", "albums", "date", "places", "folders", "keywords", "rating", "camera"].map((key) => ({ key, hidden: false }));
  assert.deepEqual(keys(mergeSections(stored)), ["color", "hue", "albums", "date", "places", "folders", "keywords", "rating", "camera"]);
  // Nothing before it known: first.
  assert.deepEqual(keys(mergeSections([{ key: "camera", hidden: false }])).slice(0, 2), ["albums", "date"]);
});

test("move", () => {
  assert.deepEqual(move(["a", "b", "c", "d"], 0, 2), ["b", "c", "a", "d"]);
  assert.deepEqual(move(["a", "b", "c", "d"], 3, 0), ["d", "a", "b", "c"]);
  assert.deepEqual(move(["a", "b", "c"], 1, 9), ["a", "c", "b"]);
  assert.deepEqual(move(["a", "b", "c"], 5, 0), ["a", "b", "c"]);
});
