import { test } from "node:test";
import assert from "node:assert/strict";
import type { ImageStore } from "../src/lib/api";
import {
  addRoot,
  excludesToText,
  gbToMb,
  mbToGb,
  offlineText,
  presetPatterns,
  progressText,
  renderPattern,
  storeMoved,
  textToExcludes,
  withPreset,
} from "../src/lib/folders";

const central: ImageStore = {
  preset: "Central",
  thumb: "{data}/images/{tier}/{hash:2}/{hash}.jpg",
  preview: "{data}/images/{tier}/{hash:2}/{hash}.jpg",
  central_limit_mb: 4096,
};

test("presets bring the patterns the provider uses; Custom has none of its own", () => {
  assert.equal(presetPatterns("Central")?.thumb, "{data}/images/{tier}/{hash:2}/{hash}.jpg");
  assert.equal(presetPatterns("PerRoot")?.preview, "{root}/.c1studio/{tier}/{relpath}.jpg");
  assert.equal(presetPatterns("Beside")?.thumb, "{dir}/.previews/{stem}.{tier}.jpg");
  assert.equal(presetPatterns("Custom"), null);
});

test("switching preset sets the patterns; Custom keeps those there are; the limit stays", () => {
  const beside = withPreset(central, "Beside");
  assert.equal(beside.preset, "Beside");
  assert.equal(beside.thumb, "{dir}/.previews/{stem}.{tier}.jpg");
  assert.equal(beside.central_limit_mb, 4096);
  const custom = withPreset(beside, "Custom");
  assert.deepEqual({ ...custom }, { ...beside, preset: "Custom" });
});

test("only a different preset or pattern moves images", () => {
  assert.equal(storeMoved(central, { ...central, central_limit_mb: 10 }), false);
  assert.equal(storeMoved(central, withPreset(central, "PerRoot")), true);
  assert.equal(storeMoved(central, { ...central, preview: "{data}/p/{hash}.jpg" }), true);
});

test("tokens are replaced for an example photo", () => {
  assert.equal(
    renderPattern("{data}/images/{tier}/{hash:2}/{hash}.jpg", "thumb"),
    "…/work/images/thumb/9f/9f3c1a2b4d5e6f70.jpg",
  );
  assert.equal(
    renderPattern("{root}/.c1studio/{tier}/{relpath}.jpg", "preview"),
    "D:/Photos/Family/.c1studio/preview/2024/Trip/DSCF0001.RAF.jpg",
  );
  assert.equal(
    renderPattern("{dir}/.previews/{stem}.{tier}.jpg", "thumb"),
    "D:/Photos/Family/2024/Trip/.previews/DSCF0001.thumb.jpg",
  );
  assert.equal(renderPattern("{name} {ext} {size}", "preview"), "DSCF0001.RAF RAF 2000");
});

test("an unknown token is left as it was written", () => {
  assert.equal(renderPattern("{nope}/{tier}", "thumb"), "{nope}/thumb");
});

test("excludes: one glob per line, blanks and repeats dropped", () => {
  assert.deepEqual(textToExcludes("**/tmp/**\n\n  *.tmp  \r\n**/tmp/**\n"), ["**/tmp/**", "*.tmp"]);
  assert.deepEqual(textToExcludes(""), []);
  assert.equal(excludesToText(["a", "b"]), "a\nb");
  assert.deepEqual(textToExcludes(excludesToText(["x/**", "*.bak"])), ["x/**", "*.bak"]);
});

test("the size limit goes between megabytes and the gigabytes shown", () => {
  assert.equal(mbToGb(4096), "4");
  assert.equal(mbToGb(1536), "1.5");
  assert.equal(mbToGb(null), "");
  assert.equal(gbToMb("4"), 4096);
  assert.equal(gbToMb("1,5"), 1536);
  assert.equal(gbToMb(""), null);
  assert.equal(gbToMb("0"), null);
  assert.equal(gbToMb("lots"), undefined);
});

test("scan progress reads as a phrase, with the total when it is known", () => {
  const p = { phase: "Scanning folders", done: 1234, total: 5678, note: null };
  assert.equal(progressText(p, "en-US"), "Scanning folders 1,234 / 5,678");
  assert.equal(progressText({ ...p, total: null }, "en-US"), "Scanning folders 1,234");
});

test("a root is added once, with an id nobody else has", () => {
  const ids = ["aaaaaaaa", "aaaaaaaa", "bbbbbbbb"];
  const next = () => ids.shift() as string;
  const one = addRoot([], "D:/Photos", next);
  assert.deepEqual(one, [{ id: "aaaaaaaa", path: "D:/Photos", label: null }]);
  assert.equal(addRoot(one, "D:/Photos/", next), one, "the same folder again changes nothing");
  const two = addRoot(one, "E:/Photos", next);
  assert.deepEqual(two.map((r) => r.id), ["aaaaaaaa", "bbbbbbbb"]);
});

test("offline roots are counted", () => {
  assert.equal(offlineText([]), "");
  assert.equal(offlineText(["Drive"]), "1 root offline");
  assert.equal(offlineText(["A", "B"]), "2 roots offline");
});
