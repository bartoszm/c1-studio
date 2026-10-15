import { test } from "node:test";
import assert from "node:assert/strict";
import { emptyNote, groupBadge, groupCount, moreLabel } from "../src/lib/analysis";

test("a group's badge: its size, and how many match when not all", () => {
  assert.equal(groupBadge({ size: 7, matched: 7 }), "7");
  assert.equal(groupBadge({ size: 7, matched: 2 }), "2 of 7");
  assert.equal(groupBadge({ size: 1200, matched: 3 }), `3 of ${(1200).toLocaleString()}`);
});

test("an empty chart says why", () => {
  assert.equal(emptyNote(0, 0), "Nothing matches the filter.");
  assert.equal(emptyNote(4305, 0), `Not measured yet (${(4305).toLocaleString()} photos).`);
  assert.equal(emptyNote(1, 0), "Not measured yet (1 photo).");
  assert.equal(emptyNote(10, 4), "Nothing measured here yet.");
});

test("a list says how many groups there are, and how many are left to show", () => {
  assert.equal(groupCount(1), "1 group");
  assert.equal(groupCount(0), "0 groups");
  assert.equal(groupCount(1234), `${(1234).toLocaleString()} groups`);
  assert.equal(moreLabel(1234, 60), `Show more (${(1174).toLocaleString()} left)`);
});
