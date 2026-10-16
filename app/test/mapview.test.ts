import { test } from "node:test";
import assert from "node:assert/strict";
import { hitView, mapPadding, parseHits, placeZoom, routeStep, shouldFollow, viewBox, wrapLon, type NominatimRow } from "../src/lib/mapview";

// The map in #21: 892×543.
const SIZE = { width: 892, height: 543 };

// Nominatim's rows. Tokyo's box and point as quoted in #21; the app shows it at zoom 10
// (measured on Windows with tools/record/checks/21.mjs), so its kind is a city's. France's
// and Fiji's as that check printed them.
const TOKYO: NominatimRow = {
  display_name: "Tokyo, Japan", lat: "35.6768601", lon: "139.7638947",
  boundingbox: ["20.2145811", "35.8984245", "135.8536855", "154.2055410"], addresstype: "city",
};
const FRANCE: NominatimRow = {
  display_name: "France", lat: "46.603354", lon: "1.8883335",
  boundingbox: ["-50.2187169", "51.3055721", "-178.3873749", "172.3057152"], place_rank: 4, addresstype: "country",
};
const FIJI: NominatimRow = {
  display_name: "Fiji", lat: "-18.1239696", lon: "179.0122737",
  boundingbox: ["-21.9434274", "-12.2613866", "-180.0000000", "180.0000000"], place_rank: 4, addresstype: "country",
};
// Approximate: a city whose point is near the middle of its box. (The app showed Kraków at
// zoom 10.34 on Windows.)
const KRAKOW: NominatimRow = {
  display_name: "Kraków, Lesser Poland Voivodeship, Poland", lat: "50.0619474", lon: "19.9368564",
  boundingbox: ["49.9676668", "50.1261338", "19.7922355", "20.2173455"], place_rank: 16, addresstype: "city",
};

test("hits carry their box (west, south, east, north), rank and kind", () => {
  const [tokyo, france] = parseHits([TOKYO, FRANCE]);
  assert.deepEqual(tokyo.box, [135.8536855, 20.2145811, 154.205541, 35.8984245]);
  assert.equal(tokyo.lat, 35.6768601);
  assert.equal(france.rank, 4);
  assert.equal(france.type, "country");
  assert.equal(parseHits([{ display_name: "x", lat: "1", lon: "2" }])[0].box, null);
});

test("Tokyo: centred on Tokyo, not on its islands' box, at city zoom", () => {
  const v = hitView(parseHits([TOKYO])[0], SIZE);
  assert.deepEqual(v.center, [139.7638947, 35.6768601]);
  assert.equal(v.zoom, 10);
});

test("France: the mainland in the middle, at country zoom", () => {
  const v = hitView(parseHits([FRANCE])[0], SIZE);
  assert.deepEqual(v.center, [1.8883335, 46.603354]);
  assert.equal(v.zoom, 5);
});

test("Fiji: a box round the world is no guide", () => {
  const v = hitView(parseHits([FIJI])[0], SIZE);
  assert.deepEqual(v.center, [179.0122737, -18.1239696]);
  assert.equal(v.zoom, 5);
  // Its rank alone says the same.
  assert.equal(hitView({ ...parseHits([FIJI])[0], type: undefined }, SIZE).zoom, 5);
});

test("a city whose point is well inside its box is fitted to the box", () => {
  const hit = parseHits([KRAKOW])[0];
  const v = hitView(hit, SIZE);
  assert.deepEqual(v.center, [19.9368564, 50.0619474]);
  assert.ok(v.zoom > 9.5 && v.zoom < 11, `zoom ${v.zoom}`);
  // A smaller map zooms out to keep the box in.
  assert.ok(hitView(hit, { width: 400, height: 300 }).zoom < v.zoom);
});

test("the zoom by kind, then rank, then a street's", () => {
  assert.equal(placeZoom({ type: "state" }), 7);
  assert.equal(placeZoom({ type: "city" }), 10);
  assert.equal(placeZoom({ type: "village" }), 13);
  assert.equal(placeZoom({ type: "road", rank: 26 }), 16);
  assert.equal(placeZoom({ type: "building" }), 16);
  assert.equal(placeZoom({ rank: 30 }), 16);
  assert.equal(placeZoom({ rank: 12 }), 9);
  assert.equal(placeZoom({}), null);
  // Neither, nor a usable box: as before.
  assert.equal(hitView({ name: "x", lat: 1, lon: 2, box: null }, SIZE).zoom, 14);
});

test("longitudes wrap into -180..180", () => {
  assert.equal(wrapLon(499.77).toFixed(2), "139.77");
  assert.equal(wrapLon(-190), 170);
  assert.equal(wrapLon(180), -180);
  assert.equal(wrapLon(-180), -180);
  assert.equal(wrapLon(19.9), 19.9);
});

test("the view as one box: wrapped, or every longitude when it crosses ±180", () => {
  // Japan after a turn round the world (#25): west 422, east 578 crosses 180 once wrapped.
  assert.deepEqual(viewBox(422, 20, 578, 50), { min_lat: 20, max_lat: 50, min_lon: -180, max_lon: 180 });
  // Japan after a turn, not crossing: wrapped back.
  const japan = viewBox(489, 30, 509, 45);
  assert.deepEqual([japan.min_lon, japan.max_lon], [129, 149]);
  // Fiji, across 180.
  assert.deepEqual(viewBox(170, -25, 190, -10), { min_lat: -25, max_lat: -10, min_lon: -180, max_lon: 180 });
  assert.deepEqual(viewBox(-190, -25, -170, -10), { min_lat: -25, max_lat: -10, min_lon: -180, max_lon: 180 });
  // Up to 180 exactly is one box.
  assert.deepEqual(viewBox(170, -25, 180, -10), { min_lat: -25, max_lat: -10, min_lon: 170, max_lon: 180 });
  // Zoomed out past the whole world, and the poles.
  assert.deepEqual(viewBox(-300, -95, 400, 95), { min_lat: -90, max_lat: 90, min_lon: -180, max_lon: 180 });
  assert.deepEqual(viewBox(19, 49, 21, 51), { min_lat: 49, max_lat: 51, min_lon: 19, max_lon: 21 });
});

