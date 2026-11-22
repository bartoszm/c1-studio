// Choosing the best of alike photos: `b` opens the first series, the arrows move through
// its photos and the groups, Enter keeps the active photo and trashes the rest (the app's
// own trash), and Ctrl+Z takes the pick back. Real key
// input; the pick is undone at the end, and nothing is written to the catalog.
// Needs a library whose photos are measured (the Analysis tab) with at least two series.
// Expected: every step ok, and "ALL OK" at the end.
//
// usage: node tools/record/drive.mjs tools/record/checks/cull.mjs --port 9237
const BAR = `document.querySelector(".cull .where strong")?.innerText ?? null`;
const ACTIVE = `Number(document.querySelector(".cull .strip .frame.active")?.dataset.row ?? -1)`;
const FRAMES = `[...document.querySelectorAll(".cull .strip .frame")].map((e) => Number(e.dataset.row))`;
const SUGGESTED = `Number(document.querySelector(".cull .strip .frame .rank.best")?.closest(".frame").dataset.row ?? -1)`;
const OPEN = `!!document.querySelector(".cull")`;

export default {
  name: "check-cull",
  title: "Best of: a series gone through, a pick made and taken back",
  async run(d) {
    await d.waitFor(".cell img", null, { timeout: 900000 });
    await d.reset();
    const results = [];
    const check = (step, ok, got) => {
      results.push({ step, ok });
      console.log(JSON.stringify({ step, ok, got }));
    };
    const key = async (k, o) => { await d.key(k, o); await d.sleep(700); };
    const until = async (js, ms = 20000) => {
      for (let t = 0; t < ms; t += 250) {
        if (await d.evaluate(js)) return true;
        await d.sleep(250);
      }
      return false;
    };
    const group = (id) => d.invoke("cull_group", { filter: {}, id });
    const stacked = async (id) => (await d.invoke("query", { filter: { group: id, stacked: true }, options: {} })).rows;

    const page = await d.invoke("groups", { filter: {}, series: true, sort: "Oldest", reverse: false, offset: 0, limit: 3 });
    if (page.total < 2) return console.log(`FAILED: ${page.total} series in this library; measure the photos first (Analysis)`);
    const [first, second] = page.groups.map((g) => g.id);
    const before = await group(first);
    const rows = before.photos.map((p) => p.row);
    const wasStacked = await stacked(first);

    try { await d.evaluate(`localStorage.setItem("cullKind", "series")`); } catch { /* the default */ }
    await key("b");
    await until(`${FRAMES}.length > 0`);
    check("b opens the first series", (await d.evaluate(OPEN)) && /series 1 of/.test((await d.evaluate(BAR)) ?? ""), await d.evaluate(BAR));
    check("its photos are in the strip, oldest first", JSON.stringify(await d.evaluate(FRAMES)) === JSON.stringify(rows), { rows, strip: await d.evaluate(FRAMES) });
    check("the suggested photo is the active one", (await d.evaluate(ACTIVE)) === before.order[0] && (await d.evaluate(SUGGESTED)) === before.order[0], { active: await d.evaluate(ACTIVE), order: before.order });

    await key("ArrowRight");
    const next = rows[(rows.indexOf(before.order[0]) + 1) % rows.length];
    check("→ moves to the next photo of the group", (await d.evaluate(ACTIVE)) === next, { active: await d.evaluate(ACTIVE), next });
    await key("g");
    check("g goes back to the suggested photo", (await d.evaluate(ACTIVE)) === before.order[0], await d.evaluate(ACTIVE));

    await key("ArrowDown");
    await until(`/series 2 of/.test(${BAR} ?? "")`);
    check("↓ shows the next group", /series 2 of/.test((await d.evaluate(BAR)) ?? ""), await d.evaluate(BAR));
    await key("ArrowUp");
    await until(`/series 1 of/.test(${BAR} ?? "")`);
    await until(`${ACTIVE} === ${before.order[0]}`);

    // Keep a photo that is not the suggested one, so Stack has to follow the pick.
    const keep = before.order[before.order.length - 1];
    while ((await d.evaluate(ACTIVE)) !== keep) await key("ArrowRight");
    await d.choose(".cull .rest select", "trash");
    await key("Enter");
    await until(`/series 2 of/.test(${BAR} ?? "")`);
    const after = await group(first);
    check("Enter keeps the active photo and trashes the rest", after.photos.length === 1 && after.photos[0].row === keep && after.photos[0].kept, after.photos.map((p) => [p.row, p.kept]));
    check("…and shows the next group", /series 2 of/.test((await d.evaluate(BAR)) ?? ""), await d.evaluate(BAR));
    check("only the photo kept is left of the group", JSON.stringify(await stacked(first)) === JSON.stringify([keep]), await stacked(first));
    const queued = await d.invoke("pending");
    check("nothing is queued for the source", (queued.edits ?? queued).length === 0, queued);

    await key("z", { modifiers: 2 });
    await until(`/series 1 of/.test(${BAR} ?? "")`);
    await until(`${FRAMES}.length === ${rows.length}`);
    const undone = await group(first);
    check("Ctrl+Z brings the group back, and shows it", undone.photos.length === rows.length && undone.photos.every((p) => !p.kept) && /series 1 of/.test((await d.evaluate(BAR)) ?? ""), { photos: undone.photos.length, bar: await d.evaluate(BAR) });
    check("…with every photo it had", JSON.stringify(await stacked(first)) === JSON.stringify(wasStacked), { now: await stacked(first), wasStacked });

    await key("Escape");
    check("Esc closes it", !(await d.evaluate(OPEN)), await d.evaluate(OPEN));
    void second;

    const failed = results.filter((r) => !r.ok);
    console.log(failed.length ? `FAILED ${failed.length} of ${results.length}` : `ALL OK (${results.length})`);
  },
};
