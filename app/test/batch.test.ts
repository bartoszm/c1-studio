import { test } from "node:test";
import assert from "node:assert/strict";
import { collapse, editingNote, mergedField, sameRows, scrollFix } from "../src/lib/batch";

const n = (v: number) => v.toLocaleString();

test("a reply is about the pick only when its rows are the picked ones", () => {
  assert.equal(sameRows([1, 2, 3], new Set([3, 2, 1])), true);
  assert.equal(sameRows([1, 2], new Set([1, 2, 3])), false);
  assert.equal(sameRows([1, 2, 4], new Set([1, 2, 3])), false);
});

test("a field over the selection: the shared value, or how they differ, never one photo's", () => {
  // Not read yet: nothing shown as theirs.
  assert.deepEqual(mergedField(null, 10), { shown: "", placeholder: "…", title: "Reading the selection…" });
  assert.equal(mergedField({ same: "Rynek", values: 1, empty: 0, top: [] }, 10).shown, "Rynek");
  assert.equal(mergedField({ same: null, values: 0, empty: 10, top: [] }, 10).placeholder, "empty on 10");
  // As the Location fields say it: the one value and on how many, or how many values.
  assert.equal(mergedField({ same: null, values: 1, empty: 7, top: [["Rynek", 3]] }, 10).placeholder, "Rynek (3/10)");
  assert.equal(mergedField({ same: null, values: 3, empty: 0, top: [] }, 10).placeholder, "3 values");
  const two = mergedField({ same: null, values: 2, empty: 4, top: [["a", 4], ["b", 2]] }, 10);
  assert.equal(two.placeholder, "2 values (4 empty)");
  // The tooltip lists them, with those left out and the empty ones.
  assert.equal(two.title, "a: 4\nb: 2\n(empty): 4\nType to set it on all 10");
  const many = mergedField({ same: null, values: 30, empty: 0, top: [["a", 9]] }, 20559);
  assert.equal(many.title, `a: 9\n… 29 more\nType to set it on all ${n(20559)}`);
  // A shared position, written as the GPS field writes it.
  const at = mergedField({ same: [50.06, 19.94] as [number, number], values: 1, empty: 0, top: [] }, 2, ([a, b]) => `${a}, ${b}`);
  assert.equal(at.shown, "50.06, 19.94");
});

test("long lists show their first 8, those on only some first", () => {
  const rows = Array.from({ length: 12 }, (_, i) => ({ name: `k${i}`, n: i % 3 ? 10 : 4 }));
  const cut = collapse(rows, (r) => r.n < 10, false);
  assert.equal(cut.shown.length, 8);
  assert.equal(cut.more, 4);
  assert.deepEqual(cut.shown.slice(0, 4).map((r) => r.name), ["k0", "k3", "k6", "k9"]);
  assert.equal(collapse(rows, (r) => r.n < 10, true).shown.length, 12);
  assert.equal(collapse(rows.slice(0, 8), () => false, false).more, 0);
});

test("the footer while an edit runs", () => {
  assert.equal(editingNote("edit", 20559), `Editing ${n(20559)} photos…`);
  assert.equal(editingNote("edit", 1), "Editing 1 photo…");
  assert.equal(editingNote("undo", 10), "Undoing the edit on 10 photos…");
  assert.equal(editingNote("redo", null), "Redoing…");
  assert.equal(editingNote("edit", null), "Editing…");
});

test("the scroll keeps a field in place when what is above it changes", () => {
  assert.equal(scrollFix(300, 420, 100), 220);
  assert.equal(scrollFix(300, 250, 100), 50);
  assert.equal(scrollFix(null, 420, 100), 100);
});
