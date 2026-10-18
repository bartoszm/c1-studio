// Sorting: a field, and a direction button that says what it does.
export default {
  name: "sort",
  title: "Sort",
  async run(d) {
    await d.reset();
    const sort = 'header select[aria-label="Sort photos by"]';
    await d.record(async () => {
      await d.caption("Sort by a field: date, rating, colour label, sharpness, brightness…");
      await d.move(...Object.values(await d.locate(sort)).slice(0, 2));
      await d.sleep(500);
      await d.choose(sort, "rating");
      await d.settle();
      await d.sleep(1800);
      await d.caption("The button says which way; click it to turn the order round");
      await d.click("header .sort .dir");
      await d.settle();
      await d.sleep(1800);
      await d.caption("Sharpest first");
      await d.choose(sort, "sharpness");
      await d.settle();
      await d.sleep(1800);
      await d.caption("Brightest first");
      await d.choose(sort, "brightness");
      await d.settle();
      await d.sleep(1800);
    });
  },
};
