// The grid, the details panel and the full-screen viewer.
export default {
  name: "browse",
  title: "Browse",
  async run(d) {
    await d.reset();
    await d.click("input.search");
    await d.type("tokyo street", 0);
    await d.key("Enter");
    await d.key("Escape");
    await d.settle();
    await d.record(async () => {
      await d.caption("Click a photo: the details panel shows everything about it");
      const cells = await d.evaluate(`[...document.querySelectorAll(".cell")].slice(0, 12).map((c) => { const r = c.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })`);
      await d.clickAt(cells[6].x, cells[6].y);
      await d.sleep(2200);
      await d.caption("Double-click or Enter opens it full size; arrows step through");
      await d.clickAt(cells[6].x, cells[6].y, { clickCount: 2 });
      await d.sleep(1800);
      for (let i = 0; i < 2; i++) {
        await d.key("ArrowRight");
        await d.sleep(1200);
      }
      await d.key("Escape");
      await d.sleep(700);
      await d.caption("Tiles any size: the slider, or Ctrl+wheel over the grid");
      const r = await d.evaluate(`(() => { const e = document.querySelector('input[aria-label="Tile size"]'); const b = e.getBoundingClientRect(); return { x: b.x, y: b.y + b.height / 2, w: b.width, v: (+e.value - +e.min) / (+e.max - +e.min) }; })()`);
      const at = r.x + r.w * r.v;
      await d.drag(at, r.y, at + r.w * 0.45, r.y, { ms: 1100 });
      await d.settle();
      await d.sleep(900);
      await d.drag(at + r.w * 0.45, r.y, at, r.y, { ms: 1000 });
      await d.settle();
      await d.sleep(900);
    });
  },
};
