import { test } from "node:test";
import assert from "node:assert/strict";
import type { Collection } from "../src/lib/api";
import { findAlbums, fitMenu } from "../src/lib/menu";

const window = { width: 1000, height: 700 };

test("a menu opens at the click when it fits", () => {
  assert.deepEqual(fitMenu({ x: 100, y: 200 }, { width: 250, height: 400 }, window), { left: 100, top: 200 });
});

test("a menu near the right or bottom edge moves in", () => {
  assert.deepEqual(fitMenu({ x: 900, y: 600 }, { width: 250, height: 400 }, window), { left: 742, top: 292 });
});

test("a menu taller than the window starts at its top", () => {
  assert.deepEqual(fitMenu({ x: 10, y: 600 }, { width: 250, height: 900 }, window), { left: 10, top: 8 });
});

test("the album finder finds albums only, by any part of the path", () => {
  const c = (id: string, path: string, kind: Collection["kind"] = "Album", system = false): Collection =>
    ({ id, name: path, kind, parent: null, path, rule: null, system });
  const all = [c("1", "Trips/Japan"), c("2", "Best of Japan", "Smart"), c("3", "Trips", "Group"), c("4", "Japan 2024"), c("5", "Japan", "Album", true)];
  assert.deepEqual(findAlbums(all, " japan ").map((a) => a.id), ["1", "4"]);
  assert.deepEqual(findAlbums(all, "").map((a) => a.id), ["1", "4"]);
  assert.deepEqual(findAlbums(all, "", 1).map((a) => a.id), ["1"]);
});
