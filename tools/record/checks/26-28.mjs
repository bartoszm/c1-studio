// #26 and #28 on the Analysis tab.
// #26: with *red* picked, prints how many photos the strip under the charts shows and the
// header's count against the "Show N photos" button, then clicks a photo in the strip and
// prints the details panel's name. Expected: cells on screen, the two counts equal, the
// panel naming the clicked photo, and the Analysis tab still open.
// #28: prints each list's card count and the total in its header, presses "Show more"
// where there is one and prints them again. Expected: no "300+", the total exact, and
// each press adding a page (60) or the rest, until the cards reach the total.
//
// usage: node tools/record/drive.mjs tools/record/checks/26-28.mjs --port 9223

/** The number of photos the grid shows. */
const total = (d) => d.evaluate(`+document.querySelector("header .count")?.textContent.split(" photos")[0].replace(/\\D/g, "")`);
const list = (title) =>
  `[...document.querySelectorAll(".analysis .alike")].find((s) => s.querySelector("h3").firstChild.textContent.trim() === ${JSON.stringify(title)})`;
/** A list's cards and the total in its header. */
const cards = (d, title) =>
  d.evaluate(`(() => {
    const s = ${list(title)};
    return {
      cards: s?.querySelectorAll(".group").length ?? 0,
      total: s ? +s.querySelector(".group-total").textContent.replace(/\\D/g, "") : null,
      header: s?.querySelector("h3 .totals").textContent.trim().replace(/\\s+/g, " "),
      more: s?.querySelector(".more")?.textContent.trim() ?? null,
    };
  })()`);

export default {
  name: "check-26-28",
  title: "Analysis: photos in a strip, every group",
  async run(d) {
    await d.reset();
    await d.press(".tabs button", /^Analysis$/);
    await d.waitFor(".analysis .bar-row");
    await d.press(".analysis .bar-row", /^red/);
    await d.sleep(2500);
    await d.settle();

    // #26: the strip.
    const strip = await d.evaluate(`(() => {
      const main = document.querySelector(".pane > main");
      const r = main?.getBoundingClientRect();
      const cells = [...document.querySelectorAll(".pane > main .cell")].filter((c) => {
        const b = c.getBoundingClientRect();
        return b.bottom > r.top && b.top < r.bottom && b.height > 0;
      });
      return { height: r ? Math.round(r.height) : null, cells: cells.length };
    })()`);
    const header = await total(d);
    const button = await d.evaluate(`+document.querySelector(".analysis .show")?.textContent.replace(/\\D/g, "")`);
    console.log(JSON.stringify({ step: "red", strip, header, button, ok: strip.cells > 0 && header === button }));

    const cell = await d.evaluate(`(() => {
      const c = document.querySelector(".pane > main .cell"); if (!c) return null;
      const r = c.getBoundingClientRect();
      return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
    })()`);
    if (!cell) {
      console.log(JSON.stringify({ step: "click", note: "no photo in the strip" }));
    } else {
      await d.clickAt(cell.x, cell.y);
      await d.waitFor(".info-panel h2");
      await d.sleep(500);
      const panel = await d.evaluate(`document.querySelector(".info-panel h2")?.textContent.trim()`);
      const tab = await d.evaluate(`document.querySelector(".tabs button.on")?.textContent.trim()`);
      console.log(JSON.stringify({ step: "click", panel, tab, ok: !!panel && tab === "Analysis" }));
    }

    // #28: every group, a page at a time.
    for (const title of ["Duplicates", "Series"]) {
      const before = await cards(d, title);
      console.log(JSON.stringify({ step: title, ...before, ok: !/\+/.test(before.header ?? "") }));
      if (!before.more) continue;
      await d.evaluate(`${list(title)}.querySelector(".more").click()`);
      await d.sleep(2000);
      const after = await cards(d, title);
      const added = after.cards - before.cards;
      console.log(JSON.stringify({
        step: `${title}: show more`,
        ...after,
        added,
        ok: after.total === before.total && added === Math.min(60, before.total - before.cards),
      }));
    }
    await d.reset();
  },
};
