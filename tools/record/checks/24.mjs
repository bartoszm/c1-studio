// #24: "grid follows map" applies as soon as it can, not at the next move of the map.
// Turns following off and on, then uses Clear filters with it on. Prints the chips, the
// grid's total and the map's note each time, the map never moved. Expected: the
// "Map: area" chip back right after turning it on and after Clear filters, and the grid's
// total the number the note gives in view.
//
// usage: node tools/record/drive.mjs tools/record/checks/24.mjs --port 9223
import { openMap, state } from "./map-common.mjs";

export default {
  name: "check-24",
  title: "Grid follows map at once",
  async run(d) {
    await openMap(d);
    const follow = (on) =>
      d.evaluate(`(() => { const f = document.querySelector("header .follow input"); if (f.checked !== ${on}) f.click(); })()`);
    await follow(true);
    await d.sleep(1500);
    console.log(JSON.stringify({ step: "on", ...(await state(d)) }));
    await follow(false);
    await d.sleep(1500);
    console.log(JSON.stringify({ step: "off", ...(await state(d)) }));
    await follow(true);
    await d.sleep(1500);
    console.log(JSON.stringify({ step: "on again", ...(await state(d)) }));
    await d.click("input.search");
    await d.type("market", 0);
    await d.key("Enter");
    await d.key("Escape");
    await d.sleep(1500);
    await d.press("button", /^Clear filters$/);
    await d.sleep(1500);
    console.log(JSON.stringify({ step: "Clear filters", ...(await state(d)) }));
  },
};
