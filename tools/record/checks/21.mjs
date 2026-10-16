// #21: a place found by name is in the middle of the map, at a zoom that suits it.
// Prints, per place, the map's centre against Nominatim's point, the zoom, and Nominatim's
// row (box, rank, kind), to compare with app/test/mapview.test.ts's rows. Each reading
// waits for the map to stop. Expected: the centre within a few hundredths of a degree of
// the point; Tokyo zoom 10, the countries about 5, Fiji not the whole world.
//
// usage: node tools/record/drive.mjs tools/record/checks/21.mjs --port 9223
import { find, openMap, view } from "./map-common.mjs";

export default {
  name: "check-21",
  title: "Find a place: centred on it",
  async run(d) {
    await openMap(d);
    for (const place of ["Tokyo", "France", "Netherlands", "Norway", "Fiji", "Kraków"]) {
      const hit = await find(d, place);
      const v = await view(d);
      console.log(JSON.stringify({
        place,
        centre: v.center.map((n) => +n.toFixed(4)),
        point: [+hit.lon, +hit.lat],
        off: +Math.hypot(v.center[0] - hit.lon, v.center[1] - hit.lat).toFixed(4),
        zoom: +v.zoom.toFixed(2),
        nominatim: { boundingbox: hit.boundingbox, place_rank: hit.place_rank, addresstype: hit.addresstype },
      }));
    }
    await d.evaluate(`(() => { const i = document.querySelector(".map .find input"); i.value = ""; i.dispatchEvent(new Event("input", { bubbles: true })); i.blur(); })()`);
  },
};
