// What the photos look like: shape, main colour, light and focus, duplicates and series.
export default {
  name: "analysis",
  title: "See what your photos look like",
  async run(d) {
    await d.reset();
    await d.record(async () => {
      await d.caption("Every photo is measured once: shape, colour, light, focus");
      await d.click(".tabs button", /^Analysis$/);
      await d.waitFor(".analysis .card");
      await d.sleep(1600);
      await d.caption("Each bar is a filter: say, red photos with blocked shadows");
      await d.click(".analysis .bar-row", /^red\n/);
      await d.sleep(1300);
      await d.click(".analysis .bar-row", /^blocked shadows\n/);
      await d.sleep(1500);
      await d.caption("Duplicates and bursts across the whole library");
      await d.evaluate(`(() => {
        const a = document.querySelector(".analysis .alike");
        const s = a.closest(".analysis");
        s.scrollTo({ top: a.offsetTop - 60, behavior: "smooth" });
      })()`);
      await d.sleep(1200);
      await d.settle();
      await d.sleep(1200);
      await d.caption("The grid shows what the filters pick");
      await d.click(".tabs button", /^Photos$/);
      await d.sleep(500);
      await d.settle();
      await d.sleep(1800);
    });
  },
};
