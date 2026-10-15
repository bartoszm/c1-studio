// #23: with a trip shown, an edit keeps the map's view and the playhead.
// Picks the first trip with a route (two photos or more with a position), opens the Map
// tab, zooms in and moves the playhead, rates the first photo in the strip and undoes it.
// Prints the view and the playhead before and after. Expected: both the same. A catalog
// with no such trip says so.
//
// usage: node tools/record/drive.mjs tools/record/checks/23.mjs --port 9223
import { view } from "./map-common.mjs";

const playhead = (d) => d.evaluate(`document.querySelector(".map .route input[type=range]")?.value ?? null`);

export default {
  name: "check-23",
  title: "A trip's route through an edit",
  async run(d) {
    await d.reset({ timeline: true });
    if (await d.locate("aside button.primary", /^Home is/)) {
      await d.press("aside button.primary", /^Home is/);
      await d.waitFor("aside .trip");
    }
    // The trips in turn, until one draws a route.
    const trips = await d.evaluate(`document.querySelectorAll("aside .trip").length`);
    await d.press(".tabs button", /^Map$/);
    await d.waitFor(".map .find input", null, { timeout: 30000 });
    let found = null;
    for (let i = 0; i < trips && !found; i++) {
      await d.evaluate(`document.querySelectorAll("aside .trip")[${i}].click()`);
      await d.waitFor(null, null, { timeout: 8000, fn: `!!document.querySelector(".map .route")` }).catch(() => {});
      await d.sleep(500);
      if (await d.evaluate(`!!document.querySelector(".map .route input[type=range]")`)) {
        found = await d.evaluate(`document.querySelectorAll("aside .trip")[${i}].textContent.trim().split("\n")[0]`);
      }
    }
    if (!found) {
      console.log(JSON.stringify({ note: `none of the ${trips} trips has two photos or more with a position: #23 can't be checked on this catalog` }));
      return;
    }
    console.log(JSON.stringify({ trip: found }));
    await d.sleep(2500);
    const c = await d.evaluate(`(() => { const r = document.querySelector(".map .canvas").getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await d.wheel(c.x, c.y, -600);
    await d.sleep(1500);
    await d.evaluate(`(() => { const i = document.querySelector(".map .route input[type=range]"); i.value = String(Math.floor(i.max / 2)); i.dispatchEvent(new Event("input", { bubbles: true })); })()`);
    await d.sleep(500);
    const before = { view: await view(d), along: await playhead(d) };
    const cell = await d.locate(".cell");
    await d.clickAt(cell.x, cell.y);
    await d.sleep(300);
    // Moving the playhead or picking a photo must not move the map: measure from here.
    const picked = { view: await view(d), along: await playhead(d) };
    let edited = false;
    try {
      await d.key("4");
      edited = true;
      await d.sleep(2000);
    } finally {
      const after = { view: await view(d), along: await playhead(d) };
      if (edited) {
        await d.key("z", { modifiers: 2 });
        await d.sleep(2000);
      }
      const undone = { view: await view(d), along: await playhead(d) };
      console.log(JSON.stringify({ before, picked, after, undone }, null, 2));
      console.log("expected: picked, after and undone the same view and playhead");
    }
  },
};
