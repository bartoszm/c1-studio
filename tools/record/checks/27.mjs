// #27 and #32 on the Analysis tab.
// #27: with *red* picked, prints the series' and duplicates' badges, then opens the first
// series from its card and from the details panel's link, and prints the grid's total
// each way. Expected: no badge of 1, "N of M" where the filter splits a group, and both
// totals equal to the first card's N.
// #32: searches for something no photo has and prints what the charts say. Expected:
// "Nothing matches the filter." and no "0 of 0 photos measured".
//
// usage: node tools/record/drive.mjs tools/record/checks/27.mjs --port 9223
const total = (d) => d.evaluate(`document.querySelector("header .count")?.textContent.trim()`);
const series = `[...document.querySelectorAll(".analysis .alike")].find((s) => /Series/.test(s.querySelector("h3").textContent))`;

export default {
  name: "check-27",
  title: "Analysis groups and empty charts",
  async run(d) {
    await d.reset();
    await d.press(".tabs button", /^Analysis$/);
    await d.waitFor(".analysis .bar-row");
    await d.press(".analysis .bar-row", /^red/);
    await d.sleep(2500);
    const badges = await d.evaluate(`Object.fromEntries([...document.querySelectorAll(".analysis .alike")].map((s) =>
      [s.querySelector("h3").firstChild.textContent.trim(), [...s.querySelectorAll(".group .size")].slice(0, 12).map((b) => b.textContent.trim())]))`);
    console.log(JSON.stringify({ step: "red", badges }));
    const first = await d.evaluate(`${series}?.querySelector(".group .size")?.textContent.trim() ?? null`);
    if (first) {
      await d.evaluate(`${series}.querySelector(".group").click()`);
      await d.sleep(2000);
      const fromCard = await total(d);
      const cell = await d.locate(".cell");
      await d.clickAt(cell.x, cell.y);
      await d.waitFor(".info-panel .look-link", /series/);
      await d.press(".info-panel .look-link", /series/);
      await d.sleep(2000);
      console.log(JSON.stringify({ step: "open the first series", badge: first, fromCard, fromPanel: await total(d) }));
    } else console.log(JSON.stringify({ step: "open the first series", note: "no series among the red photos" }));

    await d.reset();
    await d.click("input.search");
    await d.type("zzqqxnothing", 0);
    await d.key("Enter");
    await d.key("Escape");
    await d.press(".tabs button", /^Analysis$/);
    await d.sleep(2000);
    const empty = await d.evaluate(`({
      header: document.querySelector(".analysis .progress")?.textContent.trim().replace(/\\s+/g, " "),
      charts: [...document.querySelectorAll(".analysis .card .none")].map((p) => p.textContent.trim()),
    })`);
    console.log(JSON.stringify({ step: "nothing matches", ...empty }));
    await d.reset();
  },
};
