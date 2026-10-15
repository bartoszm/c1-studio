// #34: edits must not take the details panel's Look section away, so the fields below it
// stay put. Samples, every frame, whether Look is there and where Country, City, GPS and
// Headline are, across a rating, a keyword and five undos. Expected: Look never missing,
// 0 px movement.
//
// usage: node tools/record/drive.mjs tools/record/checks/34.mjs --port 9223
export default {
  name: "check-34",
  title: "Look stays through edits",
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
    await d.sleep(1000);
    const measured = await d.evaluate(`[...document.querySelectorAll("aside.info-panel h3")].some((h) => h.textContent === "Look")`);

    await d.evaluate(`(() => {
      const ids = ["place-country", "place-city", "place-geo", "iptc-headline"];
      const w = (window.__check34 = { frames: 0, noLook: 0, moved: Object.fromEntries(ids.map((id) => [id, 0])), at: {} });
      const tick = () => {
        w.frames++;
        if (![...document.querySelectorAll("aside.info-panel h3")].some((h) => h.textContent === "Look")) w.noLook++;
        for (const id of ids) {
          const top = document.getElementById(id)?.getBoundingClientRect().top;
          if (top === undefined) continue;
          if (w.at[id] !== undefined) w.moved[id] = Math.max(w.moved[id], Math.abs(top - w.at[id]));
          w.at[id] ??= top;
        }
        if (!w.stop) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    })()`);
    let edits = 0;
    try {
      await d.key("4");
      edits++;
      await d.sleep(800);
      await d.key("k", { modifiers: 2 });
      await d.type("c1check", 20);
      await d.key("Enter");
      edits++;
      await d.sleep(800);
      await d.evaluate(`document.activeElement?.blur()`);
      while (edits > 0) {
        await d.key("z", { modifiers: 2 });
        edits--;
        await d.sleep(800);
      }
      for (let i = 0; i < 3; i++) {
        await d.key("y", { modifiers: 2 });
        await d.sleep(400);
        await d.key("z", { modifiers: 2 });
        await d.sleep(400);
      }
    } finally {
      await d.evaluate(`document.activeElement?.blur()`);
      for (let i = 0; i < edits; i++) {
        await d.key("z", { modifiers: 2 });
        await d.sleep(300);
      }
    }
    const w = await d.evaluate(`(() => { window.__check34.stop = true; const { frames, noLook, moved } = window.__check34; return { frames, noLook, moved }; })()`);
    console.log(JSON.stringify({ lookShownAtStart: measured, ...w }, null, 2));
    console.log("expected: noLook 0 (when the photo is measured), every moved value 0");
  },
};
