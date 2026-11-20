import { test } from "node:test";
import assert from "node:assert/strict";
import { parseKeywords, pasteAt, withKeywords } from "../src/lib/keywordEntry";

test("one keyword, levels trimmed", () => {
  assert.deepEqual(parseKeywords("  people | family "), { paths: ["people|family"] });
});

test("several, split by semicolons and new lines", () => {
  assert.deepEqual(parseKeywords("travel; people|family\nstreet\r\nbeach;;"), {
    paths: ["travel", "people|family", "street", "beach"],
  });
});

test("nothing typed gives nothing", () => {
  assert.deepEqual(parseKeywords(" ; \n "), { paths: [] });
});

test("repeats are dropped, ignoring case", () => {
  assert.deepEqual(parseKeywords("Travel; travel; TRAVEL|x; travel|X"), { paths: ["Travel", "TRAVEL|x"] });
});

test("an empty level or a comma is an error", () => {
  assert.equal(parseKeywords("a||b").error, "A keyword level is empty.");
  assert.equal(parseKeywords("ok; |b").error, "A keyword level is empty.");
  assert.equal(parseKeywords("a, b").error, "Keyword names cannot contain a comma.");
});

test("more keywords join those already there", () => {
  assert.deepEqual(withKeywords(["a", "B"], ["b", "c", "A", "d"]), ["a", "B", "c", "d"]);
});

test("a bad part keeps the good ones and is handed back as typed", () => {
  assert.deepEqual(parseKeywords("travel; a, b\nstreet; x||y"), {
    paths: ["travel", "street"],
    error: "Keyword names cannot contain a comma.",
    bad: ["a, b", "x||y"],
  });
});

test("a paste goes in at the caret, replacing the selection", () => {
  assert.equal(pasteAt("ab", 1, 1, "X"), "aXb");
  assert.equal(pasteAt("abcd", 1, 3, "X"), "aXd");
  assert.equal(pasteAt("ab", null, null, "X"), "abX");
});
