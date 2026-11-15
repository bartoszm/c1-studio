import { test } from "node:test";
import assert from "node:assert/strict";
import { parseRange, parseSearch, rangeChip, withSearch } from "../src/lib/search.ts";

const near = (a: number | undefined, b: number) => assert.ok(a !== undefined && Math.abs(a - b) < 1e-9, `${a} ≉ ${b}`);

test("tokens set fields and the other words are the free text", () => {
  const p = parseSearch('street camera:x100 lens:"XF 23" make:Fuji "old town" iso:>1600');
  assert.deepEqual(p.filter.camera, "x100");
  assert.deepEqual(p.filter.lens, "XF 23");
  assert.deepEqual(p.filter.make, "Fuji");
  assert.deepEqual(p.filter.iso, { min: 1600, min_exclusive: true });
  assert.equal(p.rest, 'street "old town"');
  assert.equal(p.left, 'street "old town"');
  assert.deepEqual(p.keys, ["camera", "lens", "make", "iso"]);
  assert.deepEqual(p.errors, []);
});

test("aliases and case", () => {
  const p = parseSearch("MODEL:x-pro2 F:2.8 mm:35 ss:1/250 Format:RAF orientation:Portrait megapixels:24");
  assert.equal(p.filter.camera, "x-pro2");
  near(p.filter.aperture?.min, 2.75);
  near(p.filter.aperture?.max, 2.85);
  near(p.filter.focal?.min, 34.5);
  near(p.filter.shutter?.max, 0.004 * 1.02);
  assert.deepEqual(p.filter.ext, ["raf"]);
  assert.deepEqual(p.filter.orientation, ["portrait"]);
  near(p.filter.megapixels?.min, 23.5);
  near(p.filter.megapixels?.max, 24.5);
  assert.equal(p.rest, "");
});

test("unknown keys stay free text, unfinished tokens stay in the box", () => {
  const p = parseSearch("foo:bar iso: lens:");
  assert.equal(p.rest, "foo:bar");
  assert.equal(p.left, "foo:bar iso: lens:");
  assert.deepEqual(p.filter, {});
  assert.deepEqual(p.errors, []);
});

test("bad values are errors, kept in the box but not searched for", () => {
  const p = parseSearch("iso:lots orient:sideways format:j?g -lens:23 dog");
  assert.equal(p.rest, "dog");
  assert.equal(p.left, "iso:lots orient:sideways format:j?g -lens:23 dog");
  assert.deepEqual(
    p.errors.map((e) => e.token),
    ["iso:lots", "orient:sideways", "format:j?g", "-lens:23"],
  );
  assert.match(p.errors[0].message, /iso:>1600/);
});

test("formats: lists, aliases and excludes", () => {
  assert.deepEqual(parseSearch("ext:.jpg,tiff").filter.ext, ["jpg", "jpeg", "tif", "tiff"]);
  assert.ok(parseSearch("format:raw").filter.ext?.includes("raf"));
  const p = parseSearch("-format:jpg ext:raf -orient:pano,sq");
  assert.deepEqual(p.filter.ext_not, ["jpg", "jpeg"]);
  assert.deepEqual(p.filter.ext, ["raf"]);
  assert.deepEqual(p.filter.orientation_not, ["panorama", "square"]);
});

test("ranges: operators, spans and units", () => {
  assert.deepEqual(parseRange("iso", "200-800"), { min: 200, max: 800 });
  assert.deepEqual(parseRange("iso", "800-200"), { min: 200, max: 800 });
  assert.deepEqual(parseRange("iso", "<=400"), { max: 400 });
  assert.deepEqual(parseRange("iso", "=400"), { min: 400, max: 400 });
  assert.deepEqual(parseRange("width", ">=6000"), { min: 6000 });
  assert.equal(parseRange("iso", ">"), undefined);
  assert.equal(parseRange("iso", "abc"), undefined);
  // Sizes: megabytes unless said otherwise. A bound is meant as typed; a value is "about".
  assert.deepEqual(parseRange("size", ">30mb"), { min: 30e6, min_exclusive: true });
  assert.deepEqual(parseRange("size", ">30"), { min: 30e6, min_exclusive: true });
  assert.deepEqual(parseRange("size", "<1.5GB"), { max: 1.5e9, max_exclusive: true });
  assert.deepEqual(parseRange("size", "<=500k"), { max: 500e3 });
  near(parseRange("size", "30")?.min, 30e6 * 0.95);
  near(parseRange("size", "30")?.max, 30e6 * 1.05);
  // Megapixels likewise: `mp:<12` keeps 11.8.
  assert.deepEqual(parseRange("mp", "<12"), { max: 12, max_exclusive: true });
  assert.deepEqual(parseRange("mp", "20-24"), { min: 19.5, max: 24.5 });
  // Shutter: fractions and seconds.
  near(parseRange("shutter", ">=2s")?.min, 2 * 0.98);
  near(parseRange("shutter", '>=2"')?.min, 2 * 0.98);
  near(parseRange("shutter", "1/60-1/30")?.max, (1 / 30) * 1.02);
  assert.equal(parseRange("shutter", "1/0"), undefined);
  // Aperture and focal take their units.
  near(parseRange("aperture", "<f/2.8")?.max, 2.75);
  near(parseRange("focal", "50mm")?.max, 50.5);
});

test("chips say the range as it was typed", () => {
  const chip = (key: Parameters<typeof rangeChip>[0], text: string) => rangeChip(key, parseRange(key, text)!);
  assert.equal(chip("iso", ">1600"), "> 1600");
  assert.equal(chip("iso", "200-800"), "200 – 800");
  assert.equal(chip("aperture", "<=2.8"), "≤ f/2.8");
  assert.equal(chip("aperture", "2.8"), "f/2.8");
  assert.equal(chip("shutter", ">1/30"), "> 1/30 s");
  assert.equal(chip("shutter", ">=2"), "≥ 2 s");
  assert.equal(chip("shutter", "1/250"), "1/250 s");
  assert.equal(chip("size", ">30mb"), "> 30 MB");
  assert.equal(chip("size", "<1.5gb"), "< 1.5 GB");
  assert.equal(chip("focal", "35-50"), "35 mm – 50 mm");
  assert.equal(chip("mp", "<12"), "< 12 MP");
  assert.equal(chip("mp", "24"), "24 MP");
  assert.equal(chip("mp", "20-24"), "20 MP – 24 MP");
  assert.equal(chip("size", "30"), "30 MB");
  assert.equal(chip("width", ">=6000"), "≥ 6000 px");
});

test("tokens replace what the filter had for their keys, and leave the rest", () => {
  const base = { cameras: ["X100VI"], ext: ["jpg"], ext_not: ["tif"], iso: { min: 100 }, text: "old" };
  const f = withSearch(base, parseSearch("format:raf iso:<400"));
  assert.deepEqual(f, { cameras: ["X100VI"], ext: ["raf"], iso: { max: 400, max_exclusive: true }, text: "old" });
});
