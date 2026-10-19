// The README's screenshots (docs/media/*.jpg), taken from the recording instance.
//
// usage: node tools\record\drive.mjs tools\record\stills.mjs [--port 9223]
//
// Runs like a scenario (without a frames dir, nothing is recorded). The last shot opens the
// photo folders source made by scenarios\folders.mjs (record that one first), then goes
// back to the catalog.
import path from "node:path";
import fs from "node:fs";
import { showFolder } from "./scenarios/folders.mjs";

const media = path.join(path.dirname(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1")), "..", "..", "docs", "media");

/** A JPEG of the window, or of `clip` ({ x, y, width, height }), the pointer hidden. */
async function jpeg(d, name, clip) {
  await d.evaluate(`document.querySelectorAll("#rec-pointer, .rec-ring").forEach((e) => (e.style.visibility = "hidden"))`);
  await d.sleep(200);
  const s = await d.send("Page.captureScreenshot", { format: "jpeg", quality: 88, ...(clip ? { clip: { ...clip, scale: 1 } } : {}) });
  fs.writeFileSync(path.join(media, name), Buffer.from(s.data, "base64"));
  console.log("wrote", name);
  await d.evaluate(`document.querySelectorAll("#rec-pointer, .rec-ring").forEach((e) => (e.style.visibility = ""))`);
}

const search = async (d, text) => {
  await d.click("input.search");
  await d.type(text, 0);
  await d.key("Enter");
  await d.sleep(500);
  await d.evaluate(`document.querySelector("input.search").blur()`);
  await d.settle();
  await d.sleep(800);
};

const cells = (d) =>
  d.evaluate(`[...document.querySelectorAll(".cell")].slice(0, 6).map((c) => { const r = c.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })`);

export default {
  name: "stills",
  async run(d) {
    // Search: a query with its counts.
    await d.reset();
    await search(d, "hong kong street");
    await jpeg(d, "search.jpg");

    // Analysis of the same search.
    await d.press(".tabs button", /^Analysis$/);
    await d.sleep(2500);
    await d.settle();
    await jpeg(d, "analysis.jpg");

    // An album, then a photo of it in the details panel and the viewer.
    await d.reset();
    await d.press("aside button.label", /^portfolio\n/);
    await d.settle();
    await d.sleep(800);
    await jpeg(d, "library.jpg");
    const [first] = await cells(d);
    await d.clickAt(first.x, first.y);
    await d.sleep(1500);
    await d.settle();
    await jpeg(d, "details.jpg");
    await d.key("Enter");
    await d.sleep(2500);
    await d.settle();
    await jpeg(d, "viewer.jpg");
    await d.key("Escape");

    // A photo's history: the photo the history scenario rates.
    await d.reset();
    await search(d, "tokyo street");
    const third = (await cells(d))[2];
    await d.clickAt(third.x, third.y);
    await d.sleep(800);
    await d.evaluate(`(() => { const h = document.querySelector("details.history"); h.open = true;
      // History is the panel's last section: room below it lets it scroll to the top.
      let p = h.parentElement; while (p && p.scrollHeight <= p.clientHeight) p = p.parentElement;
      if (p) p.style.paddingBottom = "600px";
      h.scrollIntoView({ block: "start" }); })()`);
    await d.sleep(1500);
    // The section and a little above it, the width of the panel, above the footer.
    const box = await d.evaluate(`(() => {
      const history = document.querySelector("details.history");
      const h = history.getBoundingClientRect();
      const r = (history.closest("aside, .info, .details") ?? history.parentElement).getBoundingClientRect();
      const bottom = Math.min(document.querySelector("footer").getBoundingClientRect().top, h.bottom + 8);
      const top = Math.max(0, Math.min(h.y - 40, bottom - 270));
      const end = Math.min(bottom, top + 270);
      return { x: r.x, y: top, width: r.width, height: end - top };
    })()`);
    await jpeg(d, "history.jpg", box);

    // Photo folders: the source scenarios\folders.mjs made.
    const view = await d.invoke("sources");
    const home = view.current;
    const folders = view.recent.find((r) => r.provider === "folders");
    if (!folders) {
      console.log("no photo folders source yet: record scenarios\\folders.mjs first");
      return;
    }
    try {
      await d.open({ provider: "folders", location: folders.location });
      await d.reset();
      await showFolder(d, "hk-exhibition-2026");
      await d.press("aside button.label", /^hk-exhibition-2026\n/);
      await d.settle();
      const [photo] = await cells(d);
      await d.clickAt(photo.x, photo.y);
      await d.sleep(1500);
      await d.settle();
      await jpeg(d, "folders.jpg");
    } finally {
      await d.open({ provider: home.provider, location: home.location });
    }
  },
};
