// The map as a filter: find a place, open a cluster, pick an area. Uses Kraków, where
// the fuji catalog has photos with GPS (elsewhere most are placed only by name, ≈).
const markers = (d) =>
  d.evaluate(`(() => {
    const c = document.querySelector(".map .canvas").getBoundingClientRect();
    return [...document.querySelectorAll(".map .maplibregl-marker:not(.approx)")].map((m) => {
      const r = m.getBoundingClientRect();
      return { x: r.x + r.width / 2, y: r.y + r.height / 2, cx: c.x + c.width / 2, cy: c.y + c.height / 2 };
    }).filter((p) => p.x > c.x + 40 && p.x < c.right - 40 && p.y > c.y + 60 && p.y < c.bottom - 60);
  })()`);

const find = async (d, place, delay) => {
  await d.click(".map .find input");
  await d.evaluate(`document.querySelector(".map .find input").select()`);
  await d.type(place, delay);
  await d.key("Enter");
  await d.waitFor(".map .find li button", null, { timeout: 15000 });
  await d.sleep(delay ? 500 : 0);
  await d.click(".map .find li button");
};

/** Waits for the map to stop and its photos to show. */
const still = (d) =>
  d.waitFor(null, null, {
    timeout: 20000,
    fn: `(() => { const m = document.querySelectorAll(".map .maplibregl-marker:not(.approx)"); return m.length > 0 && [...document.querySelectorAll(".map .maplibregl-marker img")].every((i) => i.complete); })()`,
  }).then(() => d.sleep(800));

export default {
  name: "map",
  title: "On the map",
  async run(d) {
    await d.reset();
    await d.press(".tabs button", /^Map$/);
    await d.waitFor(".map .find input", null, { timeout: 30000 });
    await d.evaluate(`(() => { const f = document.querySelector("header .follow input"); if (f && !f.checked) f.click(); })()`);
    await find(d, "Poland", 0);
    await d.sleep(4000);
    await d.evaluate(`(() => { const i = document.querySelector(".map .find input"); i.value = ""; i.dispatchEvent(new Event("input", { bubbles: true })); i.blur(); })()`);
    await d.settle();
    await d.record(async () => {
      await d.caption("Photos on the map, clustered; the grid shows what is in view");
      await d.sleep(2200);
      await d.caption("Find a place by name, or type coordinates");
      await find(d, "Kraków", 110);
      await d.cut(() => still(d));
      await d.settle();
      await d.sleep(800);
      await d.caption("Click a cluster to zoom in to its photos");
      const near = (await markers(d)).sort((a, b) => Math.hypot(a.x - a.cx, a.y - a.cy) - Math.hypot(b.x - b.cx, b.y - b.cy))[0];
      if (near) {
        await d.clickAt(near.x, near.y);
        await d.cut(() => d.sleep(1500).then(() => still(d)));
        await d.settle();
        await d.sleep(1000);
      }
      await d.caption("Shift+drag a box: the grid shows just those photos");
      const ms = await markers(d);
      const xs = ms.map((p) => p.x);
      const ys = ms.map((p) => p.y);
      const [x0, x1, y0, y1] = [Math.min(...xs) - 50, Math.max(...xs) + 50, Math.min(...ys) - 50, Math.max(...ys) + 40];
      await d.drag(x0, y0, x1, y1, { ms: 1100, modifiers: 8 });
      await d.sleep(800);
      await d.settle();
      await d.sleep(2000);
    });
  },
};
