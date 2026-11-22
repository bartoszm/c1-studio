import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CULL_DEFAULTS, cullKey, cullSettings, describePick, keeperMark, keepers, restAction, restLabel, restOf, shownRows, stepGroup, stepPhoto,
} from "../src/lib/cull";

const key = (k: string, more: Record<string, unknown> = {}) => ({ key: k, target: null, defaultPrevented: false, ...more });

test("saved settings are read with anything unreadable at its default", () => {
  assert.deepEqual(cullSettings(undefined), CULL_DEFAULTS);
  assert.deepEqual(cullSettings("nonsense"), CULL_DEFAULTS);
  const saved = { rest: "colour", restRating: 9, restColour: "Blue", keeper: "rating", keeperRating: 4, keeperColour: "Mauve" };
  assert.deepEqual(cullSettings(saved), { ...CULL_DEFAULTS, rest: "colour", restColour: "Blue", keeper: "rating", keeperRating: 4 });
  assert.equal(cullSettings({ rest: "burn" }).rest, "trash");
});

test("the rest goes where the settings say, and is hidden where there is no trash", () => {
  const s = CULL_DEFAULTS;
  assert.equal(restAction(s, "MarksTrashed"), "Trash");
  assert.equal(restAction(s, "Files"), "Trash");
  assert.equal(restAction(s, "None"), "Hide");
  assert.equal(restAction(s, undefined), "Trash");
  assert.equal(restAction({ ...s, rest: "hide" }, "Files"), "Hide");
  assert.deepEqual(restAction({ ...s, rest: "rating", restRating: 2 }, "Files"), { Rating: 2 });
  assert.deepEqual(restAction({ ...s, rest: "colour" }, "Files"), { Color: "Red" });
  assert.equal(restAction({ ...s, rest: "nothing" }, "Files"), "Nothing");
  assert.equal(keeperMark(s), "Nothing");
  assert.deepEqual(keeperMark({ ...s, keeper: "rating" }), { Rating: 5 });
  assert.deepEqual(keeperMark({ ...s, keeper: "colour" }), { Color: "Green" });
  assert.equal(restLabel("rating", { ...s, restRating: 2 }), "Rate ★★");
  assert.equal(restLabel("colour", s), "Tag red");
  assert.equal(restLabel("trash", s, "None"), "Hide (no trash here)");
});

test("the active photo and the marked ones are kept, the others are the rest", () => {
  const rows = [10, 11, 12, 13];
  assert.deepEqual(keepers(rows, 12, []), [12]);
  assert.deepEqual(keepers(rows, 12, [13, 10]), [10, 12, 13]);
  assert.deepEqual(keepers(rows, undefined, [11]), [11]);
  assert.deepEqual(keepers(rows, 99, []), [], "a photo not in the group keeps nothing");
  assert.deepEqual(restOf(rows, [10, 12]), [11, 13]);
});

test("a small group is shown whole, a big one four at a time around the active photo", () => {
  assert.deepEqual(shownRows([1, 2, 3], 2), [1, 2, 3]);
  const rows = [1, 2, 3, 4, 5, 6, 7];
  assert.deepEqual(shownRows(rows, 1), [1, 2, 3, 4]);
  assert.deepEqual(shownRows(rows, 6), [3, 4, 5, 6], "moved no further than the active one needs");
  // The photos shown stay while the active one is among them.
  assert.deepEqual(shownRows(rows, 4, [3, 4, 5, 6]), [3, 4, 5, 6]);
  assert.deepEqual(shownRows(rows, 7, [3, 4, 5, 6]), [4, 5, 6, 7]);
  assert.deepEqual(shownRows(rows, 2, [3, 4, 5, 6]), [2, 3, 4, 5]);
  // Two swapped, or one taken out: as the comparison left them.
  assert.deepEqual(shownRows(rows, 4, [4, 3, 6]), [4, 3, 6]);
  // A photo shown that is gone from the group goes; fewer than two start again.
  assert.deepEqual(shownRows([1, 2, 4, 5, 6, 7], 4, [3, 4, 5, 6]), [4, 5, 6]);
  assert.deepEqual(shownRows([1, 2, 5, 6, 7], 5, [3, 5]), [2, 5, 6, 7]);
  // A small group whose photo came back (an undo) is whole again.
  assert.deepEqual(shownRows([1, 2, 3], 1, [1, 3]), [1, 2, 3]);
  assert.deepEqual(shownRows([], undefined), []);
});

test("groups step to the ends and stay, photos step round", () => {
  assert.equal(stepGroup(0, 5, 1), 1);
  assert.equal(stepGroup(4, 5, 1), 4);
  assert.equal(stepGroup(0, 5, -1), 0);
  assert.equal(stepGroup(0, 0, 1), 0);
  assert.equal(stepPhoto([1, 2, 3], 3, 1), 1);
  assert.equal(stepPhoto([1, 2, 3], 1, -1), 3);
  assert.equal(stepPhoto([1, 2, 3], 9, 1), 1);
  assert.equal(stepPhoto([], 1, 1), undefined);
});

test("the keys keep, mark, and move between photos and groups", () => {
  assert.deepEqual(cullKey(key("Enter"), false), { kind: "keep" });
  assert.equal(cullKey(key("Enter", { repeat: true }), false), null, "a held Enter keeps once");
  assert.deepEqual(cullKey(key("k"), false), { kind: "mark" });
  assert.deepEqual(cullKey(key("K"), false), { kind: "mark" });
  assert.deepEqual(cullKey(key("g"), false), { kind: "best" });
  assert.deepEqual(cullKey(key("ArrowDown"), false), { kind: "group", delta: 1 });
  assert.deepEqual(cullKey(key("ArrowUp"), false), { kind: "group", delta: -1 });
  assert.deepEqual(cullKey(key("ArrowLeft"), false), { kind: "photo", delta: -1 });
  assert.deepEqual(cullKey(key("Tab", { shiftKey: true }), false), { kind: "photo", delta: -1 });
  // The comparison's own keys, a dialog's, a field's and the app's are not ours.
  assert.equal(cullKey(key("z"), false), null);
  assert.equal(cullKey(key("Enter"), true), null);
  assert.equal(cullKey(key("k", { ctrlKey: true }), false), null);
  assert.equal(cullKey(key("Enter", { defaultPrevented: true }), false), null);
  assert.equal(cullKey(key("k", { target: { tagName: "SELECT" } }), false), null);
});

test("a pick is said in a line", () => {
  const none = { binned: [], hidden: [], skipped: [] };
  assert.equal(describePick(1, 3, "Trash", { ...none, binned: [1, 2, 3] }), "Kept 1 photo, 3 to the trash");
  assert.equal(describePick(2, 1, "Hide", { ...none, hidden: [4] }), "Kept 2 photos, 1 hidden");
  assert.equal(describePick(1, 2, { Rating: 1 }, none), "Kept 1 photo, 2 rated ★");
  assert.equal(describePick(1, 2, { Color: "Red" }, none), "Kept 1 photo, 2 tagged red");
  assert.equal(describePick(1, 2, "Nothing", none), "Kept 1 photo");
  assert.equal(
    describePick(1, 2, "Trash", { binned: [5], hidden: [], skipped: [6] }),
    "Kept 1 photo, 1 to the trash, 1 left: a variant of a photo kept",
  );
});
