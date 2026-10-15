// Helpers for the map checks: open the Map tab, read what it shows.

export async function openMap(d) {
  await d.reset();
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
  await d.press(".map .find li button");
  await d.sleep(3500);
  const res = await fetch(`https://nominatim.openstreetmap.org/search?format=jsonv2&limit=1&q=${encodeURIComponent(place)}`, {
    headers: { "User-Agent": "c1-studio checks" },
  });
  return (await res.json())[0];
}
