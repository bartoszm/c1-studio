import { test } from "node:test";
import assert from "node:assert/strict";
import type { ClassSettings } from "../src/lib/api";
import { GROUPS, sameSettings, shown, summary, withField } from "../src/lib/classes";

const defaults: ClassSettings = {
  mono_chroma: 4,
  colour_share: 0.16666667,
  colour_floor: 0.05,
  dark_below: 0.2,
  bright_above: 0.55,
  clipped_share: 0.03,
  blocked_share: 0.2,
  flat_below: 0.12,
  soft_below: 700,
  blurry_below: 300,
  square_ratio: 1.1,
  panorama_ratio: 2,
  duplicate_bits: 3,
  series_bits: 22,
  series_gap_ms: 4000,
  series_min: 3,
};
const field = (key: keyof ClassSettings) => GROUPS.flatMap((g) => g.fields).find((f) => f.key === key)!;

test("every setting has a field, once", () => {
  const keys = GROUPS.flatMap((g) => g.fields.map((f) => f.key)).sort();
  assert.deepEqual(keys, Object.keys(defaults).sort());
});

test("shares show as percent and the wait as seconds, without float noise", () => {
  assert.equal(shown(field("colour_share"), defaults), 16.667);
  assert.equal(shown(field("clipped_share"), defaults), 3);
  assert.equal(shown(field("series_gap_ms"), defaults), 4);
});

test("typing converts back, stays in range, and ignores what is not a number", () => {
  assert.equal(withField(defaults, field("colour_share"), 25).colour_share, 0.25);
  assert.equal(withField(defaults, field("colour_share"), 250).colour_share, 1);
  assert.equal(withField(defaults, field("series_gap_ms"), 2.5).series_gap_ms, 2500);
  assert.equal(withField(defaults, field("duplicate_bits"), 9).duplicate_bits, 3);
  assert.equal(withField(defaults, field("soft_below"), null), defaults);
  assert.equal(withField(defaults, field("soft_below"), NaN), defaults);
});

test("settings are the same despite float noise", () => {
  assert.ok(sameSettings(defaults, { ...defaults, colour_share: 1 / 6 }));
  assert.ok(!sameSettings(defaults, { ...defaults, soft_below: 701 }));
});

test("the preview lists the classes that have photos; colours biggest first", () => {
  const preview = {
    measured: 100,
    counts: { "focus|soft": 30, "focus|blurry": 5, "hue|red": 10, "hue|blue": 40, "hue|muted": 2 },
    duplicates: 4,
    series: 9,
  };
  const group = (title: string) => GROUPS.find((g) => g.title === title)!;
  assert.equal(summary(group("Focus"), preview), "30 soft · 5 blurry");
  assert.equal(summary(group("Colour"), preview), "2 muted · 40 blue · 10 red");
  assert.equal(summary(group("Alike"), preview), "4 in duplicate groups · 9 in series");
  assert.equal(summary(group("Focus"), null), "");
});
