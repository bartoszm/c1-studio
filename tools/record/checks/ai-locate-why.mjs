// AI Locate: why a photo was placed nowhere, whole scenes, the map from a card, and
// positions accepted as approximate in their own colour. Checks, on the running app, with
// proposed locate suggestions in its AI store (none are made: no call costs anything):
//   1. a card placed nowhere says why: the model's reason and clues;
//   2. a card placed in part says what keeps the rest open;
//   3. a scene of more photos than the card shows opens to show them all;
//   4. *Show on map* on a card with a position opens the map on its pin, its card open;
//   5. *Show on map* on a card with none picks its photos and says how to place them;
//   6. leaving the map and coming back does not go to the guess again;
//   7. a guess accepted as approximate is drawn in the AI's colour, and not at all when
//      Settings says so (the footer counts it).
// Steps whose kind of suggestion the store lacks are skipped, and say so.
//
// usage: node tools/record/drive.mjs tools/record/checks/ai-locate-why.mjs [shots dir] --port 9237
const out = process.argv.slice(2).find((a, i, all) => i > 0 && !a.startsWith("--") && all[i - 1] !== "--port");

const results = [];
const check = (name, ok, detail) => {
  results.push({ name, ok });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}: ${JSON.stringify(detail)}`);
};
const skip = (name, why) => console.log(`SKIP ${name}: ${why}`);

/** The card showing `row` (as its first photo), as the page has it. */
const card = (d, row) =>
  d.evaluate(`(() => {
    const c = [...document.querySelectorAll(".cards .card")].find((c) => c.querySelector(".thumb img")?.src.match(/thumb\\/(\\d+)/)?.[1] === "${row}");
    if (!c) return null;
    c.scrollIntoView({ block: "center" });
    return {
      unplaced: c.querySelector(".unplaced")?.innerText.trim() ?? null,
      reason: c.querySelector(".reason")?.innerText.trim() ?? null,
      strip: c.querySelectorAll(".scene-strip img").length,
      more: c.querySelector(".more")?.innerText.trim() ?? null,
      onmap: !!c.querySelector(".onmap"),
    };
  })()`);
const clickIn = (d, row, selector) =>
  d.evaluate(`(() => {
    const c = [...document.querySelectorAll(".cards .card")].find((c) => c.querySelector(".thumb img")?.src.match(/thumb\\/(\\d+)/)?.[1] === "${row}");
    const b = c?.querySelector(${JSON.stringify(selector)});
    b?.click();
    return !!b;
  })()`);
const note = (d) => d.evaluate(`document.querySelector(".map .note")?.innerText.trim() ?? null`);
const popup = (d) => d.evaluate(`document.querySelector(".popup.ai b")?.innerText.trim() ?? null`);
/** The rows picked in the grid (or strip), from their thumbnails. */
const picked = (d) =>
  d.evaluate(`[...document.querySelectorAll(".cell.picked img")].map((i) => Number(i.src.match(/thumb\\/(\\d+)/)?.[1]))`);

async function aiTab(d) {
  await d.press(".tabs button", /^AI$/);
  await d.settle();
  await d.press("button", /^Locate$/);
  await d.settle();
}
async function waitFor(d, what, fn, timeout = 20000) {
  const end = Date.now() + timeout;
  let v = await fn();
  while (Date.now() < end && !v) {
    await d.sleep(300);
    v = await fn();
  }
  return v;
}

export default {
  name: "check-ai-locate-why",
  title: "AI Locate: why, scenes, the map",
  async run(d) {
    const shot = async (name) => out && d.shot(`${out}/${name}.png`);
    await d.reset();
    const rows = await d.invoke("suggestions", { status: "Proposed", task: "locate" });
    console.log(`proposed locate suggestions: ${rows.length}`);
    // A card per photo, or per scene (its first listed photo), as the AI tab shows them.
    const cards = rows.filter((r, i) => r.group == null || rows.findIndex((m) => m.group === r.group) === i);
    const sceneOf = (r) => (r.group == null ? [r] : rows.filter((m) => m.group === r.group));
    await aiTab(d);

    // 1. Placed nowhere: why.
    const nowhere = rows.find((r) => !r.suggestion.place && r.suggestion.unplaced?.reason);
    if (nowhere) {
      const c = await waitFor(d, "card", () => card(d, nowhere.row));
      await shot("1-unplaced");
      check("placed nowhere: the card says why", !!c?.unplaced?.includes(nowhere.suggestion.unplaced.reason) && !!c?.unplaced?.includes(nowhere.suggestion.unplaced.clues[0] ?? ""), c);
    } else skip("placed nowhere: the card says why", "no proposed suggestion with a reason (made by prompt v3)");

    // 2. Placed in part: what keeps the rest open.
    const part = rows.find((r) => r.suggestion.place?.reason);
    if (part) {
      const c = await waitFor(d, "card", () => card(d, part.row));
      await shot("2-reason");
      check("placed in part: the reason under the levels", c?.reason === part.suggestion.place.reason, c);
    } else skip("placed in part: the reason under the levels", "no proposed suggestion with a place and a reason");

    // 3. A scene bigger than the card shows: opened, all of it.
    const big = cards.find((r) => sceneOf(r).length > 9);
    if (big) {
      const n = sceneOf(big).length;
      const before = await waitFor(d, "card", () => card(d, big.row));
      await clickIn(d, big.row, ".more");
      await d.sleep(300);
      const after = await card(d, big.row);
      await shot("3-scene");
      await clickIn(d, big.row, ".more");
      check("a big scene opens to show all its photos", before?.strip === 8 && after?.strip === n - 1 && after?.more === "Fewer", { n, before, after });
    } else skip("a big scene opens to show all its photos", "no proposed scene of more than 9 photos");

    // 4. Show on map, with a position: the map on its pin, its card open.
    const pinned = cards.find((r) => r.suggestion.place?.point && !(r.current?.placed && !r.current?.approx));
    if (pinned) {
      await waitFor(d, "card", () => card(d, pinned.row));
      await clickIn(d, pinned.row, ".onmap");
      const p = pinned.suggestion.place.point;
      const near = (v) => !!v && Math.abs(v.center[1] - p.lat) < 0.05 && Math.abs(v.center[0] - p.lon) < 0.05 && v.zoom > 12;
      const viewNow = () => d.evaluate(`JSON.parse(localStorage.getItem("map-view") ?? "null")`);
      const view = await waitFor(d, "view", async () => { const v = await viewNow(); return near(v) && v; }, 30000);
      const label = await waitFor(d, "popup", () => popup(d), 10000);
      // The card stands by its pin: the pin's dot is just below it on screen.
      const placed = await d.evaluate(`(() => {
        const box = document.querySelector(".popup.ai")?.getBoundingClientRect();
        const dot = [...document.querySelectorAll(".canvas .pin.ai .dot")].map((e) => e.getBoundingClientRect())
          .find((r) => box && Math.abs(r.x + r.width / 2 - (box.x + box.width / 2)) < 40 && r.y > box.y && r.y - box.bottom < 120);
        return !!dot;
      })()`);
      await shot("4-pin");
      check("Show on map: the guess's pin, its card open by it", !!label && !!view && placed, { label, view: view || (await viewNow()), point: p, placed });
      await d.press(".popup.ai button", /^Close$/).catch(() => {});
    } else skip("Show on map: the guess's pin, its card open", "no proposed guess with a position");

    // 5. Show on map, with no position: its photos picked, how to place them.
    // Names but no position, if there is one: the map goes to the place it names.
    const unpinned = cards.find((r) => r.suggestion.place && !r.suggestion.place.point) ?? cards.find((r) => !r.suggestion.place?.point);
    if (unpinned) {
      await aiTab(d);
      await waitFor(d, "card", () => card(d, unpinned.row));
      const g = unpinned.suggestion.place;
      const named = g && { country: g.country?.name ?? null, state: g.state?.name ?? null, city: g.city?.name ?? null, location: g.location?.name ?? null, country_code: g.country_code ?? null };
      const there = named ? await d.invoke("place_point", { place: named }) : null;
      await clickIn(d, unpinned.row, ".onmap");
      const said = await waitFor(d, "note", async () => ((await note(d)) ?? "").startsWith("No AI position") && note(d), 30000);
      const atPlace = (v) => !!v && !!there && Math.abs(v.center[1] - there.lat) < 0.05 && Math.abs(v.center[0] - there.lon) < 0.05;
      const view = there
        ? await waitFor(d, "view", async () => { const v = await d.evaluate(`JSON.parse(localStorage.getItem("map-view") ?? "null")`); return atPlace(v) && v; }, 20000)
        : null;
      await shot("5-no-pin");
      check(
        "Show on map with no position: at the place named, says how to place them",
        !!said && (!there || !!view),
        { said, there, view, members: sceneOf(unpinned).map((r) => r.row), shownPicked: await picked(d) },
      );

      // 6. Away and back: the map does not go to the guess again.
      await d.press(".tabs button", /^Photos$/);
      await d.settle();
      await d.press(".tabs button", /^Map$/);
      await d.sleep(2500);
      const again = await note(d);
      check("back on the map: no stale guess", !again?.startsWith("No AI position") && !(await popup(d)), { note: again });
    } else skip("Show on map with no position", "every proposed guess has a position");

    // 7. Accepted as approximate: the AI's colour, or not shown.
    if (pinned) {
      await aiTab(d);
      await waitFor(d, "card", () => card(d, pinned.row));
      await clickIn(d, pinned.row, ".onmap");
      await waitFor(d, "popup", () => popup(d), 30000);
      const precise = await d.evaluate(`[...document.querySelectorAll(".popup.ai label")].find((l) => /precise/i.test(l.innerText))?.querySelector("input")?.checked ?? false`);
      if (precise) await d.evaluate(`[...document.querySelectorAll(".popup.ai label")].find((l) => /precise/i.test(l.innerText)).querySelector("input").click()`);
      await d.press(".popup.ai button", /^Accept/);
      const drawn = await waitFor(d, "violet pin", () => d.evaluate(`document.querySelectorAll(".canvas .pin.ai-placed").length`), 20000);
      const counted = await note(d);
      await shot("7-ai-placed");
      check("accepted as approximate: drawn in the AI's colour", drawn > 0 && /placed by AI/.test(counted ?? ""), { drawn, note: counted });

      await d.press("button.toggle.settings");
      await d.press("#tab-map");
      await d.choose('select[aria-label="Positions from AI Locate, accepted as approximate"]', "hide");
      await d.key("Escape");
      const gone = await waitFor(d, "hidden", async () => (await d.evaluate(`document.querySelectorAll(".canvas .pin.ai-placed").length`)) === 0, 10000);
      const hiddenNote = await note(d);
      await shot("7-hidden");
      check("not shown when Settings says so, the footer counts them", !!gone && /not shown \(Settings\)/.test(hiddenNote ?? ""), { note: hiddenNote });
      await d.press("button.toggle.settings");
      await d.press("#tab-map");
      await d.choose('select[aria-label="Positions from AI Locate, accepted as approximate"]', "colour");
      await d.key("Escape");
    } else skip("accepted as approximate: the AI's colour", "no proposed guess with a position to accept");

    await d.reset();
    const failed = results.filter((r) => !r.ok).length;
    console.log(`${results.length - failed}/${results.length} passed`);
  },
};
