// A smart album: a rule, not photos. Undone afterwards.
export default {
  name: "albums",
  title: "Albums and smart albums",
  async run(d) {
    await d.reset();
    await d.evaluate(`document.querySelector("aside").scrollTo({ top: 0 })`);
    let done = false;
    try {
      await d.record(async () => {
        await d.caption("A smart album is a rule: it shows whatever matches it, now and later");
        await d.click(".albums-section button", /^\+ Smart album$/);
        await d.waitFor(".dialog h2");
        await d.sleep(500);
        await d.click(".dialog label input");
        await d.type("Tokyo, the best", 80);
        await d.sleep(400);
        await d.click(".dialog input[aria-label=Text]");
        await d.type("tokyo", 110);
        await d.sleep(500);
        await d.click(".dialog button.add");
        await d.sleep(400);
        await d.choose(".dialog select[aria-label=Condition]", "at_least", 1);
        await d.choose(".dialog select[aria-label=Stars]", "4", 0);
        await d.sleep(1400);
        await d.click(".dialog button.primary");
        done = true;
        await d.waitFor(null, null, { fn: `!document.querySelector(".dialog")` });
        await d.sleep(900);
        await d.caption("It is counted by its rule, and filters like any album");
        await d.click("aside button.label", /^Tokyo, the best/);
        await d.sleep(500);
        await d.settle();
        await d.sleep(2200);
      });
    } finally {
      if (done) await d.key("z", { modifiers: 2 });
    }
  },
};
