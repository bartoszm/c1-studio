// Duplicates and series in one list, sorted; and stacking, one photo per burst in the grid.
export default {
  name: "alike",
  title: "Duplicates, bursts and stacking",
  async run(d) {
    await d.reset();
    await d.press(".tabs button", /^Analysis$/);
    await d.waitFor(".parts button", /^Alike/);
    await d.press(".parts button", /^Alike/);
    await d.waitFor(".alike .group", null, { timeout: 30000 });
    if (await d.locate(".alike .kinds button.on", /^Series/)) await d.press(".alike .kinds button", /^Duplicates/);
    await d.choose('.alike select[aria-label="Sort groups by"]', "Matched");
    await d.settle();
    await d.record(async () => {
      await d.caption("Duplicates (one moment saved twice) and series (bursts), across the library");
      await d.sleep(2200);
      await d.caption("Sorted as you like: by size, date or sharpness, either way");
      await d.choose('.alike select[aria-label="Sort groups by"]', "Size");
      await d.settle();
      await d.sleep(1600);
      await d.click(".alike .kinds button", /^Series/);
      await d.settle();
      await d.sleep(1800);
      await d.caption("Stack: the grid shows one photo per burst, its sharpest");
      const before = await d.evaluate(`document.querySelector("header .count")?.textContent`);
      await d.click("header button.toggle", /^Stack$/);
      await d.waitFor(null, null, { fn: `document.querySelector("header .count")?.textContent !== ${JSON.stringify(before)}` }).catch(() => {});
      await d.click(".tabs button", /^Photos$/);
      await d.settle();
      await d.sleep(2600);
    });
    await d.press("header button.toggle", /^Stack$/);
    await d.press(".tabs button", /^Analysis$/);
    await d.press(".alike .kinds button", /^Duplicates/);
    await d.choose('.alike select[aria-label="Sort groups by"]', "Matched");
    await d.press(".parts button", /^Look/);
  },
};
