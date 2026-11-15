// AI Locate: why a photo was placed nowhere, whole scenes, the map from a card, and
// positions accepted as approximate in their own colour. Checks, on the running app, with
// proposed locate suggestions in its AI store (none are made: no call costs anything):
//   1. a card placed nowhere says why: the model's reason and clues;
//   2. a card placed in part says what keeps the rest open;
//   3. a scene of more photos than the card shows opens to show them all;
//   4. *Show on map* on a card with a position opens the map on its pin, its card open,
//      and the note says the card's photos are picked; again with the map already there,
//      the card still opens by its pin; Esc right after the click stops it;
//   5. *Show on map* on a card with none picks its photos and says how to place them;
//   6. leaving the map and coming back does not go to the guess again;
//   7. a guess accepted as approximate is drawn in the AI's colour, and not at all when
//      Settings says so (the footer counts it);
//   8. *Show on map* on that card, now accepted, shows the area of its position, no pin card;
//   9. with only that photo searched for, the footer counts it as placed by AI, shown or
//      hidden, rather than saying no photo has a position.
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
      // The note under it says what is shown and that the card's photos are picked.
      const said = await note(d);
      check("Show on map: the note says the card's photos are picked", /the card's (photo is|\d+ photos are) picked/.test(said ?? ""), { said });
      await d.press(".popup.ai button", /^Close$/).catch(() => {});

      // 4b. Again, with the map already there: no flight, so no `moveend` need come; the
      // card opens all the same, and stays by its pin once the padding settles.
      await aiTab(d);
      await waitFor(d, "card", () => card(d, pinned.row));
      await clickIn(d, pinned.row, ".onmap");
      const again = await waitFor(d, "popup", () => popup(d), 15000);
      await d.sleep(1500);
      const stillBy = await d.evaluate(`(() => {
        const box = document.querySelector(".popup.ai")?.getBoundingClientRect();
        return !!box && [...document.querySelectorAll(".canvas .pin.ai .dot")].map((e) => e.getBoundingClientRect())
          .some((r) => Math.abs(r.x + r.width / 2 - (box.x + box.width / 2)) < 40 && r.y > box.y && r.y - box.bottom < 120);
      })()`);
      await shot("4b-already-there");
      check("Show on map with the map already there: the card opens by its pin", !!again && stillBy, { again, stillBy });
      await d.press(".popup.ai button", /^Close$/).catch(() => {});

      // 4c. Esc right after Show on map: the map does not go on to the guess.
      await aiTab(d);
      await waitFor(d, "card", () => card(d, pinned.row));
      await clickIn(d, pinned.row, ".onmap");
      await d.key("Escape");
      await d.sleep(4000);
      const stopped = await popup(d);
      await shot("4c-escape");
      check("Esc stops a Show on map on its way: no card opens", !stopped, { popup: stopped });
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

      // 8. Show on map on the card now accepted: no pin of its own (only guesses to review
      // have one), so the area of its position, no pin card, and a note saying so.
      await aiTab(d);
      await d.press("button", /^Accepted$/);
      await d.settle();
      await waitFor(d, "card", () => card(d, pinned.row));
      await clickIn(d, pinned.row, ".onmap");
      const p = pinned.suggestion.place.point;
      const said = await waitFor(d, "note", async () => { const n = await note(d); return /^Accepted: the area of the AI's position/.test(n ?? "") && n; }, 30000);
      const view = await waitFor(d, "view", async () => {
        const v = await d.evaluate(`JSON.parse(localStorage.getItem("map-view") ?? "null")`);
        return !!v && Math.abs(v.center[1] - p.lat) < 0.05 && Math.abs(v.center[0] - p.lon) < 0.05 && v.zoom > 10 && v;
      }, 20000);
      await d.sleep(1500);
      const card8 = await popup(d);
      await shot("8-accepted");
      check("Show on map on an accepted card: the area of its position, no pin card", !!said && !!view && !card8, { said, view, popup: card8 });
      await aiTab(d);
      await d.press("button", /^To review$/);

      // 9. Photos whose only position is the AI's: the footer counts them, shown or hidden,
      // instead of saying no photo here has a position.
      const photo = await d.invoke("photo", { row: pinned.row });
      await d.reset();
      await d.click("input.search");
      await d.type(`${photo.file.name}`, 0);
      await d.key("Enter");
      await d.key("Escape");
      await d.settle();
      await d.press(".tabs button", /^Map$/);
      const only = await waitFor(d, "note", async () => { const n = await note(d); return /placed by AI/.test(n ?? "") && n; }, 20000);
      await shot("9-ai-only");
      check("only AI positions: the footer counts them", /^No photos here have a position of their own · \d+ placed by AI/.test(only ?? ""), { note: only ?? (await note(d)), file: photo.file.name });
      await d.press("button.toggle.settings");
      await d.press("#tab-map");
      await d.choose('select[aria-label="Positions from AI Locate, accepted as approximate"]', "hide");
      await d.key("Escape");
      const hidden = await waitFor(d, "note", async () => { const n = await note(d); return /not shown \(Settings\)/.test(n ?? "") && n; }, 10000);
      check("only AI positions, hidden: the footer says they are there", !!hidden && !/have a position yet/.test(hidden), { note: hidden ?? (await note(d)) });
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
