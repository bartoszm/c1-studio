// #35, #36, #37: editing several photos at once. Samples, every frame, where the "Edit N
// photos" block's fields are in the details panel, and the header's search box and Copy
// button, from the moment the block shows: through Ctrl+A, a rating, a keyword, a country
// and their undos. Also notes whether the footer said "Editing …" during an edit.
// Expected: every `moved` value 0 (fields and header), `editingSeen` true for the big pick.
//
// usage: node tools/record/drive.mjs tools/record/checks/batch-fields.mjs --port 9223
const FIELDS = ["keyword-input", "place-country", "place-country_code", "place-city", "place-geo", "iptc-headline", "iptc-description"];

export default {
  name: "check-batch-fields",
  title: "Batch editing keeps its fields still",
  async run(d) {
    await d.reset();
    await d.click("input.search");
    await d.type("market", 0);
    await d.key("Enter");
    await d.key("Escape");
    await d.settle();
    const cells = await d.evaluate(`[...document.querySelectorAll(".cell")].slice(0, 10).map((c) => { const r = c.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })`);
    await d.clickAt(cells[0].x, cells[0].y);
    await d.clickAt(cells[9].x, cells[9].y, { modifiers: 8 });
    // From the moment the block shows, while the selection is still being read.
    await d.waitFor(".info-panel .batch", null, { timeout: 15000 });

    await d.evaluate(`(() => {
      const ids = ${JSON.stringify(FIELDS)};
      const w = (window.__batch = { frames: 0, moved: {}, at: {}, editing: false, header: {} });
      const panel = () => document.querySelector("aside.info-panel");
      const tick = () => {
        w.frames++;
        const top = panel()?.getBoundingClientRect().top ?? 0;
        const scroll = panel()?.scrollTop ?? 0;
        const spots = Object.fromEntries(ids.map((id) => [id, document.getElementById(id)?.getBoundingClientRect().top]));
        const search = document.querySelector("input.search")?.getBoundingClientRect();
        const copy = document.querySelector(".copy-slot")?.getBoundingClientRect();
        spots["search width"] = search?.width;
        spots["copy slot x"] = copy?.x;
        for (const [id, at] of Object.entries(spots)) {
          if (at === undefined) continue;
          // Fields against the panel's content (its scroll counted), the header as it is.
          const v = id.includes(" ") ? at : at - top + scroll;
          if (w.at[id] !== undefined) w.moved[id] = Math.max(w.moved[id] ?? 0, Math.abs(v - w.at[id]));
          w.at[id] ??= v;
        }
        if (/^Editing /.test(document.querySelector("footer .busy")?.textContent ?? "")) w.editing = true;
        if (!w.stop) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    })()`);
    const take = (step) => d.evaluate(`(() => { const w = window.__batch; const out = { step: ${JSON.stringify(step)}, moved: { ...w.moved }, editingSeen: w.editing }; w.moved = {}; w.editing = false; return out; })()`);
    const results = [];
    let edits = 0;
    try {
      await d.sleep(1500);
      results.push(await take("the pick read"));
      await d.key("4");
      edits++;
      await d.sleep(1500);
      results.push(await take("rating on 10"));
      await d.key("k", { modifiers: 2 });
      await d.type("c1check", 20);
      await d.key("Enter");
      edits++;
      await d.sleep(1500);
      results.push(await take("keyword on 10"));
      await d.evaluate(`document.activeElement?.blur()`);
      await d.evaluate(`(() => { const i = document.getElementById("place-country"); i.focus(); i.value = "Atlantis"; i.dispatchEvent(new Event("input", { bubbles: true })); })()`);
      await d.key("Enter");
      edits++;
      await d.sleep(1500);
      results.push(await take("country on 10"));
      await d.evaluate(`document.activeElement?.blur()`);
      while (edits > 0) {
        await d.key("z", { modifiers: 2 });
        edits--;
        await d.sleep(1200);
      }
      results.push(await take("undos on 10"));
      // Every photo of the result (the block stays: still several picked).
      await d.key("a", { modifiers: 2 });
      await d.sleep(2500);
      results.push(await take("Ctrl+A"));
      await d.key("3");
      edits++;
      await d.sleep(2500);
      results.push(await take("rating on all"));
      await d.key("z", { modifiers: 2 });
      edits--;
      await d.sleep(2500);
      results.push(await take("undo on all"));
    } finally {
      await d.evaluate(`document.activeElement?.blur()`);
      for (let i = 0; i < edits; i++) {
        await d.key("z", { modifiers: 2 });
        await d.sleep(500);
      }
      await d.evaluate(`window.__batch && (window.__batch.stop = true)`);
    }
    for (const r of results) console.log(JSON.stringify(r));
    console.log("expected: every moved value 0; editingSeen true at least for the edits on all (a short edit may finish within a frame)");
  },
};
