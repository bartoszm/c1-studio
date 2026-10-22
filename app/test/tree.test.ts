import { test } from "node:test";
import assert from "node:assert/strict";
import { cityWithoutLocation, placeTree, type Node } from "../src/lib/tree";

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