test("a new trip is fitted and played from its end; an edit keeps the playhead", () => {
  assert.deepEqual(routeStep(null, "Japan 2019", 0, 40), { fresh: true, along: 39 });
  assert.deepEqual(routeStep("Japan 2019", "Kraków 2020", 12.5, 10), { fresh: true, along: 9 });
  // The same trip read again after a rating: stays at 12.5.
  assert.deepEqual(routeStep("Japan 2019", "Japan 2019", 12.5, 40), { fresh: false, along: 12.5 });
  // A photo left the route: the playhead stays within it.
  assert.deepEqual(routeStep("Japan 2019", "Japan 2019", 39, 30), { fresh: false, along: 29 });
  assert.deepEqual(routeStep("Japan 2019", "Japan 2019", 3, 0), { fresh: false, along: 0 });
});

test("the grid follows the map only when nothing holds it elsewhere", () => {
  const area = { min_lat: 0, max_lat: 1, min_lon: 0, max_lon: 1 };
  const base = { follow: true, mapOn: true, tile: false, filter: {} };
  assert.equal(shouldFollow(base), true);
  assert.equal(shouldFollow({ ...base, filter: { gps: true } }), true);
  assert.equal(shouldFollow({ ...base, follow: false }), false);
  assert.equal(shouldFollow({ ...base, mapOn: false }), false);
  assert.equal(shouldFollow({ ...base, tile: true }), false);
  assert.equal(shouldFollow({ ...base, filter: { gps: false } }), false);
  assert.equal(shouldFollow({ ...base, filter: { approx: true } }), false);
  assert.equal(shouldFollow({ ...base, filter: { approx: false } }), true);
  assert.equal(shouldFollow({ ...base, filter: { within: area } }), false, "already following, or a drawn area");
});

// The map in #22: 605 px high. What covers it, from the styles: the find box ends 36 px from
// the top (8 + its 28 px input), 180 px with six places listed; along the bottom the note
// reaches 28 px, the closed tray 59 (32 + 27), the open one 186 (32 + 154), the route bar 62.
const MAP = { width: 892, height: 605 };

test("padding: pins clear the find box above, cards the closed tray below", () => {
  const p = mapPadding({ ...MAP, findBottom: 36, coverBottom: 59 });
  // A pin stands 80 above its point, a ≈ card hangs 60 below it; 8 to spare.
  assert.deepEqual(p, { top: 124, bottom: 127, left: 42, right: 42 });
});

test("padding: the open tray", () => {
  const p = mapPadding({ ...MAP, findBottom: 36, coverBottom: 186 });
  assert.deepEqual(p, { top: 124, bottom: 254, left: 42, right: 42 });
});

test("padding: a trip's route bar", () => {
  assert.equal(mapPadding({ ...MAP, findBottom: 36, coverBottom: 62 }).bottom, 130);
  // Only the note: as low as it goes.
  assert.equal(mapPadding({ ...MAP, findBottom: 36, coverBottom: 28 }).bottom, 96);
});

test("padding: places listed under the find box, the tray open: a quarter of the map left", () => {
  const p = mapPadding({ ...MAP, findBottom: 180, coverBottom: 186 });
  assert.ok(p.top > 124 && p.top < 268, `top ${p.top}`);
  assert.ok(p.top > p.bottom);
  assert.ok(MAP.height - p.top - p.bottom >= MAP.height / 4, `left ${MAP.height - p.top - p.bottom}`);
  // A place found fits what is left: further out than on the whole map.
  const krakow = parseHits([KRAKOW])[0];
  const free = { width: MAP.width - p.left - p.right, height: MAP.height - p.top - p.bottom };
  assert.ok(hitView(krakow, free, 0).zoom < hitView(krakow, MAP, 0).zoom);
});

test("padding: a tiny map keeps a quarter of each side", () => {
  const p = mapPadding({ width: 100, height: 160, findBottom: 36, coverBottom: 59 });
  assert.ok(160 - p.top - p.bottom >= 40, `height left ${160 - p.top - p.bottom}`);
  assert.ok(100 - p.left - p.right >= 25, `width left ${100 - p.left - p.right}`);
  assert.equal(p.left, p.right);
  // Not measured yet: nothing.
  assert.deepEqual(mapPadding({ width: 0, height: 0, findBottom: 36, coverBottom: 59 }), { top: 0, bottom: 0, left: 0, right: 0 });
});

test("padding: whole pixels, never below 0", () => {
  const p = mapPadding({ width: 641.5, height: 333.3, findBottom: 36.4, coverBottom: 185.7 });
  for (const v of Object.values(p)) assert.ok(Number.isInteger(v) && v >= 0, `${v}`);
  assert.deepEqual(mapPadding({ ...MAP, findBottom: -5, coverBottom: -5 }), { top: 88, bottom: 68, left: 42, right: 42 });
});
