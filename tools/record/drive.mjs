// Drives a C1 Studio instance over CDP (see start.ps1) and records it.
//
// usage: node drive.mjs <scenario.mjs> [frames dir] [--port 9223]
//
// A scenario's default export is { name, title, run(d) }. run() sets the scene and wraps
// what is shown in d.record(). With a frames dir, d.record() writes screencast frames
// (frame-NNNN.jpg) and times.json (each frame's time, in seconds) for assemble.py and
// to_mp4.py; without one it just runs, which is handy while writing a scenario.
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const args = process.argv.slice(2);
const portAt = args.indexOf("--port");
const port = portAt >= 0 ? Number(args.splice(portAt, 2)[1]) : 9223;
const [scenarioFile, framesDir] = args;
if (!scenarioFile) {
  console.error("usage: node drive.mjs <scenario.mjs> [frames dir] [--port 9223]");
  process.exit(2);
}

/** Every clip has this size, whatever the window's. */
export const WIDTH = 1440;
export const HEIGHT = 900;

const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const ws = new WebSocket(list.find((t) => t.type === "page").webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0;
const wait = new Map();
const handlers = new Map();
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id) wait.get(msg.id)?.(msg);
  else handlers.get(msg.method)?.(msg.params);
};
const send = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const i = ++id;
    wait.set(i, (msg) => {
      wait.delete(i);
      if (msg.error) reject(new Error(method + ": " + msg.error.message));
      else resolve(msg.result);
    });
    ws.send(JSON.stringify({ id: i, method, params }));
  });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const evaluate = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
  return r.result.value;
};

/** JS source of a function finding the elements matching `selector` whose trimmed text
 * includes `text` (a string) or matches it (a RegExp), visible ones only. */
const finder = (selector, text) => `(() => {
  const re = ${text instanceof RegExp ? text.toString() : "null"};
  const s = ${JSON.stringify(typeof text === "string" ? text : null)};
  return [...document.querySelectorAll(${JSON.stringify(selector)})].filter((e) => {
    const t = (e.innerText ?? e.textContent).trim();
    if (re && !re.test(t)) return false;
    if (s !== null && !t.includes(s)) return false;
    const r = e.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  });
})()`;

/** Centre of the first match (scrolled into view), or null. */
const locate = (selector, text) =>
  evaluate(`(() => {
    const e = ${finder(selector, text)}[0]; if (!e) return null;
    e.scrollIntoView({ block: "nearest", inline: "nearest" });
    const r = e.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2, w: r.width, h: r.height };
  })()`);

