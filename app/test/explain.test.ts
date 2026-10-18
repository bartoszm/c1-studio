import { test } from "node:test";
import assert from "node:assert/strict";
import type { ClassSettings } from "../src/lib/api";
import { HUE_NAMES } from "../src/lib/api";
import { barRule, cardHint, colourLines, focusLine, lightLine, monoLine, nameOf, pct } from "../src/lib/explain";

const lines: ClassSettings = {
  mono_chroma: 4,
  colour_share: 1 / 6,
  colour_floor: 0.05,
  colour_mode: "Covers",
  patch_share: 0.02,
  patch_chroma: 25,
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
const stats = (parts: Record<string, [number, number, number]>) =>
  HUE_NAMES.map((n) => {
    const [share, patch, chroma] = parts[n] ?? [0, 0, 0];
    return { share, patch, chroma };
  });

test("colour names agree with the analysis (c1analysis colour::tests::names)", () => {
  const table: [[number, number, number], string][] = [
    [[10, 10, 12], "black"], [[250, 250, 248], "white"], [[128, 130, 128], "grey"], [[200, 30, 30], "red"],
    [[240, 140, 30], "orange"], [[120, 70, 30], "brown"], [[230, 210, 40], "yellow"], [[50, 160, 60], "green"],
    [[30, 160, 170], "teal"], [[40, 80, 210], "blue"], [[130, 50, 200], "purple"], [[230, 90, 170], "pink"],
  ];
  for (const [rgb, name] of table) assert.equal(nameOf(rgb), name, String(rgb));
});

test("percents keep small shares readable", () => {
  assert.equal(pct(0.19), "19%");
  assert.equal(pct(0.034), "3.4%");
  assert.equal(pct(0.0004), "<0.1%");
  assert.equal(pct(0), "0%");
});

test("colour lines say each colour's measure and whether it counts", () => {
  const s = stats({ red: [0.19, 0.1, 60], blue: [0.09, 0.09, 40] });
  const out = colourLines(s, lines, ["red"]);
  assert.deepEqual(out.map((l) => [l.name, l.counts]), [["red", true], ["blue", false]]);
  assert.equal(out[0].text, "red 19% · counts from 16.7%");
  assert.equal(out[1].text, "blue 9% · needs 16.7%");
  // A black & white photo (a sepia print): its colours do not count, whatever their share.
  const sepia = colourLines(stats({ brown: [0.9, 0.9, 3] }), lines, ["black & white"]);
  assert.equal(sepia[0].text, "brown 90% · not counted: the photo is black & white");
  // A colour that counts is listed even when five bigger ones come first.
  const many = stats({ brown: [0.3, 0.3, 30], orange: [0.25, 0.2, 30], yellow: [0.2, 0.2, 30], teal: [0.18, 0.1, 30], blue: [0.17, 0.1, 30], pink: [0.05, 0.05, 80] });
  const patchy = colourLines(many, { ...lines, colour_mode: "Patch" }, ["pink"]);
  assert.ok(patchy.some((c) => c.name === "pink" && c.counts));
  // No colour reaches the line: the biggest stands for the photo, and says so.
  const small = colourLines(stats({ red: [0.052, 0.04, 50] }), lines, ["red"]);
  assert.equal(small[0].text, "red 5.2% · the biggest colour, counts from 5%");
  const patch = colourLines(s, { ...lines, colour_mode: "Patch" }, ["red", "blue"]);
  assert.equal(patch[0].text, "red patch 10% (chroma 60) · counts from 2%, chroma 25");
});

test("black and white says its chroma, and a vivid patch that overrides it", () => {
  assert.equal(monoLine(2.04, stats({}), lines), "mean chroma 2.0 (black & white below 4)");
  assert.equal(monoLine(2, stats({ red: [0.03, 0.03, 60] }), lines), "mean chroma 2.0 (black & white below 4), but it has a vivid patch");
});

test("light, focus and the card hints follow the lines", () => {
  const a = { brightness: 0.31, highlights: 0.04, shadows: 0.01, contrast: 0.18 } as Parameters<typeof lightLine>[0];
  assert.equal(
    lightLine(a, lines),
    "brightness 31% (dark below 20%, bright above 55%) · clipped 4% (from 3%) · blocked 1% (from 20%) · contrast 0.18 (low below 0.12)",
  );
  assert.equal(focusLine(1240.4, lines), "detail 1,240 (soft below 700, blurry below 300)");
  assert.match(cardHint("hue", lines), /covers 16.7%/);
  assert.match(cardHint("hue", { ...lines, colour_mode: "Patch" }), /vivid patch of 2%/);
  assert.equal(barRule("focus", "soft", lines), "detail 300 to 700");
  assert.equal(barRule("hue", "red", lines), "red counts when it covers 16.7% of the photo or more");
});
