// Show on map: a right-click on photos in the grid opens the map on where they are.
// Checks, on the running app:
//   1. one photo with GPS of its own, from the Photos tab: the Map tab opens centred on it,
//      close in (zoom 16);
//   2. several picked photos: the map takes them all in;
//   3. a photo placed only by its place's name: no closer than its place's level;
//   4. a photo with no position and no place: the item is greyed out.
// Prints what it found and saves a screenshot of each step next to the frames dir given.
//
// usage: node tools/record/drive.mjs tools/record/checks/show-on-map.mjs [shots dir] --port 9237
import { openMap, settled, view } from "./map-common.mjs";

const LEVEL_ZOOM = { Country: 5, State: 8, City: 11, Location: 14 };
const out = process.argv.slice(2).find((a, i, all) => i > 0 && !a.startsWith("--") && all[i - 1] !== "--port");

/** The grid's cells on screen: where each is and its photo's row (from its thumbnail). */
const cells = (d) =>
  d.evaluate(`[...document.querySelectorAll(".cell")].map((c) => {
    const r = c.getBoundingClientRect();
    const row = Number(c.querySelector("img")?.src.match(/thumb\\/(\\d+)/)?.[1]);
    return { x: r.x + r.width / 2, y: r.y + r.height / 2, row, w: r.width };
  }).filter((c) => c.w > 0 && c.row >= 0)`);

const menu = (d) =>
  d.evaluate(`(() => {
    const m = document.querySelector(".menu");
    if (!m) return null;
    const item = [...m.querySelectorAll("button")].find((b) => b.textContent.trim() === "Show on map");
    return {
      title: m.querySelector(".menu-title")?.textContent.trim(),
      disabled: item?.disabled,
      notes: [...m.querySelectorAll(".menu-note")].map((n) => n.textContent.trim()),
    };
  })()`);

async function rightClickAt(d, x, y) {
  await d.move(x, y);
  await d.sleep(80);
  await d.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "right", clickCount: 1 });
  await d.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "right", clickCount: 1 });
  // The menu asks where the photos are before it says what it can do.
  await d.waitFor(".menu .menu-title", null, { timeout: 5000 });
  for (let i = 0; i < 20 && (await menu(d))?.disabled && !(await menu(d))?.notes.length; i++) await d.sleep(100);
}

/** *Show on map*, then the map's view once it is where `there` says (or after 20 s): the
 *  map opened afresh first loads its style and stores its starting view, then moves. */
async function show(d, there) {
  await d.press(".menu button", /^Show on map$/);
  await d.waitFor(".map .find input", null, { timeout: 30000 });
  const end = Date.now() + 20000;
  let v = await view(d);
  while (Date.now() < end && !(v && there(v))) {
    await d.sleep(300);
    v = await view(d);
  }
  await settled(d, null, 2000);
  return view(d);
}

const inBox = (b, [lon, lat]) => lat >= b.min_lat - 1e-6 && lat <= b.max_lat + 1e-6 && lon >= b.min_lon - 1e-6 && lon <= b.max_lon + 1e-6;
const results = [];
const check = (name, ok, detail) => {
  results.push({ name, ok });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}: ${JSON.stringify(detail)}`);
};

export default {
  name: "check-show-on-map",
  title: "Show on map",
  async run(d) {
    const shot = async (name) => out && d.shot(`${out}/${name}.png`);

    // 1. One photo with GPS of its own, from the Photos tab; the map starts over the world.
    await d.reset();
    await d.evaluate(`localStorage.removeItem("map-view")`);
    let one = null;
    for (const c of await cells(d)) {
      const a = await d.invoke("area", { rows: [c.row] });
      if (a.exact === 1) {
        one = { ...c, area: a };
        break;
      }
    }
    if (!one) throw new Error("no photo with GPS among the first cells");
    await rightClickAt(d, one.x, one.y);
    const m1 = await menu(d);
    await shot("1-menu");
    const p = one.area.bounds;
    const v1 = await show(d, (v) => v.zoom > 15 && Math.abs(v.center[1] - p.min_lat) < 0.01);
    await shot("1-map");
    check("one GPS photo: map opens close on it", v1.zoom > 15 && Math.abs(v1.center[1] - p.min_lat) < 0.01, { menu: m1, view: v1, at: p });

    // 2. Several picked photos, from further down the Photos tab (other places).
    await d.reset();
    const all = await cells(d);
    const several = [all[0], all[Math.floor(all.length / 2)], all[all.length - 1]];
    await d.clickAt(several[0].x, several[0].y);
    for (const c of several.slice(1)) await d.clickAt(c.x, c.y, { modifiers: 2 });
    const a2 = await d.invoke("area", { rows: several.map((c) => c.row) });
    await rightClickAt(d, several[1].x, several[1].y);
    const m2 = await menu(d);
    await shot("2-menu");
    const cap = a2.coarsest ? LEVEL_ZOOM[a2.coarsest] : 16;
    const v2 = await show(d, (v) => !!a2.bounds && inBox(a2.bounds, v.center) && v.zoom <= cap + 1e-6);
    await shot("2-map");
    check("several photos: the map takes them in", m2.title === "3 photos" && !!a2.bounds && inBox(a2.bounds, v2.center) && v2.zoom <= cap + 1e-6, { menu: m2, area: a2, view: v2 });

    // 3. Photos placed only by their place's name: the tray's *Only a place*, in the strip.
    await openMap(d);
    if (!(await d.locate(".tray [role=tab]"))) await d.press(".tray .fold");
    await d.press("[role=tab]", /^Only a place$/);
    await d.press("button", /^Show in grid$/);
    await d.settle();
    const placed = (await cells(d))[0];
    if (placed) {
      const a3 = await d.invoke("area", { rows: [placed.row] });
      await rightClickAt(d, placed.x, placed.y);
      const m3 = await menu(d);
      await shot("3-menu");
      const v3 = await show(d, (v) => Math.abs(v.zoom - LEVEL_ZOOM[a3.coarsest]) < 0.01);
      await shot("3-map");
      check(
        "a place only: no closer than its level, its marker in view",
        a3.approx === 1 && Math.abs(v3.zoom - LEVEL_ZOOM[a3.coarsest]) < 0.01,
        { menu: m3, area: a3, view: v3, marker: await d.evaluate(`document.querySelectorAll(".canvas .approx, .canvas [class*=approx]").length`) },
      );
    } else console.log("SKIP a place only: none in this catalog");

    // 4. No position and no place: *No location*.
    await d.press("[role=tab]", /^No location$/);
    await d.press("button", /^Show in grid$/);
    await d.settle();
    const nowhere = (await cells(d))[0];
    if (nowhere) {
      await rightClickAt(d, nowhere.x, nowhere.y);
      const m4 = await menu(d);
      await shot("4-menu");
      check("nowhere known: greyed out with a note", m4?.disabled === true && m4.notes.length === 1, m4);
      await d.key("Escape");
      check("Escape closes the menu", !(await menu(d)), null);
    } else console.log("SKIP nowhere known: none in this catalog");

    await d.reset();
    const failed = results.filter((r) => !r.ok).length;
    console.log(`${results.length - failed}/${results.length} passed`);
  },
};