const mouse = (x, y, type, extra = {}) =>
  send("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: type === "mouseMoved" ? 0 : 1, ...extra });

const KEYS = {
  Enter: 13, Escape: 27, Tab: 9, Backspace: 8, Delete: 46, " ": 32,
  ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40,
};
const PUNCT = { "-": ["Minus", 189], "+": ["Equal", 187], "*": ["Digit8", 56] };

const d = {
  send,
  sleep,
  evaluate,
  locate,
  WIDTH,
  HEIGHT,

  /** Waits until `selector` (with `text`) is on screen, or `fn` (JS source) is truthy. */
  async waitFor(selector, text, { timeout = 15000, fn } = {}) {
    const expr = fn ?? `${finder(selector, text)}.length > 0`;
    const until = Date.now() + timeout;
    while (Date.now() < until) {
      if (await evaluate(expr)) return;
      await sleep(100);
    }
    throw new Error(`timed out waiting for ${fn ?? selector + " " + (text ?? "")}`);
  },
  /** Waits for the grid's images to be decoded, so a clip never shows empty tiles. */
  async settle(timeout = 8000) {
    await d.waitFor(null, null, {
      timeout,
      fn: `[...document.images].filter((i) => i.getBoundingClientRect().height > 0).every((i) => i.complete)`,
    }).catch(() => {});
    await sleep(150);
  },

  /** Glides the pointer to (x, y). The pointer is drawn by caption.js's cursor. */
  async move(x, y, ms = 350) {
    const from = d._at ?? { x: WIDTH / 2, y: HEIGHT / 2 };
    const steps = Math.max(2, Math.round(ms / 16));
    for (let i = 1; i <= steps; i++) {
      const t = i / steps;
      const e = t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2;
      await mouse(from.x + (x - from.x) * e, from.y + (y - from.y) * e, "mouseMoved", { button: "none" });
      await sleep(16);
    }
    d._at = { x, y };
  },
  async clickAt(x, y, opts = {}) {
    await d.move(x, y);
    await sleep(80);
    await mouse(x, y, "mousePressed", opts);
    await sleep(60);
    await mouse(x, y, "mouseReleased", opts);
  },
  async click(selector, text, opts) {
    const p = await locate(selector, text);
    if (!p) throw new Error(`nothing matches ${selector} ${text ?? ""}`);
    await d.clickAt(p.x, p.y, opts);
  },
  async rightClick(selector, text) {
    const p = await locate(selector, text);
    if (!p) throw new Error(`nothing matches ${selector} ${text ?? ""}`);
    await d.move(p.x, p.y);
    await sleep(80);
    await mouse(p.x, p.y, "mousePressed", { button: "right" });
    await mouse(p.x, p.y, "mouseReleased", { button: "right" });
  },
  /** Sets a <select> (its native list isn't recorded, so this skips it). */
  async choose(selector, value, nth = 0) {
    await evaluate(`(() => { const s = document.querySelectorAll(${JSON.stringify(selector)})[${nth}];
      s.value = ${JSON.stringify(value)}; s.dispatchEvent(new Event("change", { bubbles: true })); })()`);
    await sleep(300);
  },
  /** Drags from (x0, y0) to (x1, y1) with the button held. */
  async drag(x0, y0, x1, y1, { ms = 700, modifiers = 0 } = {}) {
    await d.move(x0, y0);
    await mouse(x0, y0, "mousePressed", { modifiers });
    const steps = Math.round(ms / 16);
    for (let i = 1; i <= steps; i++) {
      await mouse(x0 + ((x1 - x0) * i) / steps, y0 + ((y1 - y0) * i) / steps, "mouseMoved", { buttons: 1, modifiers });
      await sleep(16);
    }
    await mouse(x1, y1, "mouseReleased", { modifiers });
    d._at = { x: x1, y: y1 };
  },
  /** Scrolls the wheel at (x, y); negative `dy` scrolls up (zooms the map in). */
  async wheel(x, y, dy, { steps = 6 } = {}) {
    await d.move(x, y);
    for (let i = 0; i < steps; i++) {
      await send("Input.dispatchMouseEvent", { type: "mouseWheel", x, y, deltaX: 0, deltaY: dy / steps });
      await sleep(40);
    }
  },
  async type(text, delay = 60) {
    for (const ch of text) {
      await send("Input.insertText", { text: ch });
      await sleep(delay);
    }
  },
  /** Presses a key; `modifiers` is CDP's bit mask (1 Alt, 2 Ctrl, 4 Meta, 8 Shift). */
  async key(key, { modifiers = 0, code } = {}) {
    const [punctCode, punctKeyCode] = PUNCT[key] ?? [];
    const digit = /^[0-9]$/.test(key);
    const keyCode = KEYS[key] ?? punctKeyCode ?? (key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0);
    code ??= punctCode ?? (digit ? `Digit${key}` : key.length === 1 ? `Key${key.toUpperCase()}` : key);
    const base = { key, code, windowsVirtualKeyCode: keyCode, modifiers };
    // Enter needs its text too, or a form isn't submitted.
    const text = modifiers & 2 ? undefined : key === "Enter" ? "\r" : key.length === 1 ? key : undefined;
    await send("Input.dispatchKeyEvent", { type: text ? "keyDown" : "rawKeyDown", text, ...base });
    await send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    await sleep(40);
  },

  /** Shows `text` in a band at the bottom of the window until the next caption. */
  caption: (text) => evaluate(`window.__rec?.caption(${JSON.stringify(text)})`),

  /** Clicks the first match through the DOM: no pointer, for setting a scene off camera. */
  async press(selector, text) {
    const done = await evaluate(`(() => { const e = ${finder(selector, text)}[0]; e?.click(); return !!e; })()`);
    if (!done) throw new Error(`nothing matches ${selector} ${text ?? ""}`);
    await sleep(250);
  },

  /** Back to the whole library: no filters, nothing selected, nothing open, Photos view,
   * the details panel on, and the timeline on only if `timeline`. */
  async reset({ timeline = false } = {}) {
    for (let i = 0; i < 3; i++) await d.key("Escape");
    await evaluate(`(() => {
      const i = document.querySelector("input.search");
      if (i) { i.value = ""; i.dispatchEvent(new Event("input", { bubbles: true })); i.blur(); }
      window.getSelection()?.removeAllRanges();
    })()`);
    await sleep(400);
    if (await locate("button", /^Clear filters$/)) await d.press("button", /^Clear filters$/);
    await d.press(".tabs button", /^Photos$/);
    const on = (text) => evaluate(`${finder("header button.toggle", text)}[0]?.classList.contains("on")`);
    if ((await on(/^Timeline$/)) !== timeline) await d.press("header button.toggle", /^Timeline$/);
    if (!(await on(/^Details$/))) await d.press("header button.toggle", /^Details$/);
    // Stacking and hidden photos off, newest first: what a clip starts from.
    if (await on(/^Stack$/)) await d.press("header button.toggle", /^Stack$/);
    if (await on(/^Hidden$/)) await d.press("header button.toggle", /^Hidden$/);
    const sort = 'header select[aria-label="Sort photos by"]';
    if ((await evaluate(`document.querySelector(${JSON.stringify(sort)})?.value`)) !== "date") await d.choose(sort, "date");
    if (await locate("header .sort .dir", /Oldest first/)) await d.press("header .sort .dir");
    await d.key("0", { modifiers: 2 });
    await d.caption("");
    await d.settle();
  },

  async shot(file) {
    const s = await send("Page.captureScreenshot", { format: "png" });
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, Buffer.from(s.data, "base64"));
    console.log("wrote", file);
  },

  /** Runs `fn`, leaving its time out of the clip (see record). */
  cut: (fn) => fn(),

  /** Records screencast frames while `fn` runs (only with a frames dir; see the top). */
  async record(fn, { quality = 90 } = {}) {
    if (!framesDir) return fn();
    fs.rmSync(framesDir, { recursive: true, force: true });
    fs.mkdirSync(framesDir, { recursive: true });
    const times = [];
    const writes = [];
    // d.cut() leaves its time out of the clip: frames are dropped while it runs, and later
    // ones are moved back by how long it took.
    let cutting = false;
    let removed = 0;
    d.cut = async (fn, keep = 0.6) => {
      const start = Date.now() / 1000;
      cutting = true;
      try {
        await fn();
      } finally {
        cutting = false;
        removed += Math.max(0, Date.now() / 1000 - start - keep);
      }
      // A frame comes only when something changes: nudge the pointer for one.
      const at = d._at ?? { x: WIDTH / 2, y: HEIGHT / 2 };
      await d.move(at.x + 1, at.y, 32);
    };
    handlers.set("Page.screencastFrame", (p) => {
      send("Page.screencastFrameAck", { sessionId: p.sessionId });
      if (cutting) return;
      const n = times.length;
      times.push(p.metadata.timestamp - removed);
      writes.push(fs.promises.writeFile(path.join(framesDir, `frame-${String(n).padStart(4, "0")}.jpg`), Buffer.from(p.data, "base64")));
    });
    await send("Page.startScreencast", { format: "jpeg", quality, maxWidth: WIDTH, maxHeight: HEIGHT, everyNthFrame: 1 });
    try {
      await sleep(400);
      await fn();
      await sleep(800);
    } finally {
      await send("Page.stopScreencast");
      handlers.delete("Page.screencastFrame");
      await Promise.all(writes);
      fs.writeFileSync(path.join(framesDir, "times.json"), JSON.stringify(times));
      console.log(`recorded ${times.length} frames into ${framesDir}`);
    }
  },
};

// A fixed viewport, the recording overlay (captions and a visible pointer), and no
// transient scrollbars or text cursor blinking from earlier runs.
await send("Page.enable");
await send("Emulation.setDeviceMetricsOverride", { width: WIDTH, height: HEIGHT, deviceScaleFactor: 1, mobile: false });
await evaluate(fs.readFileSync(new URL("./overlay.js", import.meta.url), "utf8"));
await sleep(300);

const scenario = (await import(pathToFileURL(path.resolve(scenarioFile)).href)).default;
try {
  await scenario.run(d);
} finally {
  await d.caption("").catch(() => {});
  ws.close();
}
