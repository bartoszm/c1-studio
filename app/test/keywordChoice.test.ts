import { test } from "node:test";
import assert from "node:assert/strict";
import { covers, keywordTree, numberName, offeredCount, toggle, type LeftOut } from "../src/lib/keywordChoice";

const KEYWORDS: [string, number][] = [
  ["trips", 0],
  ["trips|2023", 4],
  ["trips|2023|01", 12],
  ["trips|2023|02", 9],
  ["trips|beach", 7],
  ["street", 40],
  ["city|street", 3],
];

test("number names, as c1ai::number_name", () => {
  for (const n of ["01", "2023", "2023-05", "3.", " 7 "]) assert.ok(numberName(n), n);
  for (const n of ["", "--", "01 beach", "b&w", "x2"]) assert.ok(!numberName(n), n);
});

test("a path is left out with all below it; numbers when asked", () => {
  const left: LeftOut = { paths: ["trips|beach", "city"], numbers: true };
  assert.ok(covers("trips|beach", left));
  assert.ok(covers("city|street", left));
  assert.ok(!covers("cityscape", left), "a sibling that starts the same is kept");
  assert.ok(covers("trips|2023|01", left));
  assert.ok(!covers("street", left));
  assert.ok(!covers("trips|2023|01", { paths: [], numbers: false }));
});

test("the tree makes the levels in between and sorts numbers as numbers", () => {
  const tree = keywordTree([["a|b|10", 1], ["a|b|9", 2], ["c", 3]]);
  assert.deepEqual(tree.map((n) => n.name), ["a", "c"]);
  const b = tree[0].children[0];
  assert.equal(b.path, "a|b");
  assert.equal(b.uses, 0);
  assert.deepEqual(b.children.map((n) => n.name), ["9", "10"]);
});

test("offered counts distinct names in use", () => {
  // Names in use: 2023, 01, 02, beach, street (street twice).
  assert.deepEqual(offeredCount(KEYWORDS, { paths: [], numbers: false }), { offered: 5, all: 5 });
  assert.deepEqual(offeredCount(KEYWORDS, { paths: [], numbers: true }), { offered: 2, all: 5 });
  // "street" stays offered through "street" while "city|street" is left out.
  assert.deepEqual(offeredCount(KEYWORDS, { paths: ["city"], numbers: true }), { offered: 2, all: 5 });
});

test("leaving a path out drops the ones below it; offering it again takes it off", () => {
  let left: LeftOut = { paths: ["trips|2023|01"], numbers: false };
  left = toggle(left, "trips", false);
  assert.deepEqual(left.paths, ["trips"]);
  left = toggle(left, "trips", true);
  assert.deepEqual(left.paths, []);
});
