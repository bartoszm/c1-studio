// Free text plus sidebar filters: every count narrows as you go.
export default {
  name: "search",
  title: "Find anything, instantly",
  async run(d) {
    await d.reset();
    await d.record(async () => {
      await d.caption("Search keywords, captions, places and file names");
      await d.click("input.search");
      await d.type("hong kong street", 70);
      await d.key("Enter");
      await d.sleep(600);
      await d.settle();
      await d.sleep(1200);
      await d.caption("Every count in the sidebar follows the search");
      await d.click("aside button.label", /^2025\n/);
      await d.sleep(500);
      await d.settle();
      await d.sleep(900);
      await d.click("aside .facet", /^★★★★\n/);
      await d.sleep(500);
      await d.settle();
      await d.sleep(900);
      await d.caption("Each filter is a chip: × drops it");
      await d.click(".chip .drop");
      await d.sleep(500);
      await d.settle();
      await d.sleep(1400);
    });
  },
};
