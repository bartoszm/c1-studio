// #23: with a trip shown, an edit keeps the map's view and the playhead.
// Picks the first trip, opens the Map tab, zooms in and moves the playhead, rates the
// first photo in the strip and undoes it. Prints the view and the playhead before and
// after. Expected: both the same.
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
    await d.press("aside .trip");
    await d.press(".tabs button", /^Map$/);
    await d.waitFor(".map .route input[type=range]", null, { timeout: 30000 });
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
