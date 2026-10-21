import { test } from "node:test";
import assert from "node:assert/strict";
import type { LogEvent, LogGroup } from "../src/lib/api";
import { dayRange, groupText, kindOf, pickText } from "../src/lib/history";

const photo = { source: "c1", ext: "7" };
const rated: LogEvent = { at: 1, origin: "App", change: { photo, op: { SetRating: { base: null, value: 3 } } } };

function group(over: Partial<LogGroup>): LogGroup {
  return { first: 1, last: 1, at: 1, origin: "App", kind: "rating", entries: 1, photos: 1, sample: rated, row: null, ...over };
}

test("a group of one reads as its entry", () => {
  assert.equal(groupText(group({})), "rating: unrated → ★★★");
});

test("a batch says how many photos it is about", () => {
  assert.equal(groupText(group({ entries: 1240, photos: 1240 })), "rating changed on 1,240 photos");
  assert.equal(groupText(group({ kind: "added", entries: 3, photos: 3 })), "3 photos added");
  assert.equal(groupText(group({ kind: "removed", entries: 2, photos: 2 })), "2 photos removed");
  assert.equal(groupText(group({ kind: "keywords", entries: 2, photos: 1 })), "keywords: 2 changes");
});

test("a kind comes from the entry", () => {
  assert.equal(kindOf(rated), "rating");
  assert.equal(kindOf({ at: 1, origin: "Source", added: { photo, name: "a.raf" } }), "added");
  assert.equal(kindOf({ at: 1, origin: "App", remap: { label: "x" } }), "remap");
  assert.equal(kindOf({ at: 1, origin: "App", change: { photo, op: { AddToCollection: { collection: "1" } } } }), "albums");
});

test("a day is its local midnight to the last millisecond before the next", () => {
  const noon = new Date(2026, 2, 12, 12, 30).getTime();
  const { from, to } = dayRange(noon);
  assert.equal(from, new Date(2026, 2, 12).getTime());
  assert.equal(to, new Date(2026, 2, 13).getTime() - 1);
  // Days through another one.
  assert.equal(dayRange(noon, new Date(2026, 2, 20, 8).getTime()).to, new Date(2026, 2, 21).getTime() - 1);
  // Clocks changing do not make a day 24 hours.
  assert.equal(dayRange(new Date(2026, 9, 25, 12).getTime()).to, new Date(2026, 9, 26).getTime() - 1);
});

test("a pick reads as what it is about and when", () => {
  const { from, to } = dayRange(new Date(2026, 2, 12).getTime(), new Date(2026, 2, 20).getTime());
  const day = (d: number) => new Date(2026, 2, d).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
  const range = `${day(12)} – ${day(20)}`;
  assert.equal(pickText({ Changed: { from, to, what: "Any" } }), `changed, ${range}`);
  assert.equal(pickText({ Changed: { from, to, what: { Field: "rating" } } }), `rating changed, ${range}`);
  assert.equal(pickText({ Changed: { from, to, what: { KeywordAdded: "a|b" } } }), `“a › b” added, ${range}`);
  assert.equal(pickText({ Changed: { from, to: null, what: "Added" } }), `added, since ${day(12)}`);
  assert.equal(pickText({ Changed: { from, to, what: { AlbumIn: "7" } } }, () => "“street”"), `put into “street”, ${range}`);
  assert.equal(pickText({ Changed: { from, to: from + 1000, what: "Any" } }), `changed, ${day(12)}`);
  assert.equal(pickText({ Entries: { first: 4, last: 4, kind: null, origin: null } }), "photos of one log entry");
  assert.equal(pickText({ Entries: { first: 4, last: 9, kind: "rating", origin: "App" } }), "photos of a batch of log entries (rating)");
});
