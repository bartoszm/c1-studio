import { test } from "node:test";
import assert from "node:assert/strict";
import { eitherPicks, fieldsOf, keptEither, pillText, pills, tidyEither, type Term } from "../src/lib/filterLine.ts";
import { ratingFor } from "../src/lib/criteria.ts";
import { SECTION_KEYS } from "../src/lib/sections.ts";

const order = [...SECTION_KEYS];

test("one pill per section, in the sidebar's order, the search first", () => {
  const terms: Term[] = [
    { key: "text", what: "Search", text: "“tram”" },
    { key: "rating", text: "★ ≥ 4" },
    { key: "cameras", what: "Camera", text: "X-T5" },
    { key: "cameras", what: "Camera", text: "X100VI" },
    { key: "cameras_not", what: "Camera", text: "GFX", out: true },
    { key: "places_under", what: "Place", text: "Poland" },
  ];
  const ps = pills(terms, order, {}, {});
  assert.deepEqual(ps.map((p) => p.id), ["search", "places", "rating", "camera"]);
  assert.deepEqual(ps.map(pillText), [
    "Search: “tram”",
    "Place: Poland",
    "★ ≥ 4",
    "Camera: X-T5 or X100VI except GFX",
  ]);
  assert.deepEqual(ps[3].keys, ["cameras", "cameras_not"]);
  // The value goes along, for a click to leave it out or show it again.
  const gfx: Term = { key: "cameras_not", what: "Camera", text: "GFX 100S", value: "GFX100S", out: true };
  assert.equal(pills([gfx], order, {}, {})[0].outs[0].value, "GFX100S");
});

test("the sidebar's own order decides", () => {
  const terms: Term[] = [
    { key: "rating", text: "★ ≥ 4" },
    { key: "ext", what: "Format", text: "RAF" },
  ];
  assert.deepEqual(pills(terms, ["ext", "rating"], {}, {}).map((p) => p.id), ["ext", "rating"]);
});

test("keywords say and, or or once switched; left out alone says not", () => {
  const terms: Term[] = [
    { key: "keywords_under", what: "Keyword", text: "Food" },
    { key: "keywords_under", what: "Keyword", text: "Kraków" },
  ];
  const f = { keywords_under: ["Food", "Kraków"] };
  const [all] = pills(terms, order, f, {});
  assert.equal(all.parts[0].join, "all");
  assert.equal(all.parts[0].combine, "keywords");
  assert.equal(pillText(all), "Keyword: Food and Kraków");
  assert.equal(pillText(pills(terms, order, f, { keywords: "any" })[0]), "Keyword: Food or Kraków");
  const out = pills([{ key: "keywords_under_not", what: "Keyword", text: "Work", out: true }], order, {}, {});
  assert.equal(pillText(out[0]), "Keyword: not Work");
});

test("fields of one section narrow together; a field's own label stays when it differs", () => {
  const terms: Term[] = [
    { key: "places_under", what: "Place", text: "Poland" },
    { key: "places_under_not", what: "Place", text: "Kraków", out: true },
    { key: "gps", what: "GPS", text: "no position" },
    { key: "month", text: "May 2024" },
    { key: "trip", what: "Trip", text: "Japan" },
  ];
  const ps = pills(terms, order, {}, {});
  assert.equal(pillText(ps[0]), "Date: May 2024 and Trip Japan");
  assert.equal(pillText(ps[1]), "Place: Poland and GPS no position except Kraków");
});

test("fields outside the sidebar keep their own pill, after the sections", () => {
  const terms: Term[] = [
    { key: "iso", what: "ISO", text: "≥ 800" },
    { key: "hues", what: "Main colour", text: "red" },
    { key: "hues", what: "Main colour", text: "blue" },
  ];
  const ps = pills(terms, order, { hues: ["red", "blue"] }, { hues: "all" });
  assert.deepEqual(ps.map((p) => p.id), ["hue", "iso"]);
  assert.equal(pillText(ps[0]), "Main colour: red and blue");
  assert.equal(pillText(ps[1]), "ISO: ≥ 800");
});

const KR = "Poland|Małopolskie|Kraków";

test("a pill's fields: its section's, or its own", () => {
  assert.deepEqual(fieldsOf("hue"), ["hues"]);
  assert.ok(fieldsOf("places").includes("places_under_not"));
  assert.deepEqual(fieldsOf("light"), ["light"]);
});

test("either: Kraków or black & white, and the rest still narrows", () => {
  const f = { places_under: [KR], hues: ["black & white"], rating: ratingFor("≥", 4) };
  assert.deepEqual(eitherPicks(f, ["places", "hue"], {}), {
    rating: ratingFor("≥", 4),
    all_of: [{ any_of: [{ places_under: [KR] }, { hues: ["black & white"] }] }],
  });
  // One alone, or none: no alternatives.
  assert.deepEqual(eitherPicks(f, ["places"], {}), f);
  assert.deepEqual(eitherPicks(f, [], {}), f);
  // A section's own join still holds inside its branch.
  const kw = { keywords_under: ["Food", "Tram"], hues: ["black & white"] };
  assert.deepEqual(eitherPicks(kw, ["keywords", "hue"], { keywords: "any" }).all_of, [
    { any_of: [{ all_of: [{ any_of: [{ keywords_under: ["Food"] }, { keywords_under: ["Tram"] }] }] }, { hues: ["black & white"] }] },
  ]);
});

test("the group keeps pills with picks; one waits for a second", () => {
  const f = { places_under: [KR] };
  assert.deepEqual(tidyEither(f, ["places", "hue"]), ["places"]);
  const same = ["places"];
  assert.equal(tidyEither(f, same), same);
  assert.deepEqual(keptEither(f, ["places"]), []);
});

test("the group's pills go first, marked", () => {
  const terms: Term[] = [
    { key: "rating", text: "★ ≥ 4" },
    { key: "hues", what: "Main colour", text: "black & white" },
    { key: "places_under", what: "Place", text: "Kraków" },
  ];
  const f = { rating: ratingFor("≥", 4), hues: ["black & white"], places_under: [KR] };
  const ps = pills(terms, order, f, {}, ["hue", "places"]);
  assert.deepEqual(ps.map((p) => [p.id, !!p.inEither]), [["places", true], ["hue", true], ["rating", false]]);
});
