// Writing to Capture One, a photo's history, and the backups made before each write.
// This one really writes, to the recording copy of the catalog: it rates a photo ★★★★★,
// or ★★★★ when it already has five, so every run changes something.
export default {
  name: "history",
  title: "Write, history and backups",
  async run(d) {
    await d.reset();
    await d.click("input.search");
    await d.type("tokyo street", 0);
    await d.key("Enter");
    await d.key("Escape");
    await d.settle();
    const c = await d.evaluate(`(() => { const r = document.querySelectorAll(".cell")[2].getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await d.clickAt(c.x, c.y);
    await d.sleep(500);
    await d.evaluate(`(() => { const h = document.querySelector("details.history"); h.open = true; })()`);
    const stars = await d.evaluate(`document.querySelectorAll(".rate .star.on").length`);
    await d.record(async () => {
      await d.caption("Nothing reaches Capture One until you write it");
      await d.key(stars === 5 ? "4" : "5");
      await d.sleep(900);
      await d.click("footer button", /edits? waiting/);
      await d.waitFor(".dialog button.primary", /^Write \d+ edits?$/);
      await d.sleep(1300);
      await d.caption("A backup of the catalog is made first, then the edits go in");
      await d.click(".dialog button.primary", /^Write \d+ edits?$/);
      await d.cut(() => d.waitFor(".dialog button.primary", /^Done$/, { timeout: 120000 }));
      await d.sleep(1500);
      await d.click(".dialog button.primary", /^Done$/);
      await d.sleep(500);
      await d.caption("Every change is kept in the photo's history, wherever it was made");
      await d.evaluate(`document.querySelector("details.history").scrollIntoView({ block: "center", behavior: "smooth" })`);
      await d.sleep(2600);
      await d.caption("Any backup can be restored from the app");
      await d.click("header .source-button");
      await d.sleep(700);
      await d.click(".source-menu button", /^Backups…$/);
      await d.waitFor(".dialog h2", "Backups");
      await d.cut(() => d.waitFor(".dialog li", null, { timeout: 60000 }));
      await d.sleep(2800);
      await d.key("Escape");
      await d.sleep(500);
    });
  },
};
