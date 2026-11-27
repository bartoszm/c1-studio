import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CULL_DEFAULTS, afterPick, afterTurn, askCost, askPlanKey, askRecipient, cullKey, cullSettings, describePick, firstUnpicked, keeperMark, keepers,
  isSelection, leftOutNote, lookedAt, placeAfter, restAction, restLabel, restOf, runToggle, selectionId, shownRows, stepGroup, stepPhoto,
  suggestedKeep, unseenNote,
} from "../src/lib/cull";

test("the photos suggested to keep follow the choice, and the ones picked before win", () => {
  const photo = (row: number, focus = "sharp", kept = false) => ({ row, focus, kept }) as never;
  const view = {
    order: [3, 1, 4, 2],
    suggested: [3, 4],
    photos: [photo(1), photo(2), photo(3), photo(4, "blurry")],
  };
  assert.deepEqual(suggestedKeep(view, "best"), [3]);
  assert.deepEqual(suggestedKeep(view, "auto"), [3, 4]);
  // Blurry ones are left out of a count.
  assert.deepEqual(suggestedKeep(view, "two"), [3, 1]);
  assert.deepEqual(suggestedKeep(view, "three"), [3, 1, 2]);
  // What was picked before is what the group suggests, whatever the choice.
  const picked = { ...view, suggested: [2], photos: [photo(1), photo(2, "sharp", true), photo(3), photo(4)] };
  assert.deepEqual(suggestedKeep(picked, "three"), [2]);
  assert.deepEqual(suggestedKeep({ order: [], suggested: [], photos: [] }, "auto"), []);
  assert.deepEqual(suggestedKeep({ ...view, suggested: [] }, "auto"), [3]);
  assert.equal(cullSettings({ suggest: "many" }).suggest, "auto");
  assert.equal(cullSettings({ suggest: "two" }).suggest, "two");
});

test("photos picked by hand are one group named by their rows", () => {
  assert.equal(selectionId([12, 3, 7, 3]), "pick:3,7,12");
  assert.equal(selectionId(new Set([7, 3])), selectionId([3, 7]));
  assert.ok(isSelection("pick:1,2"));
  assert.ok(!isSelection("series:4") && !isSelection(undefined));
  assert.equal(leftOutNote(0), "");
  assert.equal(leftOutNote(2), "2 photos left out: no picture to measure");
});

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
  assert.deepEqual(cullKey(key("r"), false), { kind: "rank", all: false });
  assert.deepEqual(cullKey(key("R", { shiftKey: true }), false), { kind: "rank", all: true });
  assert.equal(cullKey(key("r", { repeat: true }), false), null);
  assert.deepEqual(cullKey(key("a"), false), { kind: "ask", all: false });
  assert.deepEqual(cullKey(key("A", { shiftKey: true }), false), { kind: "ask", all: true });
  assert.equal(cullKey(key("a", { ctrlKey: true }), false), null, "Ctrl+A is the grid's");
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
  // What was done is said, not what was asked: no trash here, so the rest was hidden.
  assert.equal(describePick(1, 2, "Trash", { ...none, hidden: [4, 5] }), "Kept 1 photo, 2 hidden");
  // Variants that went with their image, without being in the group.
  assert.equal(describePick(1, 2, "Trash", { ...none, binned: [1, 2, 7] }), "Kept 1 photo, 3 to the trash (1 of them a variant not shown here)");
  assert.equal(
    describePick(1, 3, "Trash", { binned: [1, 2, 7, 8], hidden: [], skipped: [6] }),
    "Kept 1 photo, 4 to the trash (2 of them variants not shown here), 1 left: a variant of a photo kept",
  );
});

test("going through the groups starts at the first with no pick", () => {
  const list = (...picked: boolean[]) => picked.map((p) => ({ picked: p }));
  assert.equal(firstUnpicked(list(false, false)), 0);
  assert.equal(firstUnpicked(list(true, true, false, true, false)), 2);
  assert.equal(firstUnpicked(list(true, true)), 0, "all picked from: from the first again");
  assert.equal(firstUnpicked([]), 0);
});

test("a pick shows the next group, unless another was gone to meanwhile", () => {
  const groups = ["a", "b", "c"];
  assert.deepEqual(afterPick({ groups, at: 0, done: [] }, "a"), { at: 1, done: ["a"] });
  assert.deepEqual(afterPick({ groups, at: 2, done: ["a"] }, "c"), { at: 2, done: ["a", "c"] }, "the last one stays");
  // Gone on to c while the pick from a was made: c stays.
  assert.deepEqual(afterPick({ groups, at: 2, done: [] }, "a"), { at: 2, done: ["a"] });
  assert.deepEqual(afterPick({ groups, at: 1, done: ["b"] }, "b"), { at: 2, done: ["b"] }, "picked again: done once");
});

