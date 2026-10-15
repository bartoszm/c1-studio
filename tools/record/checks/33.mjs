// #33: an edit or undo on photos past position 500 must not blank the grid or the details
// panel. Prints what was replaced during each step; expected: 0 thumbnails removed, the
// panel never empty, the keyword input keeping focus, the panel's scroll kept.
//
// usage: node tools/record/drive.mjs tools/record/checks/33.mjs --port 9223
export default {
  name: "check-33",
  title: "Edits past position 500 keep the grid and the panel",
  async run(d) {
    await d.reset();
    await d.click("input.search");
    await d.type("Japan", 0);
    await d.key("Enter");
    await d.key("Escape");
    await d.settle();
    // Scroll the grid to about position 1,000.
    await d.evaluate(`(() => {
      const cells = [...document.querySelectorAll(".cell")];
      const top = cells[0].getBoundingClientRect().top;
      const cols = cells.filter((c) => c.getBoundingClientRect().top === top).length;
      const line = cells[cols].getBoundingClientRect().top - top;
      cells[0].closest("main").scrollTop = Math.floor(1000 / cols) * line;
    })()`);
    await d.sleep(800);
    await d.settle();
    const cells = await d.evaluate(`[...document.querySelectorAll(".cell")].filter((c) => {
      const r = c.getBoundingClientRect(); const m = c.closest("main").getBoundingClientRect();
      return r.top >= m.top && r.bottom <= m.bottom;
    }).slice(0, 10).map((c) => { const r = c.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })`);
    await d.clickAt(cells[0].x, cells[0].y);
    await d.clickAt(cells[9].x, cells[9].y, { modifiers: 8 });
    await d.sleep(800);

    // Counts nodes removed from the grid's thumbnails and the panel, and whether the panel
    // ever showed its empty hint.
    await d.evaluate(`(() => {
      const w = (window.__check33 = { imgs: 0, panel: 0, empty: 0 });
      const grid = document.querySelector(".cell").closest("main");
      const panel = document.querySelector("aside.info-panel");
      new MutationObserver((list) => {
        for (const m of list) for (const n of m.removedNodes) if (n.nodeName === "IMG") w.imgs++;
      }).observe(grid, { childList: true, subtree: true });
      new MutationObserver((list) => {
        for (const m of list) {
          w.panel += m.removedNodes.length;
          for (const n of m.addedNodes) if (n.classList?.contains("hint") && /Select a photo/.test(n.textContent)) w.empty++;
        }
      }).observe(panel, { childList: true, subtree: true });
    })()`);
    const take = () => d.evaluate(`(() => { const w = window.__check33; const out = { ...w }; w.imgs = w.panel = w.empty = 0; return out; })()`);
    const results = {};
    let edits = 0;
    try {
      await d.key("4");
      edits++;
      await d.sleep(1500);
      results.rate = await take();
      await d.key("z", { modifiers: 2 });
      edits--;
      await d.sleep(1500);
      results.undo = await take();

      // Ctrl+K, a keyword, Enter: focus stays in the keyword input.
      await d.key("k", { modifiers: 2 });
      await d.type("c1check", 20);
      await d.key("Enter");
      edits++;
      await d.sleep(1500);
      results.keyword = { ...(await take()), focus: await d.evaluate(`document.activeElement?.id || document.activeElement?.tagName`) };
      await d.evaluate(`document.activeElement?.blur()`);

      // The panel scrolled to Location: an edit keeps its scroll.
      await d.evaluate(`document.getElementById("place-country")?.scrollIntoView({ block: "center" })`);
      await d.sleep(300);
      const before = await d.evaluate(`document.querySelector("aside.info-panel").scrollTop`);
      await d.key("3");
      edits++;
      await d.sleep(1500);
      const after = await d.evaluate(`document.querySelector("aside.info-panel").scrollTop`);
      results.scroll = { ...(await take()), before, after };
    } finally {
      await d.evaluate(`document.activeElement?.blur()`);
      for (let i = 0; i < edits; i++) {
        await d.key("z", { modifiers: 2 });
        await d.sleep(300);
      }
    }
    console.log(JSON.stringify(results, null, 2));
    console.log("expected: imgs 0, empty 0, panel a few nodes at most (values), focus keyword-input, scroll before = after");
  },
};
