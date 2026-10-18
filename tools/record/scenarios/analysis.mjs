// What the photos look like: the Look charts and how they explain themselves, picks that
// combine as any or all, and why one photo is in its classes.
export default {
  name: "analysis",
  title: "See what your photos look like",
  async run(d) {
    await d.reset();
    await d.press(".tabs button", /^Analysis$/);
    await d.waitFor(".analysis .card");
    if (await d.locate(".parts button", /^Look/)) await d.press(".parts button", /^Look/);
    await d.evaluate(`document.querySelector(".analysis")?.scrollTo({ top: 0 })`);
    await d.sleep(800);
    await d.record(async () => {
      await d.caption("Every photo is measured once: shape, colour, light, focus");
      await d.sleep(1800);
      await d.caption("Each card says what it measures; each bar is a filter");
      await d.move(...Object.values(await d.locate(".analysis .card .hint", /colours/)).slice(0, 2));
      await d.sleep(1400);
      await d.click(".analysis .bar-row", /^red\n/);
      await d.sleep(1200);
      await d.click(".analysis .bar-row", /^blue\n/);
      await d.sleep(1300);
      await d.caption("Red or blue … or, switched to all, red and blue");
      await d.click(".analysis .combine button", /^all$/);
      await d.sleep(1800);
      await d.caption("Pick a photo: the details say why it is red, bright or sharp");
      await d.click(".tabs button", /^Photos$/);
      await d.sleep(400);
      await d.settle();
      const cell = await d.locate(".cell");
      await d.clickAt(cell.x, cell.y);
      await d.sleep(600);
      await d.evaluate(`(() => {
        const h = [...document.querySelectorAll(".info-panel h3")].find((e) => /look/i.test(e.textContent));
        h?.scrollIntoView({ block: "start", behavior: "smooth" });
      })()`);
      await d.sleep(3200);
    });
    // Back to "any" for the next clip.
    await d.press(".tabs button", /^Analysis$/);
    if (await d.locate(".analysis .combine button.on", /^all$/)) await d.press(".analysis .combine button", /^any$/);
  },
};
