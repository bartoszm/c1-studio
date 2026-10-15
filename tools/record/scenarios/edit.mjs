// Ratings, colour tags and keywords on several photos; the edits wait in a queue until
// written. The scenario undoes its edits afterwards, so it can be recorded again.
export default {
  name: "edit",
  title: "Edit, then write",
  async run(d) {
    await d.reset();
    await d.click("input.search");
    await d.type("market", 0);
    await d.key("Enter");
    await d.key("Escape");
    await d.evaluate(`(() => { const k = document.getElementById("keyword-input"); if (k) { k.value = ""; k.dispatchEvent(new Event("input", { bubbles: true })); } })()`);
    await d.settle();
    const cells = await d.evaluate(`[...document.querySelectorAll(".cell")].slice(0, 12).map((c) => { const r = c.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })`);
    let edits = 0;
    try {
      await d.record(async () => {
        await d.caption("Pick photos: click, then Shift+click");
        await d.clickAt(cells[0].x, cells[0].y);
        await d.sleep(500);
        await d.clickAt(cells[3].x, cells[3].y, { modifiers: 8 });
        await d.sleep(1200);
        await d.caption("Keys rate and tag them: 4 for ★★★★, + for green");
        await d.key("4");
        edits++;
        await d.sleep(1100);
        await d.key("+");
        edits++;
        await d.sleep(1300);
        await d.caption("Ctrl+K adds a keyword, completed from the catalog");
        await d.key("k", { modifiers: 2 });
        await d.sleep(500);
        await d.type("food", 110);
        await d.sleep(700);
        await d.key("Enter");
        edits++;
        await d.sleep(1400);
        await d.caption("Edits wait in a queue; Capture One gets them when you say");
        await d.click("footer button", /edits? waiting/);
        await d.waitFor(".dialog button", /^Write \d+ edits?$/);
        await d.sleep(2800);
        await d.key("Escape");
        await d.sleep(600);
      });
    } finally {
      for (let i = 0; i < edits; i++) {
        await d.key("z", { modifiers: 2 });
        await d.sleep(200);
      }
    }
  },
};
