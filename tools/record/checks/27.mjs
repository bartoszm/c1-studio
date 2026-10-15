// #27 and #32 on the Analysis tab.
// #27: with *red* picked, prints the series' and duplicates' badges. Then opens a series
// two ways, its card and the details panel's link on one of its photos, and prints the
// grid's total each time against the badge of that series' card. Expected: no badge of 1,
// "N of M" where the filter splits a group, and each total equal to its badge's N.
// #32: searches for something no photo has and prints what the charts say. Expected:
// "Nothing matches the filter." and no "0 of 0 photos measured".
// On a fresh copy, let the analysis finish first (the footer's "measuring photos" gone):
// before that there are few groups to compare.
//
// usage: node tools/record/drive.mjs tools/record/checks/27.mjs --port 9223

/** The number of photos the grid shows. */
const total = (d) => d.evaluate(`+document.querySelector("header .count")?.textContent.split(" photos")[0].replace(/\\D/g, "")`);
const series = `[...document.querySelectorAll(".analysis .alike")].find((s) => /Series/.test(s.querySelector("h3").textContent))`;
/** The badge of the series card showing the thumbnail `src`, or of the first card. */
const badge = (d, src) =>
  d.evaluate(`(() => {
    const cards = [...(${series}?.querySelectorAll(".group") ?? [])];
    const card = ${JSON.stringify(src ?? null)} ? cards.find((c) => [...c.querySelectorAll("img")].some((i) => i.src === ${JSON.stringify(src ?? "")})) : cards[0];
    return card?.querySelector(".size").textContent.trim() ?? null;
  })()`);
const matched = (b) => (b ? +b.split(" of ")[0].replace(/\D/g, "") : null);

/** Clicks the grid's photo of a series (one of `srcs`, on screen), then the panel's link to
 *  its series, and compares the grid with that series' card. */
async function panelLink(d, srcs) {
  await d.sleep(500);
  const cell = await d.evaluate(`(() => { const img = [...document.querySelectorAll(".cell img")].find((i) => ${JSON.stringify(srcs)}.includes(i.src)); const r = img.closest(".cell").getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2, src: img.src }; })()`);
  await d.clickAt(cell.x, cell.y);
  await d.waitFor(".info-panel .look-link", /series/);
  await d.press(".info-panel .look-link", /series/);
  await d.sleep(2000);
  const fromPanel = await total(d);
  await d.press(".tabs button", /^Analysis$/);
  await d.sleep(2500);
  const its = await badge(d, cell.src);
  console.log(JSON.stringify({ step: "from the panel's link", badge: its, grid: fromPanel, ok: its ? fromPanel === matched(its) : "not listed" }));
}

async function analysisRed(d) {
  await d.reset();
  await d.press(".tabs button", /^Analysis$/);
  await d.waitFor(".analysis .bar-row");
  await d.press(".analysis .bar-row", /^red/);
  await d.sleep(2500);
}

export default {
  name: "check-27",
  title: "Analysis groups and empty charts",
  async run(d) {
    await analysisRed(d);
    const badges = await d.evaluate(`Object.fromEntries([...document.querySelectorAll(".analysis .alike")].map((s) =>
      [s.querySelector("h3").firstChild.textContent.trim(), [...s.querySelectorAll(".group .size")].slice(0, 12).map((b) => b.textContent.trim())]))`);
    console.log(JSON.stringify({ step: "red", badges }));

    // The card.
    const first = await badge(d);
    if (!first) {
      console.log(JSON.stringify({ step: "series", note: "no series among the red photos" }));
    } else {
      await d.evaluate(`${series}.querySelector(".group").click()`);
      await d.sleep(2000);
      const n = await total(d);
      console.log(JSON.stringify({ step: "from its card", badge: first, grid: n, ok: n === matched(first) }));

      // The panel's link, from a photo of a listed series picked in the grid: the series'
      // photos only, then the grid scrolled until one of the first card's shows.
      await analysisRed(d);
      const srcs = await d.evaluate(`[...${series}.querySelector(".group").querySelectorAll("img")].map((i) => i.src)`);
      await d.evaluate(`${series}.querySelector("h3 .link").click()`);
      await d.press(".tabs button", /^Photos$/);
      await d.sleep(2000);
      await d.waitFor(".cell img");
      let cell = null;
      for (let i = 0; i < 200 && !cell; i++) {
        cell = await d.evaluate(`(() => {
          const img = [...document.querySelectorAll(".cell img")].find((i) => ${JSON.stringify(srcs)}.includes(i.src));
          if (!img) return null;
          img.closest(".cell").scrollIntoView({ block: "center" });
          const r = img.closest(".cell").getBoundingClientRect();
          return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
        })()`);
        if (cell) break;
        const moved = await d.evaluate(`(() => { const m = document.querySelector(".cell").closest("main"); const at = m.scrollTop; m.scrollTop += m.clientHeight; return m.scrollTop !== at; })()`);
        if (!moved) break;
        await d.sleep(300);
      }
      if (!cell) console.log(JSON.stringify({ step: "from the panel's link", note: "none of the first series' photos found in the grid" }));
      else await panelLink(d, srcs);
    }

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
