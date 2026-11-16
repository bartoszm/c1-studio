// How filters combine: a click picks only a value, Ctrl+click adds one, Alt+click leaves
// one out, a click in the filter bar flips it, and/or switches, and Shift+click makes
// sections alternatives (Kraków or black & white).
//
// The catalog must have three cameras and two keywords in the sidebar, Kraków under
// Poland in Location, and *black & white* in Main colour (measured photos). Everything is
// checked before recording starts, so a catalog without them fails with a reason and
// no half-made clip.

/** The visible names in a sidebar section (by its title), first `n`. */
const names = (d, section, n) =>
  d.evaluate(`(() => {
    const sec = [...document.querySelectorAll("aside section")].find((s) => s.querySelector(".head")?.textContent.includes(${JSON.stringify(section)}));
    return [...(sec?.querySelectorAll(".facet .name, .node .name") ?? [])].map((e) => e.textContent.trim()).slice(0, ${n});
  })()`);

/** Opens a tree section's branch until `target` shows: `top`, then whichever child holds it. */
const reveal = (d, section, top, target) =>
  d.evaluate(`(async () => {
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    const sec = [...document.querySelectorAll("aside section")].find((s) => s.querySelector(".head")?.textContent.includes(${JSON.stringify(section)}));
    if (!sec) return false;
    const nodes = () => [...sec.querySelectorAll(".node")];
    const nameOf = (n) => n.querySelector(".name")?.textContent.trim();
    const find = (name) => nodes().find((n) => nameOf(n) === name);
    const pad = (n) => parseInt(n.style.paddingLeft);
    const shown = () => { const t = find(${JSON.stringify(target)}); t?.scrollIntoView({ block: "center" }); return !!t; };
    const root = find(${JSON.stringify(top)});
    if (!root) return false;
    root.querySelector("button.chev:not(.open)")?.click();
    await sleep(150);
    if (shown()) return true;
    const all = nodes(), i = all.indexOf(find(${JSON.stringify(top)})), kids = [];
    for (let j = i + 1; j < all.length && pad(all[j]) > pad(all[i]); j++) if (pad(all[j]) === pad(all[i]) + 14) kids.push(nameOf(all[j]));
    for (const k of kids) {
      const c = find(k)?.querySelector("button.chev:not(.open)");
      if (!c) continue;
      c.click();
      await sleep(150);
      if (shown()) return true;
      find(k)?.querySelector("button.chev.open")?.click();
      await sleep(50);
    }
    return shown();
  })()`);

const CTRL = { modifiers: 2 };
const ALT = { modifiers: 1 };
const SHIFT = { modifiers: 8 };
const exact = (name) => new RegExp(`^${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\n`);

export default {
  name: "filters",
  title: "Filters: only, add, leave out, or",
  async run(d) {
    await d.reset();
    const cameras = await names(d, "Camera", 3);
    const keywords = await names(d, "Keywords", 2);
    if (cameras.length < 3) throw new Error("filters: needs three cameras in the sidebar's Camera section");
    if (keywords.length < 2) throw new Error("filters: needs two keywords in the sidebar's Keywords section");
    if (!(await reveal(d, "Location", "Poland", "Kraków"))) throw new Error("filters: no Kraków under Poland in Location");
    if (!(await d.locate("aside .facet", /^black & white\n/))) throw new Error("filters: no “black & white” in Main colour (measure the photos first)");
    const pause = async (ms) => {
      await d.sleep(400);
      await d.settle();
      await d.sleep(ms);
    };
    await d.record(async () => {
      await d.caption("A click shows only that value");
      await d.click("aside .facet", exact(cameras[0]));
      await pause(900);
      await d.click("aside .facet", exact(cameras[1]));
      await pause(1200);
      await d.caption("Ctrl+click adds one: the filter bar says “or”");
      await d.click("aside .facet", exact(cameras[0]), CTRL);
      await pause(1600);
      await d.caption("Alt+click leaves one out: red, struck through, “except”");
      await d.click("aside .facet", exact(cameras[2]), ALT);
      await pause(1800);
      await d.caption("Click a value in the filter bar to leave it out, or show it again");
      await d.click(".pill button.flip", cameras[1]);
      await pause(1600);
      await d.click(".pill button.flip", cameras[1]);
      await pause(1200);
      await d.caption("× drops a whole section");
      await d.click(".pill .drop.all");
      await pause(1000);

      await d.caption("Two keywords: photos with both. Click “and” for either");
      await d.click("aside button.label", exact(keywords[0]));
      await pause(600);
      await d.click("aside button.label", exact(keywords[1]), CTRL);
      await pause(1600);
      await d.click(".pill button.join.switch", /^and$/);
      await pause(1800);
      await d.click(".pill .drop.all");
      await pause(800);

      await d.caption("Shift+click: or across sections. Kraków…");
      await d.cut(() => reveal(d, "Location", "Poland", "Kraków"));
      await d.click("aside button.label", /^Kraków\n/, SHIFT);
      await pause(1200);
      await d.caption("…or black & white: either one, as the bracket says");
      await d.click("aside .facet", /^black & white\n/, SHIFT);
      await pause(2200);
      await d.caption("Click “or” to make it “and” again; ? beside the search box has it all");
      await d.click(".either button.join.switch", /^or$/);
      await pause(1800);
    });
  },
};
