// Neighbours: places made more specific, and ≈ positions moved to GPS, from the photos
// taken moments before and after. The scenario undoes what it applied, so it can be
// recorded again.
export default {
  name: "refine",
  title: "Places from neighbours in time",
  async run(d) {
    await d.reset();
    await d.press(".tabs button", /^Map$/);
    await d.waitFor(".map-tools button", /^Neighbours…$/, { timeout: 30000 });
    await d.sleep(3000);
    let applied = false;
    try {
      await d.record(async () => {
        await d.caption("Neighbours… places photos from the ones taken just before and after");
        await d.click(".map-tools button", /^Neighbours…$/);
        await d.waitFor(".dialog h2", /^Place photos from their neighbours$/);
        await d.sleep(1800);
        await d.caption("A rough place gets a neighbour's more specific one, ≈ positions their GPS");
        // Show the places alone.
        await d.cut(async () => {
          await d.evaluate(`(() => { const b = [...document.querySelectorAll(".dialog .row label")].find((l) => l.textContent.includes("Position"))?.querySelector("input"); if (b?.checked) b.click(); })()`);
          await d.waitFor(".dialog .places li", null, { timeout: 60000 }).catch(() => {});
        });
        await d.sleep(3600);
        await d.caption("London becomes London › Barbican, Tokyo becomes Tokyo › Shinjuku: every change listed first");
        await d.sleep(3400);
        await d.caption("Apply queues them like any edit: Ctrl+Z undoes them");
        const apply = await d.locate(".dialog .buttons button.primary", /^Apply \d/);
        if (apply) {
          await d.clickAt(apply.x, apply.y);
          applied = true;
          await d.waitFor("footer button", /edits? waiting/, { timeout: 60000 }).catch(() => {});
          await d.sleep(2600);
        } else {
          await d.click(".dialog .buttons button", /^Cancel$/);
        }
      });
    } finally {
      if (applied) {
        await d.key("z", { modifiers: 2 });
        await d.sleep(1500);
      }
      if (await d.locate(".dialog .buttons button", /^Cancel$/)) await d.press(".dialog .buttons button", /^Cancel$/);
    }
  },
};
