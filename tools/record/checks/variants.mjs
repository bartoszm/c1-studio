// Variants: the number on a tile, the header's Variants button, and one image's variants
// opened from the number and with `v`, left with Esc. Real mouse and key input; nothing is
// written to the catalog.
// Expected: every step ok, and "ALL OK" at the end.
//
// usage: node tools/record/drive.mjs tools/record/checks/variants.mjs --port 9237
const COUNT = `(() => {
  const t = document.querySelector("header .count")?.innerText ?? "";
  const m = t.match(/^([\\d,.\\s\\u00a0]+) photos · ([\\d.]+) ms/);
  return m ? { total: Number(m[1].replace(/[^\\d]/g, "")), ms: Number(m[2]) } : null;
})()`;
const MARK = `(() => {
  const m = document.querySelector("main").getBoundingClientRect();
  const mark = [...document.querySelectorAll(".cell .variants-mark")].find((e) => {
    const r = e.closest(".cell").getBoundingClientRect();
    return r.top >= m.top && r.bottom <= m.bottom;
  });
  if (!mark) return null;
  const r = mark.getBoundingClientRect();
  const c = mark.closest(".cell").getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2, cx: c.x + c.width / 2, cy: c.y + c.height / 2, n: Number(mark.innerText), collapsed: mark.classList.contains("collapsed") };
})()`;
const CELLS = `document.querySelectorAll(".cell img").length`;
const CHIP = `[...document.querySelectorAll(".chip .text")].some((e) => e.innerText === "one image's variants")`;
const TOGGLE = `[...document.querySelectorAll("header button.toggle")].find((b) => b.innerText.trim() === "Variants")`;

export default {
  name: "check-variants",
  title: "Variants: marked, collapsed, opened",
  async run(d) {
    await d.waitFor(".cell img", null, { timeout: 900000 });
    await d.reset();
    if (await d.evaluate(`${TOGGLE}?.classList.contains("on")`)) await d.press("header button.toggle", /^Variants$/);
    await d.settle();
    const results = [];
    const check = (step, ok, got) => {
      results.push({ step, ok });
      console.log(JSON.stringify({ step, ok, got }));
    };
    const key = async (k, o) => { await d.key(k, o); await d.sleep(600); };
    const settled = async () => { await d.sleep(800); await d.settle(); };

    // Scroll until a tile with a variants number is on screen.
    const findMark = async () => {
      await d.evaluate(`document.querySelector("main").scrollTop = 0`);
      for (let i = 0; i < 400; i++) {
        await d.sleep(120);
        const m = await d.evaluate(MARK);
        if (m) return m;
        await d.evaluate(`(() => { const m = document.querySelector("main"); m.scrollTop += m.clientHeight * 0.9; })()`);
      }
      return null;
    };

    const all = await d.evaluate(COUNT);
    const mark = await findMark();
    check("a tile shows how many variants its image has", !!mark && mark.n > 1 && !mark.collapsed, mark);
    if (!mark) return console.log("FAILED: no variants found");

    await d.click("header button.toggle", /^Variants$/);
    await settled();
    const collapsed = await d.evaluate(COUNT);
    check("Variants collapses: fewer photos", collapsed.total < all.total, { all, collapsed });
    const cmark = await findMark();
    check("a collapsed tile's number shows a card behind it", !!cmark?.collapsed, cmark);

    await d.clickAt(cmark.x, cmark.y);
    await settled();
    const opened = await d.evaluate(COUNT);
    check("clicking the number shows every variant of the image", opened.total === cmark.n && (await d.evaluate(CHIP)), { opened, n: cmark.n });
    check("…and as many tiles", (await d.evaluate(CELLS)) === cmark.n, await d.evaluate(CELLS));

    await key("Escape");
    await settled();
    check("Esc goes back to the collapsed grid", (await d.evaluate(COUNT)).total === collapsed.total && !(await d.evaluate(CHIP)), await d.evaluate(COUNT));

    const again = await findMark();
    await d.clickAt(again.cx, again.cy);
    await d.sleep(300);
    await key("v");
    await settled();
    check("v on the focused tile shows its image's variants", (await d.evaluate(COUNT)).total === again.n, await d.evaluate(COUNT));
    await key("Escape");
    await settled();

    await d.press("header button.toggle", /^Variants$/);
    await settled();
    check("Variants off: every photo again", (await d.evaluate(COUNT)).total === all.total, await d.evaluate(COUNT));

    const failed = results.filter((r) => !r.ok);
    console.log(failed.length ? `FAILED ${failed.length} of ${results.length}` : `ALL OK (${results.length})`);
  },
};
