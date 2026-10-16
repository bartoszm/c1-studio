// The keyword tree: find a keyword, group it under a new parent. Undone afterwards.
export default {
  name: "keywords",
  title: "The keyword tree",
  async run(d) {
    await d.reset();
    await d.evaluate(`document.querySelector("aside input.tree-search").scrollIntoView({ block: "start" })`);
    await d.sleep(300);
    let done = false;
    try {
      await d.record(async () => {
        await d.caption("Find a keyword in a tree of thousands, click it to filter");
        await d.click("aside input.tree-search");
        await d.type("chess", 110);
        await d.sleep(700);
        await d.click("aside button.label", /^chess\n/);
        await d.sleep(500);
        await d.settle();
        await d.sleep(1000);
        await d.caption("Right-click to rename, join, group or ungroup");
        await d.rightClick("aside button.label", /^chess\n/);
        await d.sleep(1000);
        await d.click(".menu button", /^Group under…$/);
        await d.waitFor(".dialog h2");
        await d.sleep(500);
        await d.type("games", 120);
        await d.sleep(1100);
        await d.click(".dialog button.primary");
        done = true;
        await d.waitFor(null, null, { fn: `!document.querySelector(".dialog")` });
        await d.caption("The change shows at once, queued for Capture One like any edit");
        await d.sleep(400);
        await d.click("aside input.tree-search");
        await d.evaluate(`document.querySelector("aside input.tree-search").select()`);
        await d.type("games", 110);
        await d.sleep(600);
        await d.click("aside button.label", /^games\n/);
        await d.sleep(500);
        await d.settle();
        await d.sleep(1800);
      });
    } finally {
      await d.evaluate(`document.activeElement?.blur()`);
      if (done) await d.key("z", { modifiers: 2 });
      await d.evaluate(`(() => { const i = document.querySelector("aside input.tree-search"); i.value = ""; i.dispatchEvent(new Event("input", { bubbles: true })); })()`);
    }
  },
};
