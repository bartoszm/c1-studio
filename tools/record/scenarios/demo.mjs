// The README's tour (docs/media/demo.gif): search, the counts, a photo, the viewer, the
// analysis, and what else opens. record.ps1 encodes it at a low frame rate.
export default {
  name: "demo",
  title: "C1 Studio",
  async run(d) {
    await d.reset();
    await d.record(async () => {
      await d.caption("Search keywords, captions, places and file names");
      await d.click("input.search");
      await d.type("hong kong street", 70);
      await d.key("Enter");
      await d.key("Escape");
      await d.sleep(500);
      await d.settle();
      await d.sleep(1500);
      await d.caption("Every count in the sidebar follows the search");
      await d.click("aside button.label", /^2025\n/);
      await d.sleep(500);
      await d.settle();
      await d.sleep(1800);
      await d.caption("Click a photo: everything about it, from the catalog and measured");
      const cells = await d.evaluate(`[...document.querySelectorAll(".cell")].slice(0, 8).map((c) => { const r = c.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })`);
      await d.clickAt(cells[1].x, cells[1].y);
      await d.sleep(2400);
      await d.caption("Double-click or Enter opens it full size");
      await d.key("Enter");
      await d.sleep(1600);
      await d.key("ArrowRight");
      await d.sleep(1600);
      await d.key("Escape");
      await d.sleep(600);
      await d.caption("Every photo is measured once: shape, colour, light, focus, duplicates");
      await d.click(".tabs button", /^Analysis$/);
      await d.sleep(1500);
      await d.settle();
      await d.sleep(2600);
      await d.click(".tabs button", /^Photos$/);
      await d.sleep(600);
      await d.caption("Open a Capture One catalog, or plain photo folders");
      await d.click("header .source-button");
      await d.waitFor(".source-menu");
      await d.sleep(3000);
      await d.key("Escape");
      await d.sleep(400);
    });
  },
};
