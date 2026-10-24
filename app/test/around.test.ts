import { test } from "node:test";
import assert from "node:assert/strict";
import { aroundRange, wholeDays, within } from "../src/lib/around";

test("whole days are told from spans within a day", () => {
  const at = new Date(2026, 9, 24, 15, 0).getTime();
  assert.equal(wholeDays(aroundRange(at, "day")), true);
  assert.equal(wholeDays(aroundRange(at, { minutes: 60 })), false);
  assert.equal(wholeDays({ from: new Date(2026, 9, 24).getTime() }), true);
});

test("a span either side of the photo", () => {
  const at = Date.UTC(2026, 9, 24, 12, 0);
  assert.deepEqual(aroundRange(at, { minutes: 60 }), { from: at - 3_600_000, to: at + 3_600_000 });
});

test("the day is the local one, both ends in it", () => {
  const at = new Date(2026, 9, 24, 23, 30).getTime();
  const { from, to } = aroundRange(at, "day");
  assert.equal(from, new Date(2026, 9, 24).getTime());
  assert.equal(to, new Date(2026, 9, 25).getTime() - 1);
});

test("selecting around a photo stays within the filter's own dates", () => {
  assert.deepEqual(within(undefined, { from: 1, to: 9 }), { from: 1, to: 9 });
  assert.deepEqual(within({ from: 5 }, { from: 1, to: 9 }), { from: 5, to: 9 });
  assert.deepEqual(within({ from: 0, to: 4 }, { from: 1, to: 9 }), { from: 1, to: 4 });
});
