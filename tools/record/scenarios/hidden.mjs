// Hiding photos from every view, showing them again, and bringing them back. The scenario
// unhides what it hid, so it can be recorded again.
export default {
  name: "hidden",
  title: "Hide sensitive photos",
  async run(d) {
    await d.reset();
    await d.click("input.search");
    await d.type("tokyo", 0);
    await d.key("Enter");
    await d.key("Escape");
    await d.settle();
    const cells = await d.evaluate(`[...document.querySelectorAll(".cell")].slice(0, 6).map((c) => { const r = c.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })`);
    let hidden = false;
    try {
      await d.record(async () => {
        await d.caption("Pick photos you'd rather not show, and press h");
        await d.clickAt(cells[1].x, cells[1].y);
        await d.sleep(400);
        await d.clickAt(cells[2].x, cells[2].y, { modifiers: 8 });
        await d.sleep(900);
        await d.key("h");
        hidden = true;
        await d.sleep(500);
        await d.settle();
        await d.caption("They leave every view and every count; nothing changes in Capture One");
        await d.sleep(2400);
        await d.caption("Hidden in the header shows them again, marked");
        await d.click("header button.toggle", /^Hidden$/);
        await d.settle();
        await d.sleep(2200);
        await d.caption("Pick them and press h (or Unhide) to bring them back");
        await d.clickAt(cells[1].x, cells[1].y);
        await d.sleep(300);
        await d.clickAt(cells[2].x, cells[2].y, { modifiers: 8 });
        await d.sleep(600);
        await d.key("h");
        hidden = false;
        await d.sleep(1600);
      });
    } finally {
      if (hidden) {
        await d.key("z", { modifiers: 2 });
        await d.sleep(400);
      }
      if (await d.locate("header button.toggle.on", /^Hidden$/)) await d.press("header button.toggle", /^Hidden$/);
    }
  },
};
