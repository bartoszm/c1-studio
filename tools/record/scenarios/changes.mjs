// The History tab: the library over time, two dates compared, and the change log. It picks
// the days the recording copy's change log saw, so the numbers have something to show.
export default {
  name: "changes",
  title: "History: the catalog over time",
  async run(d) {
    await d.reset();
    await d.press(".tabs button", /^History$/);
    await d.waitFor(".growth svg", null, { timeout: 30000 });
    await d.settle();
    await d.sleep(1500);
    // The stretch the change log covers, in the chart's pixels.
    const logged = await d.evaluate(`(() => {
      const svg = document.querySelector(".growth svg").getBoundingClientRect();
      const marks = [...document.querySelectorAll(".growth rect.logged")].map((r) => r.getBoundingClientRect());
      if (!marks.length) return null;
      return { x0: Math.min(...marks.map((r) => r.x)), x1: Math.max(...marks.map((r) => r.right)), y: svg.y + 30, right: svg.right };
    })()`);
    if (!logged) throw new Error("the change log has no days to compare");
    await d.record(async () => {
      await d.caption("History: photos imported per day, the running total, and the days the change log saw changes");
      await d.move(logged.x0 - 120, logged.y, 900);
      await d.sleep(2600);
      await d.caption("Drag across it to compare two dates");
      await d.drag(Math.max(logged.x0 - 6, logged.x1 - 160), logged.y, Math.min(logged.x1 + 4, logged.right - 2), logged.y, { ms: 900 });
      await d.waitFor(".compare .numbers", null, { timeout: 30000 });
      await d.sleep(3200);
      await d.caption("Photos changed, per field, keywords gained and lost, albums: every number shows its photos");
      const num = await d.locate(".compare .numbers button.num");
      if (num) {
        await d.clickAt(num.x, num.y);
        await d.sleep(600);
        await d.settle();
      }
      await d.sleep(3000);
      await d.caption("Under it, the change log, in groups");
      await d.evaluate(`document.querySelector(".history-pane .scroll").scrollTo({ top: 99999, behavior: "smooth" })`);
      await d.sleep(3200);
    });
  },
};
