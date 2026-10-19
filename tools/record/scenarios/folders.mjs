// Photo folders: plain folders opened like a catalog, their dialog, a rescan, and the kinds
// of source the menu offers. Opens (or makes, the first time) a folders source on
// $C1_RECORD_FOLDERS, off camera, and goes back to the catalog at the end. The first run
// reads every file there, which takes a while on a cloud drive.
const ROOT = process.env.C1_RECORD_FOLDERS ?? "G:\\My Drive\\foto";
const NAME = "Photos on Drive";

/** Opens the sidebar's folder tree down to `name` (off camera): the drive, then each
 * folder on the way. */
export const showFolder = async (d, name) => {
  for (let i = 0; i < 8; i++) {
    const found = await d.evaluate(`(() => {
      const nodes = [...document.querySelectorAll("aside .node")];
      if (nodes.some((n) => n.querySelector(".name")?.textContent.trim() === ${JSON.stringify(name)})) return true;
      const shut = nodes.filter((n) => n.querySelector(".chev:not(.open)") && n.querySelector(".chev").tagName === "BUTTON");
      const folder = shut.find((n) => /^[A-Z]:$|My Drive|foto/.test(n.querySelector(".name")?.textContent.trim() ?? ""));
      folder?.querySelector(".chev").click();
      return false;
    })()`);
    if (found) return;
    await d.sleep(400);
  }
  throw new Error(`no folder ${name} in the sidebar`);
};

const openMenu = async (d) => {
  await d.click("header .source-button");
  await d.waitFor(".source-menu");
  await d.sleep(700);
};

export default {
  name: "folders",
  title: "Photo folders",
  async run(d) {
    const view = await d.invoke("sources");
    const home = view.current;
    if (!home || home.provider === "folders") throw new Error("start the recording instance on the catalog copy");
    let spec = view.recent.find((r) => r.provider === "folders" && r.name === NAME);
    if (!spec) {
      const made = await d.invoke("folders_new", { name: NAME, roots: [ROOT] });
      const location = await d.invoke("folders_save", { def: made.definition, location: null, migrate: null });
      spec = { provider: "folders", location };
    }
    try {
      await d.open({ provider: "folders", location: spec.location });
      await d.reset();
      await showFolder(d, "hk-exhibition-2026");
      await d.record(async () => {
        await d.caption("Plain folders of JPEG, PNG, TIFF and raw files open like a catalog");
        await d.sleep(2600);
        await d.caption("Their folders are in the sidebar; EXIF, XMP and IPTC are read from the files");
        await d.click("aside button.label", /^hk-exhibition-2026\n/);
        await d.sleep(500);
        await d.settle();
        const cell = await d.locate(".cell");
        await d.clickAt(cell.x, cell.y);
        await d.sleep(2600);
        await d.caption("Edit folders… : the folders, what to leave out, where thumbnails go, XMP export");
        await openMenu(d);
        await d.click(".source-menu button", /^Edit folders…$/);
        await d.waitFor(".dialog h2", /Photo folders/);
        await d.sleep(2600);
        await d.evaluate(`document.querySelector(".dialog").scrollTo({ top: 10000, behavior: "smooth" })`);
        await d.sleep(2400);
        await d.click(".dialog .buttons button", /^Cancel$/);
        await d.sleep(500);
        await d.caption("Rescan finds new, moved and renamed files; a drive not there marks its photos offline");
        await openMenu(d);
        await d.click(".source-menu button", /^Rescan$/);
        await d.sleep(2800);
        await d.caption("Kinds of source: offer catalogs, folders or both (the installer asks too)");
        await openMenu(d);
        await d.click(".source-menu button", /^Kinds of source…$/);
        await d.sleep(3200);
        await d.key("Escape");
        await d.sleep(400);
      });
    } finally {
      await d.caption("");
      await d.open({ provider: home.provider, location: home.location });
    }
  },
};
