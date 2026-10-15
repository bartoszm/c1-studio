// #22: zooms and flights leave room for the find box, the "Not on the map" tray and the pins.
// Opens the tray, double-clicks the biggest cluster, finds a place, then fits a trip's route
// (the first with two photos or more with a position). After each, prints how many photo
// cards (GPS pins and ≈ cards) overlap the find box, the tray or the route bar, and how many
// reach past the canvas: of the photos zoomed to (the cluster's; those in the place's box;
// the trip's, all the map shows), and of every card in view. Expected: 0 everywhere for the
// photos zoomed to; cards of photos round them may still be anywhere.
//
// usage: node tools/record/drive.mjs tools/record/checks/22.mjs --port 9223
import { find, openMap, settled } from "./map-common.mjs";

/** The cards in view, and those of the photos in `box` (min_lat…: BBox), against what
 *  covers the map. ≈ cards carry no bounds: they count in `all` only. */
const covered = (d, box) =>
  d.evaluate(`(() => {
    const rect = (e) => e?.getBoundingClientRect();
    const canvas = rect(document.querySelector(".map .canvas"));
    const covers = {
      find: rect(document.querySelector(".map .find")),
      tray: rect(document.querySelector(".map .tray")),
      route: rect(document.querySelector(".map .route")),
    };
    const meet = (a, b) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
    const box = ${JSON.stringify(box ?? null)};
    const inBox = (pin) => {
      const b = JSON.parse(pin.dataset.bounds ?? "null");
      return !!box && !!b && b.min_lat >= box.min_lat && b.max_lat <= box.max_lat && b.min_lon >= box.min_lon && b.max_lon <= box.max_lon;
    };
    const count = (cards) => {
      const out = {
        cards: cards.length,
        outside: cards.filter((c) => c.left < canvas.left || c.right > canvas.right || c.top < canvas.top || c.bottom > canvas.bottom).length,
      };
      for (const [k, b] of Object.entries(covers)) if (b) out[k] = cards.filter((c) => meet(c, b)).length;
      return out;
    };
    const pins = [...document.querySelectorAll(".map .maplibregl-marker.pin")];
    // A card and its count; a disc far out is its own card.
    const card = (pin) => {
      const c = pin.querySelector(".card");
      if (!c) return rect(pin);
      const r = rect(c);
      const n = rect(c.querySelector("span"));
      return n ? { left: r.left, top: Math.min(r.top, n.top), right: Math.max(r.right, n.right), bottom: r.bottom } : r;
    };
    return {
      zoomed: box ? count(pins.filter(inBox).map(card)) : undefined,
      all: count(pins.map(card)),
      covers: Object.fromEntries(Object.entries(covers).filter(([, b]) => b).map(([k, b]) => [k, [Math.round(b.top - canvas.top), Math.round(b.bottom - canvas.top)]])),
      canvas: [Math.round(canvas.width), Math.round(canvas.height)],
    };
  })()`);

export default {
  name: "check-22",
  title: "Room for the find box, the tray and the pins",
  async run(d) {
    await openMap(d, { center: [15, 30], zoom: 1.5 });
    await d.press(".map .tray .fold");
    await d.sleep(1500);
    console.log(JSON.stringify({ step: "tray open", ...(await covered(d)) }));

    // The biggest cluster, zoomed to as a double click does.
    const cluster = await d.evaluate(`(() => {
      const pins = [...document.querySelectorAll(".map .maplibregl-marker.pin:not(.approx)")].filter((p) => +p.dataset.count > 1);
      pins.sort((a, b) => b.dataset.count - a.dataset.count);
      const p = pins[0];
      if (!p) return null;
      p.dataset.check = "22";
      return { count: +p.dataset.count, bounds: JSON.parse(p.dataset.bounds) };
    })()`);
    if (!cluster) {
      console.log(JSON.stringify({ note: "no cluster of two photos or more in view: the zoom can't be checked on this catalog" }));
    } else {
      const before = await d.evaluate(`localStorage.getItem("map-view")`);
      await d.evaluate(`document.querySelector('.map .pin[data-check="22"]').dispatchEvent(new MouseEvent("dblclick", { bubbles: true }))`);
      await settled(d, before);
      await d.sleep(1500);
      console.log(JSON.stringify({ step: "cluster", count: cluster.count, ...(await covered(d, cluster.bounds)) }));
    }

    const hit = await find(d, "Kraków");
    await d.sleep(1500);
    const [south, north, west, east] = hit.boundingbox.map(Number);
    console.log(JSON.stringify({ step: "find Kraków", ...(await covered(d, { min_lat: south, max_lat: north, min_lon: west, max_lon: east })) }));
    await d.evaluate(`(() => { const i = document.querySelector(".map .find input"); i.value = ""; i.dispatchEvent(new Event("input", { bubbles: true })); i.blur(); })()`);

    // A trip's route: the trips in turn, until one draws it.
    await d.reset({ timeline: true });
    if (await d.locate("aside button.primary", /^Home is/)) {
      await d.press("aside button.primary", /^Home is/);
      await d.waitFor("aside .trip");
    }
    const trips = await d.evaluate(`document.querySelectorAll("aside .trip").length`);
    await d.press(".tabs button", /^Map$/);
    await d.waitFor(".map .find input", null, { timeout: 30000 });
    let found = null;
    for (let i = 0; i < trips && !found; i++) {
      const before = await d.evaluate(`localStorage.getItem("map-view")`);
      await d.evaluate(`document.querySelectorAll("aside .trip")[${i}].click()`);
      await d.waitFor(null, null, { timeout: 8000, fn: `!!document.querySelector(".map .route")` }).catch(() => {});
      await d.sleep(500);
      if (await d.evaluate(`!!document.querySelector(".map .route input[type=range]")`)) {
        found = await d.evaluate(`document.querySelectorAll("aside .trip")[${i}].textContent.trim().split("\\n")[0]`);
        await settled(d, before);
      }
    }
    if (!found) {
      console.log(JSON.stringify({ note: `none of the ${trips} trips has two photos or more with a position: the route can't be checked on this catalog` }));
      return;
    }
    await d.sleep(1500);
    // The trip is the filter: every card the map shows is one of its photos.
    console.log(JSON.stringify({ step: "trip", trip: found, ...(await covered(d)) }));
    const before = await d.evaluate(`localStorage.getItem("map-view")`);
    await d.press(".map .route button", /^⤢$/);
    await settled(d, before);
    await d.sleep(1500);
    console.log(JSON.stringify({ step: "route fitted again", trip: found, ...(await covered(d)) }));
    console.log("expected: zoomed (cluster, Kraków) and all (trip) with outside, find, tray and route 0");
  },
};
