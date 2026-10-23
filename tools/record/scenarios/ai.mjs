// Describing photos with AI: pick photos, the estimate, a run, the cards, and one accepted.
// This one really calls the model, on the Claude plan through Claude Code (so Claude Code
// must be signed in), with four photos. The accepted edit is undone at the end; what was
// still to review before is rejected off camera, so the clip shows only this run's cards.
export default {
  name: "ai",
  title: "Describe photos with AI",
  async run(d) {
    // The map clips leave the grid following the map's area: let go of it.
    await d.press(".tabs button", /^Map$/);
    await d.waitFor(".map-tools .follow input", null, { timeout: 30000 });
    await d.evaluate(`(() => { const f = document.querySelector(".map-tools .follow input"); if (f.checked) f.click(); })()`);
    await d.reset();
    const view = await d.invoke("ai_settings");
    if (view.settings.access !== "plan") await d.invoke("set_ai_settings", { settings: { ...view.settings, access: "plan" } });
    const old = (await d.invoke("suggestions", { status: "Proposed" })).map((r) => r.row);
    if (old.length) await d.invoke("set_suggestion_status", { rows: old, status: "Rejected" });
    await d.click("input.search");
    await d.type("tokyo street", 0);
    await d.key("Enter");
    await d.key("Escape");
    await d.press(".tabs button", /^AI$/);
    await d.waitFor(".ai .bar button", /^Describe/, { timeout: 30000 });
    // Photos described before would be left out: describe them again.
    await d.evaluate(`(() => { const a = [...document.querySelectorAll(".ai .run label")].find((l) => l.textContent.includes("again"))?.querySelector("input"); if (a && !a.checked) a.click(); })()`);
    await d.settle();
    const cells = await d.evaluate(`[...document.querySelectorAll(".cell")].slice(0, 6).map((c) => { const r = c.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })`);
    let accepted = false;
    try {
      await d.record(async () => {
        await d.caption("Pick photos, and the AI tab asks a vision model about them");
        await d.clickAt(cells[0].x, cells[0].y);
        await d.sleep(400);
        await d.clickAt(cells[3].x, cells[3].y, { modifiers: 8 });
        await d.sleep(900);
        await d.click(".ai .bar button", /^Describe 4 photos/);
        await d.waitFor(".ai .bar button.primary", /^Describe$/);
        await d.caption("First what it takes: here Claude, on the Claude plan through Claude Code");
        await d.sleep(2800);
        await d.caption("Nothing is sent before you start; the answers come in as they are ready");
        await d.click(".ai .bar button.primary", /^Describe$/);
        await d.sleep(4000);
        await d.cut(() =>
          d.waitFor(null, null, {
            timeout: 300000,
            fn: `window.__TAURI_INTERNALS__.invoke("describe_state").then((s) => !!s && !s.running)`,
          }),
        );
        await d.waitFor(".ai .card", null, { timeout: 30000 });
        await d.settle();
        await d.caption("A card per photo: caption, headline, keywords from your own, a rating, and the place level by level");
        await d.sleep(4200);
        await d.evaluate(`document.querySelector(".ai .cards .card:nth-child(2)")?.scrollIntoView({ block: "start", behavior: "smooth" })`);
        await d.sleep(2600);
        await d.evaluate(`document.querySelector(".ai .cards .card")?.scrollIntoView({ block: "start", behavior: "smooth" })`);
        await d.sleep(1200);
        await d.caption("Tick what fits and accept: it becomes edits in the queue, marked AI, and Ctrl+Z undoes it");
        await d.click(".ai .card .actions button.primary", /^Accept$/);
        accepted = true;
        await d.waitFor("footer button", /edits? waiting/, { timeout: 30000 }).catch(() => {});
        await d.sleep(3000);
      });
    } finally {
      if (accepted) {
        await d.key("z", { modifiers: 2 });
        await d.sleep(800);
      }
    }
  },
};
