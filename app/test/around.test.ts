import { test } from "node:test";
import assert from "node:assert/strict";
import { SESSION_GAP_MAX, aroundRange, dayOf, dayStart, isEmpty, sessionGap, wholeDays, within } from "../src/lib/around";

// Days with a clock change in them: Poland's spring and autumn ones, wherever the tests run.
process.env.TZ = "Europe/Warsaw";
const HOUR = 3_600_000;

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

test("a day with a clock change is 23 or 25 hours, midnight to midnight", () => {
  const spring = aroundRange(new Date(2026, 2, 29, 12).getTime(), "day");
  assert.equal(spring.to! + 1 - spring.from!, 23 * HOUR);
  assert.equal(new Date(spring.to! + 1).getHours(), 0);
  assert.equal(dayOf(spring.to! + 1), "2026-03-30");
  const autumn = aroundRange(new Date(2026, 9, 25, 12).getTime(), "day");
  assert.equal(autumn.to! + 1 - autumn.from!, 25 * HOUR);
  assert.equal(dayOf(autumn.from!), "2026-10-25");
});

test("days of years below 100 are not read as 19xx", () => {
  assert.equal(dayOf(dayStart("0050-06-01")), "0050-06-01");
  assert.equal(dayOf(dayStart("2026-12-31", 1)), "2027-01-01");
});

test("ranges with nothing in common are empty", () => {
  const both = within({ from: 0, to: 4 }, { from: 10, to: 20 });
  assert.equal(isEmpty(both), true);
  assert.equal(isEmpty({ from: 1, to: 1 }), false);
  assert.equal(isEmpty({ from: 5 }), false);
});

test("the gap set is whole minutes from 1 to a day", () => {
  assert.equal(sessionGap("45"), 45);
  assert.equal(sessionGap("2.4"), 2);
  assert.equal(sessionGap("0"), 1);
  assert.equal(sessionGap("-5"), 1);
  assert.equal(sessionGap("99999"), SESSION_GAP_MAX);
  assert.equal(sessionGap(1e300), SESSION_GAP_MAX);
  assert.equal(sessionGap(""), null);
  assert.equal(sessionGap("abc"), null);
  assert.equal(sessionGap(Infinity), null);
  assert.equal(sessionGap(undefined), null);
});
