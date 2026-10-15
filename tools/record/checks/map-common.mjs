// Helpers for the map checks: open the Map tab, read what it shows.

/** The Map tab, opened afresh; at `view` ({ center, zoom }) when given. */
export async function openMap(d, view) {
  // The map is closed first: closing it would store its own view over the one asked for.
  await d.reset();
  if (view) await d.evaluate(`localStorage.setItem("map-view", ${JSON.stringify(JSON.stringify(view))})`);
  await d.press(".tabs button", /^Map$/);
  await d.waitFor(".map .find input", null, { timeout: 30000 });
  await d.sleep(1500);
}

/** The map's centre and zoom, as it remembers them after each move. */
export const view = (d) => d.evaluate(`JSON.parse(localStorage.getItem("map-view") ?? "null")`);

/** The grid's total, the map's note, and the chips. */
export const state = (d) =>
  d.evaluate(`({
    total: document.querySelector("header .count")?.textContent.trim(),
    note: document.querySelector(".map .note")?.textContent.trim().split(" · ").slice(0, 2).join(" · "),
    chips: [...document.querySelectorAll(".filters .chip")].map((c) => c.textContent.replace("×", "").trim()),
    follow: document.querySelector("header .follow input")?.checked,
  })`);

/** Finds a place by name and picks its first hit; returns Nominatim's own answer for it. */
export async function find(d, place) {
  await d.press(".map .find input");
  await d.evaluate(`(() => { const i = document.querySelector(".map .find input"); i.focus(); i.select(); })()`);
  await d.type(place, 0);
  await d.key("Enter");
  await d.waitFor(".map .find li button", null, { timeout: 15000 });
  const before = await d.evaluate(`localStorage.getItem("map-view")`);
  await d.press(".map .find li button");
  await settled(d, before);
  const res = await fetch(`https://nominatim.openstreetmap.org/search?format=jsonv2&limit=1&q=${encodeURIComponent(place)}`, {
    headers: { "User-Agent": "c1-studio checks" },
  });
  return (await res.json())[0];
}

/** Waits for the map to stop after a move: it stores its view at the end of each one, so
 *  until that has changed from `before` and then stays the same a second. */
export async function settled(d, before, timeout = 20000) {
  const end = Date.now() + timeout;
  let last = before;
  let same = 0;
  while (Date.now() < end && same < 3) {
    await d.sleep(300);
    const now = await d.evaluate(`localStorage.getItem("map-view")`);
    same = now !== before && now === last ? same + 1 : 0;
    last = now;
  }
}
