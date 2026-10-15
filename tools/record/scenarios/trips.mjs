// Trips found from when and where photos were taken, and the timeline.
export default {
  name: "trips",
  title: "Trips and the timeline",
  async run(d) {
    await d.reset({ timeline: true });
    // The first time, the Trips section asks where home is.
    if (await d.locate("aside button.primary", /^Home is/)) {
      await d.press("aside button.primary", /^Home is/);
      await d.waitFor("aside .trip");
    }
    await d.evaluate(`document.querySelector("aside .trip").closest("section").scrollIntoView({ block: "start" })`);
    await d.sleep(400);
    await d.record(async () => {
      await d.caption("Trips are found from when and where the photos were taken");
      await d.sleep(1600);
      await d.click("aside .trip", /^Tokyo, Hong Kong\n/);
      await d.sleep(500);
      await d.settle();
      await d.sleep(1500);
      await d.caption("The timeline: photos per day, with trips as bands under the bars");
      await d.sleep(1200);
      await d.caption("Drag across it to pick those days");
      const t = await d.evaluate(`(() => { const r = document.querySelector(".timeline").getBoundingClientRect(); return { x: r.x, y: r.y + r.height * 0.45, w: r.width }; })()`);
      await d.drag(t.x + t.w * 0.42, t.y, t.x + t.w * 0.62, t.y, { ms: 1000 });
      await d.sleep(500);
      await d.settle();
      await d.sleep(2000);
    });
  },
};