test("a pick taken back or made again shows its group", () => {
  const groups = ["a", "b", "c"];
  assert.deepEqual(afterTurn({ groups, at: 2, done: ["a", "b"] }, "a", false), { at: 0, done: ["b"] });
  assert.deepEqual(afterTurn({ groups, at: 2, done: ["b"] }, "a", true), { at: 0, done: ["b", "a"] });
  // A group no longer among them (they were listed again): the place stays.
  assert.deepEqual(afterTurn({ groups, at: 2, done: ["b"] }, "gone", true), { at: 2, done: ["b"] });
});

test("groups listed again keep the place of the one shown", () => {
  assert.equal(placeAfter(["a", "x", "c"], "x", 0), 1, "found by the id its photo has now");
  assert.equal(placeAfter(["a", "c"], "gone", 1), 1, "not among them: the same place");
  assert.equal(placeAfter(["a", "c"], undefined, 5), 1, "as far as the list goes");
  assert.equal(placeAfter([], undefined, 3), 0);
});

test("a run's progress has the group shown read again only when it is the one looked at", () => {
  const before = { done: 4 };
  assert.equal(lookedAt(before, { done: 5, group: "series:7" }, "series:7"), true);
  assert.equal(lookedAt(before, { done: 5, group: "series:9" }, "series:7"), false, "another group: nothing to read");
  assert.equal(lookedAt(before, { done: 4, group: "series:7" }, "series:7"), false, "the same event again");
  assert.equal(lookedAt(null, { done: 1, group: "series:7" }, "series:7"), true);
  assert.equal(lookedAt(null, { done: 0, group: null }, undefined), false, "no group shown");
});

test("a run's key starts or stops its own kind and leaves the other kind alone", () => {
  assert.equal(runToggle(null, false), "start");
  assert.equal(runToggle({ running: false, cloud: true }, false), "start", "the last run is over");
  assert.equal(runToggle({ running: true, cloud: false }, false), "stop");
  assert.equal(runToggle({ running: true, cloud: true }, true), "stop");
  assert.equal(runToggle({ running: true, cloud: false }, true), "other", "Shift+A does not stop the local model's run");
  assert.equal(runToggle({ running: true, cloud: true }, false), "other", "Shift+R does not stop a run that is paid for");
});

test("the question about asking every group takes every key", () => {
  assert.equal(askPlanKey({ key: "Enter" }, false), "go");
  assert.equal(askPlanKey({ key: "Escape" }, false), "cancel");
  assert.equal(askPlanKey({ key: "Escape" }, true), "cancel");
  // Nothing reaches the photos behind it.
  for (const k of ["Delete", "Backspace", "0", "5", "6", "k", "a", "A", "r", "h", "g", "ArrowLeft", "ArrowDown", "z", " "]) {
    assert.equal(askPlanKey({ key: k }, false), "none", k);
  }
  assert.equal(askPlanKey({ key: "z", ctrlKey: true }, false), "none", "nor an undo");
  assert.equal(askPlanKey({ key: "Enter", repeat: true }, false), "none", "a held Enter says yes once");
  assert.equal(askPlanKey({ key: "Enter", ctrlKey: true }, false), "none");
  // Its own buttons are reached and pressed as buttons are: Enter on Cancel is not a yes.
  assert.equal(askPlanKey({ key: "Tab" }, false), "own");
  assert.equal(askPlanKey({ key: "Enter" }, true), "own");
  assert.equal(askPlanKey({ key: " " }, true), "own");
});

test("the cost of asking about every group is said for what it is", () => {
  assert.equal(askRecipient("Claude"), "Anthropic");
  assert.equal(askRecipient("Gemini"), "Google");
  const api = { plan: false, via: "Claude", effort: "low" };
  assert.equal(askCost({ ...api, cost: 0.004 }), "under $0.01");
  assert.equal(askCost({ ...api, cost: 0.04 }), "about $0.04");
  assert.equal(askCost({ ...api, cost: 1234.4 }), "about $1,234");
  assert.match(askCost({ ...api, cost: 100, effort: "high" }), /^about \$100 or more \(at effort high/);
  assert.match(askCost({ ...api, cost: null }), /no price is known/, "not shown as free");
  assert.equal(askCost({ plan: true, via: "Claude", effort: "high", cost: null }), "counted against your Claude plan");
  assert.equal(askCost({ plan: true, via: "Gemini", effort: "low", cost: null }), "counted against your Google account's quota");
});

test("photos the model could not look at are named once it has looked", () => {
  assert.equal(unseenNote(0, 5, true), "");
  assert.equal(unseenNote(5, 5, false), "", "never looked at: every group starts so");
  assert.equal(unseenNote(5, 5, true), "5 photos not looked at", "asked to look, and none could be");
  assert.equal(unseenNote(1, 5, false), "1 photo not looked at", "a run left one out");
});
