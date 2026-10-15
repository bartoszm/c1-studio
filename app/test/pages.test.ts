import { test } from "node:test";
import assert from "node:assert/strict";
import { PAGE, pagesFor, reloadPages, rowAt } from "../src/lib/pages";

/** Page `page` of a result whose row at position i is `base + i`. */
const page = (n: number, base = 0, total = 3 * PAGE) =>
  Array.from({ length: Math.max(0, Math.min(PAGE, total - n * PAGE)) }, (_, k) => base + n * PAGE + k);

test("rows are read by position across pages", () => {
  const pages = new Map([[0, page(0)], [2, page(2)]]);
  assert.equal(rowAt(pages, 0), 0);
  assert.equal(rowAt(pages, 1200), 1200);
  assert.equal(rowAt(pages, 600), undefined, "page 1 not loaded");
});

test("the pages of the ranges asked for, within the result", () => {
  assert.deepEqual(pagesFor([[900, 1100]], 4305), [1, 2]);
  assert.deepEqual(pagesFor([[0, 70], [1000, 1001]], 4305), [0, 2]);
  assert.deepEqual(pagesFor([[1000, 1100], [1050, 1051]], 4305), [2]);
  // Past the end of a result that shrank: nothing.
  assert.deepEqual(pagesFor([[1000, 1100]], 800), []);
  assert.deepEqual(pagesFor([[400, 600]], 520), [0, 1]);
  assert.deepEqual(pagesFor([[5, 5]], 100), []);
});

test("a refresh keeps the loaded pages readable until the new ones are in", async () => {
  // Scrolled to about position 1,000 of 1,500: pages 0, 1 and 2 loaded.
  const old = new Map([[0, page(0)], [1, page(1)], [2, page(2)]]);
  const asked: number[] = [];
  let answer!: () => void;
  const gate = new Promise<void>((r) => (answer = r));
  const pending = reloadPages([0, page(0, 10_000)], pagesFor([[960, 1030]], 3 * PAGE), async (n) => {
    asked.push(n);
    await gate;
    return page(n, 10_000);
  });
  // While the edit's query runs, the grid and the panel still read the old rows.
  await Promise.resolve();
  assert.equal(rowAt(old, 1000), 1000);
  assert.equal(rowAt(old, 1499), 1499);
  answer();
  const fresh = await pending;
  assert.deepEqual(asked, [1, 2]);
  assert.equal(rowAt(fresh, 0), 10_000);
  assert.equal(rowAt(fresh, 1000), 11_000);
  // The old map was never touched.
  assert.equal(rowAt(old, 1000), 1000);
  assert.deepEqual([...fresh.keys()], [0, 1, 2]);
});

test("a refresh drops pages the shorter result no longer has", async () => {
  const fresh = await reloadPages([0, page(0, 0, 700)], pagesFor([[960, 1030], [600, 601]], 700), async (n) => page(n, 0, 700));
  assert.deepEqual([...fresh.keys()], [0, 1]);
  assert.equal(rowAt(fresh, 699), 699);
  assert.equal(rowAt(fresh, 1000), undefined);
});
