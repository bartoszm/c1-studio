import { test } from "node:test";
import assert from "node:assert/strict";
import { PAGE, pagesFor, refreshPages, rowAt } from "../src/lib/pages";

/** Page `n` of a result of `total` photos whose row at position i is `base + i`. */
const page = (n: number, base = 0, total = 3 * PAGE) =>
  Array.from({ length: Math.max(0, Math.min(PAGE, total - n * PAGE)) }, (_, k) => base + n * PAGE + k);
const result = (total: number, base = 0) => ({ total, rows: page(0, base, total) });

/** A promise resolved from outside. */
function later<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}
const settled = () => new Promise((r) => setTimeout(r, 0));

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

test("a refresh reads the pages on screen alongside the first, and is done only when all are in", async () => {
  // Scrolled to about position 1,000 of 1,500, the photo at 1,010 focused.
  const ranges = (total: number) => [[960, 1050], [Math.min(1010, total - 1), Math.min(1010, total - 1) + 2]] as [number, number][];
  const first = later<{ total: number; rows: number[] }>();
  const reads = new Map<number, ReturnType<typeof later<number[]>>>();
  let done = false;
  const pending = refreshPages(first.promise, ranges, 3 * PAGE, (n) => {
    const r = later<number[]>();
    reads.set(n, r);
    return r.promise;
  }).then((x) => { done = true; return x; });
  // Asked for at once, before the first page is in.
  assert.deepEqual([...reads.keys()], [1, 2]);
  first.resolve(result(3 * PAGE, 10_000));
  await settled();
  reads.get(1)!.resolve(page(1, 10_000));
  await settled();
  assert.equal(done, false, "not swapped in while page 2 is still being read");
  reads.get(2)!.resolve(page(2, 10_000));
  const { result: r, pages } = await pending;
  assert.equal(r.total, 3 * PAGE);
  assert.deepEqual([...pages.keys()], [0, 1, 2]);
  assert.equal(rowAt(pages, 0), 10_000);
  assert.equal(rowAt(pages, 1010), 11_010);
});

test("a result that shrank drops pages past its end; one that grew reads the pages it now has", async () => {
  const ranges = (total: number) => [[400, 1100], [Math.min(1050, total - 1), Math.min(1050, total - 1) + 2]] as [number, number][];
  const read = async (n: number) => page(n, 0, 3 * PAGE);
  // 1,500 photos before the edit, 700 after: page 2 is gone, the focus moves to the last one.
  const shrank = await refreshPages(Promise.resolve(result(700)), ranges, 3 * PAGE, read);
  assert.deepEqual([...shrank.pages.keys()], [0, 1]);
  assert.equal(rowAt(shrank.pages, 699), 699);
  assert.equal(rowAt(shrank.pages, 1000), undefined);
  // 700 before, 1,500 after: page 2 is read once the new total is known.
  const asked: number[] = [];
  const grew = await refreshPages(Promise.resolve(result(3 * PAGE)), ranges, 700, async (n) => { asked.push(n); return read(n); });
  assert.deepEqual(asked, [1, 2]);
  assert.equal(rowAt(grew.pages, 1050), 1050);
});
