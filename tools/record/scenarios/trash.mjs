// Deleting photos into the app's trash, looking at it, and restoring them. Nothing is
// emptied, so nothing reaches the catalog; the scenario restores what it deleted, so it
// can be recorded again.
export default {
  name: "trash",
  title: "Delete, with a trash",
  async run(d) {
    await d.reset();
    await d.click("input.search");
    await d.type("tokyo", 0);
    await d.key("Enter");
    await d.key("Escape");
    await d.settle();
    const cells = await d.evaluate(`[...document.querySelectorAll(".cell")].slice(0, 6).map((c) => { const r = c.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })`);
    let binned = false;
    try {
      await d.record(async () => {
        await d.caption("Pick photos and press Delete");
        await d.clickAt(cells[3].x, cells[3].y);
        await d.sleep(400);
        await d.clickAt(cells[5].x, cells[5].y, { modifiers: 8 });
        await d.sleep(900);
        await d.key("Delete");
        binned = true;
        await d.sleep(500);
        await d.settle();
        await d.caption("They leave every view and every count; nothing is deleted yet");
        await d.sleep(2400);
        await d.caption("Trash, at the bottom of the sidebar, shows them");
        await d.click(".trash-row");
        await d.settle();
        await d.sleep(1800);
        const empty = await d.locate(".bin-bar button.danger");
        if (empty) await d.move(empty.x, empty.y, 600);
        await d.caption("Only Empty trash deletes them, after asking: files to the Recycle Bin, Capture One's own Trash");
        await d.sleep(3000);
        await d.caption("Restore puts them back in every view");
        await d.clickAt(cells[0].x, cells[0].y);
        await d.sleep(300);
        await d.key("a", { modifiers: 2 });
        await d.sleep(600);
        await d.click(".bin-bar button", /^Restore$/);
        binned = false;
        await d.sleep(600);
        await d.settle();
        await d.sleep(1200);
        await d.click(".bin-bar button", /^Close$/);
        await d.settle();
        await d.sleep(1600);
      });
    } finally {
      if (binned) {
        await d.key("z", { modifiers: 2 });
        await d.sleep(400);
      }
      if (await d.locate(".trash-row.on")) await d.press(".trash-row");
    }
  },
};
