// #25: after panning across 180°, the map and the grid still show what is in view.
// 1. The view the app remembered from an unwrapped centre (499.77, as in the issue) is
//    opened; 2. the map is dragged east across the Pacific, once round the world.
// Prints the remembered centre, the map's note and the grid's total each time. Expected:
// the centre within -180..180, the note's counts not 0 where Japan is in view, the grid's
// total matching the note.
//
// usage: node tools/record/drive.mjs tools/record/checks/25.mjs --port 9223
import { openMap, state, view } from "./map-common.mjs";

export default {
  name: "check-25",
  title: "Across 180°",
  async run(d) {
    await d.evaluate(`localStorage.setItem("map-view", JSON.stringify({ center: [499.77, 35.6], zoom: 4 }))`);
    await openMap(d);
    await d.sleep(2000);
    console.log(JSON.stringify({ step: "opened at 499.77", view: await view(d), ...(await state(d)) }));
    const c = await d.evaluate(`(() => { const r = document.querySelector(".map .canvas").getBoundingClientRect(); return { x: r.x, y: r.y + r.height / 2, w: r.width }; })()`);
    // At zoom 4 the world is 8192 px wide: drag it ten times by 820 px, eastward.
    for (let i = 0; i < 10; i++) {
      await d.drag(c.x + c.w * 0.9, c.y, c.x + c.w * 0.9 - 820, c.y, { ms: 300 });
      await d.sleep(600);
      const v = await view(d);
      console.log(JSON.stringify({ step: `drag ${i + 1}`, centre: v.center.map((n) => +n.toFixed(2)), ...(await state(d)) }));
    }
  },
};
