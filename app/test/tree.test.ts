import { test } from "node:test";
import assert from "node:assert/strict";
import { byCount, cityWithoutLocation, hideRare, pathTree, placeTree, usesWithin, type Node } from "../src/lib/tree";

const FACET: [string, number][] = [
  ["Japan", 10],
  ["Japan|Kanto", 10],
  ["Japan|Kanto|Tokyo", 8],
  ["Japan|Kanto|Tokyo|Shibuya", 3],
  ["Japan|Kanto|Tokyo|Asakusa", 2],
  ["Japan|Kanto|Yokohama", 2],
  ["Poland", 4],
  ["Poland|—", 4],
  ["Poland|—|Kraków", 4],
  ["Poland|—|Kraków|Rynek", 4],
];

const find = (nodes: Node[], value: string): Node | undefined => {
  for (const n of nodes) {
    if (n.value === value) return n;
    const deeper = find(n.children, value);
    if (deeper) return deeper;
  }
};

test("a city with locations lists the photos without one first", () => {
  const tree = placeTree(FACET);
  const tokyo = find(tree, "Japan|Kanto|Tokyo")!;
  assert.deepEqual(tokyo.children.map((c) => [c.name, c.count]), [["(no location)", 3], ["Asakusa", 2], ["Shibuya", 3]]);
  assert.equal(tokyo.children[0].value, "Japan|Kanto|Tokyo|—");
  // No locations at all: the city is that already. Every photo with one: nothing left.
  assert.deepEqual(find(tree, "Japan|Kanto|Yokohama")!.children, []);
  assert.deepEqual(find(tree, "Poland|—|Kraków")!.children.map((c) => c.name), ["Rynek"]);
  assert.equal(find(tree, "Poland|—")!.name, "(no state)");
});

test("the entry stands for its city", () => {
  assert.equal(cityWithoutLocation("Japan|Kanto|Tokyo|—"), "Japan|Kanto|Tokyo");
  assert.equal(cityWithoutLocation("Japan|Kanto|Tokyo|Shibuya"), null);
  assert.equal(cityWithoutLocation("Poland|—"), null);
});

const KEYWORDS: [string, number][] = [
  ["trips", 30],
  ["trips|2023", 20],
  ["trips|2023|beach", 2],
  ["trips|alps", 10],
  ["cats", 40],
  ["zebra", 1],
];

const names = (nodes: Node[]): unknown[] => nodes.map((n) => (n.children.length ? [n.name, names(n.children)] : n.name));

test("keywords by name, or most counted first at every level", () => {
  assert.deepEqual(names(pathTree(KEYWORDS, "|", "k:", "keyword")), [
    "cats",
    ["trips", [["2023", ["beach"]], "alps"]],
    "zebra",
  ]);
  assert.deepEqual(names(pathTree(KEYWORDS, "|", "k:", "keyword", byCount)), [
    "cats",
    ["trips", [["2023", ["beach"]], "alps"]],
    "zebra",
  ]);
  const tied: [string, number][] = [["b", 5], ["a", 5], ["c", 9], ["c|y", 1], ["c|x", 3]];
  assert.deepEqual(names(pathTree(tied, "|", "k:", "keyword", byCount)), [["c", ["x", "y"]], "a", "b"]);
});

test("uses of a keyword add up those under it", () => {
  const uses = usesWithin([["trips", 0], ["trips|2023", 4], ["trips|2023|01", 12], ["street", 40], ["city|street", 3]]);
  assert.equal(uses.get("trips"), 16);
  assert.equal(uses.get("trips|2023"), 16);
  assert.equal(uses.get("trips|2023|01"), 12);
  assert.equal(uses.get("city"), 3);
  assert.equal(uses.get("street"), 40);
});

test("rarely used keywords are hidden, with what is under them", () => {
  const tree = pathTree(KEYWORDS, "|", "k:", "keyword");
  const uses = new Map(KEYWORDS);
  const { nodes, hidden } = hideRare(tree, uses, 5);
  assert.deepEqual(names(nodes), ["cats", ["trips", ["2023", "alps"]]]);
  assert.equal(hidden, 2);
  // Off: nothing hidden.
  assert.deepEqual(hideRare(tree, uses, 0), { nodes: tree, hidden: 0 });
});

test("a kept keyword stays, with the branch above it", () => {
  const tree = pathTree([["a", 1], ["a|b", 1], ["a|b|c", 1], ["d", 9]], "|", "k:", "keyword");
  const uses = new Map<string, number>([["a", 1], ["a|b", 1], ["a|b|c", 1], ["d", 9]]);
  const { nodes, hidden } = hideRare(tree, uses, 5, (n) => n.value === "a|b");
  assert.deepEqual(names(nodes), [["a", ["b"]], "d"]);
  assert.equal(hidden, 1);
});

test("a keyword added since the uses were counted is not hidden", () => {
  const tree = pathTree([["old", 1], ["new", 1], ["d", 9]], "|", "k:", "keyword");
  const uses = new Map<string, number>([["old", 1], ["d", 9]]);
  const { nodes, hidden } = hideRare(tree, uses, 5);
  assert.deepEqual(new Set(names(nodes)), new Set(["new", "d"]));
  assert.equal(hidden, 1);
});
