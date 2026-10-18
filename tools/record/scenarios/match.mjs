// Best match: the photos that fit the picked colours, light and focus best come first.
export default {
  name: "match",
  title: "The best match first",
  async run(d) {
    await d.reset();
    const sort = 'header select[aria-label="Sort photos by"]';
    await d.press(".tabs button", /^Analysis$/);
    await d.waitFor(".analysis .card");
    if (await d.locate(".parts button", /^Look/)) await d.press(".parts button", /^Look/);
    await d.evaluate(`document.querySelector(".analysis")?.scrollTo({ top: 0 })`);
    await d.sleep(600);
    await d.record(async () => {
      await d.caption("Pick what you are after: red and blue");
      await d.click(".analysis .bar-row", /^red\n/);
      await d.sleep(800);
      await d.click(".analysis .bar-row", /^blue\n/);
      await d.sleep(1000);
      await d.click(".tabs button", /^Photos$/);
      await d.settle();
      await d.sleep(1200);
      await d.caption("Best match: the reddest and bluest photos first");
      await d.move(...Object.values(await d.locate(sort)).slice(0, 2));
      await d.choose(sort, "match");
      await d.settle();
      await d.sleep(2600);
      await d.caption("Add a light and a focus pick: dark and sharp, scores averaged");
      await d.click(".tabs button", /^Analysis$/);
      await d.waitFor(".analysis .card");
      await d.click(".analysis .bar-row", /^dark\n/);
      await d.sleep(700);
      await d.click(".analysis .bar-row", /^sharp\n/);
      await d.sleep(900);
      await d.click(".tabs button", /^Photos$/);
      await d.settle();
      await d.sleep(2600);
      await d.caption("… or the worst match first");
      await d.click("header .sort .dir");
      await d.settle();
      await d.sleep(2200);
    });
  },
};
