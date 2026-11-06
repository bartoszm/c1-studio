// #88: Enter and Space must not activate a grid cell that kept DOM focus after a click. Each
// step starts from a fresh click, with real mouse and key input, and compares the photo shown
// with the one expected. Nothing is written to the catalog.
// Expected: every step ok, and "ALL OK" at the end.
//
// usage: node tools/record/drive.mjs tools/record/checks/88.mjs --port 9237
const rowOf = `(e) => e?.src.split("/thumb/")[1] ?? null`;
const STATE = `(() => {
  const row = ${rowOf};
  return {
    viewer: row(document.querySelector(".viewer .stage img.base")),
    sel: row(document.querySelector(".cell.sel img")),
    picked: document.querySelectorAll(".cell.picked").length,
    dialog: !!document.querySelector('[role="dialog"][aria-modal="true"]'),
  };
})()`;
const CELLS = `(() => {
  const row = ${rowOf};
  const m = document.querySelector(".cell").closest("main").getBoundingClientRect();
  return [...document.querySelectorAll(".cell")].map((c) => {
    const r = c.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2, top: r.top, bottom: r.bottom, left: r.left, row: row(c.querySelector("img")) };
  }).filter((c) => c.top >= m.top && c.bottom <= m.bottom && c.row).sort((a, b) => a.top - b.top || a.left - b.left);
})()`;
const DETAILS = `[...document.querySelectorAll("header button.toggle")].find((b) => b.innerText.trim() === "Details")`;

export default {
  name: "check-88",
  title: "Enter opens the focused photo",
  async run(d) {
    await d.waitFor(".cell img", null, { timeout: 480000 });
    await d.reset();
    await d.settle();
    const c = await d.evaluate(CELLS);
    const results = [];
    const state = () => d.evaluate(STATE);
    const key = async (k, o) => { await d.key(k, o); await d.sleep(400); };
    const space = () => key(" ", { code: "Space" });
    const click = async (i, o) => { await d.clickAt(c[i].x, c[i].y, o); await d.sleep(300); };
    const open = async (i) => { await click(i); await click(i, { clickCount: 2 }); };
    const check = (step, got, want) => {
      const ok = Object.entries(want).every(([k, v]) => got[k] === v);
      results.push({ step, ok, want, got });
      console.log(JSON.stringify({ step, ok, ...(ok ? {} : { want, got }) }));
    };

    await click(0); await key("ArrowRight"); await key("ArrowRight"); await key("Enter");
    check("click, right twice, Enter opens the focused photo", await state(), { viewer: c[2].row });
    await key("Escape");

    await open(0); await key("ArrowRight"); await key("ArrowRight"); await key("Escape"); await key("Enter");
    check("viewer, right twice, Esc, Enter reopens the photo stepped to", await state(), { viewer: c[2].row });
    await key("Escape");

    await open(0); await key("ArrowRight"); await key("Enter");
    check("Enter in the viewer keeps the photo", await state(), { viewer: c[1].row });
    await key("Escape");

    await open(0); await key("ArrowRight"); await space();
    check("Space in the viewer keeps the photo", await state(), { viewer: c[1].row });
    await key("Escape");

    await click(0); await key("ArrowRight"); await space();
    check("Space in the grid keeps the focus", await state(), { sel: c[1].row, viewer: null });

    await click(0); await click(1, { modifiers: 2 }); await click(2, { modifiers: 2 }); await key("Enter");
    check("Enter with 3 picked opens the focused photo", await state(), { viewer: c[2].row });
    await key("Escape");
    check("all 3 stay picked after Esc", await state(), { picked: 3 });

    for (const [name, press] of [["Enter", () => key("Enter")], ["Space", space]]) {
      await click(0); await key("ArrowRight"); await key("c", { modifiers: 2 });
      const opened = (await state()).dialog;
      await press();
      check(`${name} with a dialog open leaves the photo behind it`, { ...(await state()), opened }, { opened: true, sel: c[1].row });
      await key("Escape");
    }

    await click(3);
    check("a click selects", await state(), { sel: c[3].row, viewer: null });
    await open(5);
    check("a double click opens", await state(), { viewer: c[5].row });
    await key("Escape");

    const before = await d.evaluate(`${DETAILS}.classList.contains("on")`);
    await d.evaluate(`${DETAILS}.focus()`);
    await key("Enter");
    const after = await d.evaluate(`${DETAILS}.classList.contains("on")`);
    check("Enter still activates a toolbar button", { toggled: before !== after }, { toggled: true });
    await key("Escape");

    const failed = results.filter((r) => !r.ok);
    console.log(failed.length ? `FAILED ${failed.length} of ${results.length}` : `ALL OK (${results.length})`);
  },
};
