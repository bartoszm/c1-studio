// Places › No location, a plus code typed into the map, and the map and grid in sync:
// the photo picked in the grid centres the map, and the details panel gives its plus code.
const still = (d) =>
  d.waitFor(null, null, {
    timeout: 20000,
    fn: `(() => { const m = document.querySelectorAll(".map .maplibregl-marker:not(.approx)"); return m.length > 0 && [...document.querySelectorAll(".map .maplibregl-marker img")].every((i) => i.complete); })()`,
  }).then(() => d.sleep(800));

export default {
  name: "places",
  title: "No location, plus codes, map and grid in sync",
  async run(d) {
    await d.reset();
    await d.record(async () => {
      await d.caption("Places › No location: the photos with nothing in the Location field");
      await d.click(".none-row", /^No location$/);
      await d.sleep(600);
      await d.settle();
      await d.sleep(2200);
      await d.caption("Click again for the photos with one");
      await d.click(".none-row", /^No location$/);
      await d.sleep(600);
      await d.settle();
      await d.sleep(1800);
      await d.click(".none-row", /^With a location$/);
      await d.sleep(400);

      await d.caption("A Google plus code goes wherever GPS is typed, here with a town");
      await d.cut(async () => {
        await d.press(".tabs button", /^Map$/);
        await d.waitFor(".map .find input", null, { timeout: 30000 });
        await d.evaluate(`(() => { const f = document.querySelector(".map-tools .follow input"); if (f && !f.checked) f.click(); })()`);
        // Start from the whole country, so the flight to the code shows.
        await d.click(".map .find input");
        await d.evaluate(`document.querySelector(".map .find input").select()`);
        await d.type("Poland", 0);
        await d.key("Enter");
        await d.waitFor(".map .find li button", null, { timeout: 15000 });
        await d.click(".map .find li button");
        await d.sleep(4000);
        await d.evaluate(`(() => { const i = document.querySelector(".map .find input"); i.value = ""; i.dispatchEvent(new Event("input", { bubbles: true })); i.blur(); })()`);
        await d.settle();
      });
      await d.click(".map .find input");
      await d.evaluate(`document.querySelector(".map .find input").select()`);
      await d.type("3W6P+HJ Kraków", 110);
      await d.key("Enter");
      await d.sleep(2500);
      await d.cut(() => still(d));
      await d.settle();
      await d.sleep(1200);
      await d.caption("With sync grid and map on, the map centres on the photo picked in the grid");
      const cells = await d.evaluate(`[...document.querySelectorAll(".cell")].slice(0, 3).map((c) => { const r = c.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })`);
      await d.clickAt(cells[1].x, cells[1].y);
      await d.sleep(1800);
      for (let i = 0; i < 2; i++) {
        await d.key("ArrowRight");
        await d.sleep(1800);
      }
      const code = await d.locate("dt + dd button.link", /^[23456789CFGHJMPQRVWX]{4,8}\+/);
      if (code) {
        await d.caption("The details panel shows the photo's plus code: a click copies it");
        await d.move(code.x, code.y, 700);
        await d.sleep(2600);
      }
    });
  },
};
