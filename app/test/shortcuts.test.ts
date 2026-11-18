import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { MENU_ITEMS, PLACES, SHORTCUTS, keysFor } from "../src/lib/shortcuts";

test("no key does two things in one place", () => {
  const seen = new Map<string, string>();
  for (const s of SHORTCUTS) {
    for (const key of s.keys.split(" · ")) {
      const at = `${s.where}: ${key}`;
      assert.ok(!seen.has(at), `${at} is both “${seen.get(at)}” and “${s.what}”`);
      seen.set(at, s.what);
    }
  }
});

test("every shortcut is in a place the list shows", () => {
  for (const s of SHORTCUTS) assert.ok(PLACES.includes(s.where), s.keys);
});

test("the guide's Keyboard section lists the same keys", () => {
  const guide = readFileSync(new URL("../../docs/GUIDE.md", import.meta.url), "utf8");
  const section = guide.slice(guide.indexOf("## Keyboard"), guide.indexOf("\n## ", guide.indexOf("## Keyboard") + 1));
  const keys = (text: string) => text.replaceAll("`", "").split(/[\s,;()]+/).filter((k) => k && !["or", "viewer", "compare"].includes(k));
  const listed = new Set(
    section
      .split("\n")
      .filter((l) => l.startsWith("| ") && !l.startsWith("| Key |"))
      .flatMap((l) => keys(l.split("|")[1])),
  );
  const ours = new Set(SHORTCUTS.flatMap((s) => keys(s.keys.replaceAll(" · ", " "))));
  for (const k of ours) assert.ok(listed.has(k), `${k} is not in the guide's Keyboard section`);
  for (const k of listed) assert.ok(ours.has(k), `the guide lists ${k}, the app's list does not`);
});

test("a menu finds its key by what it does", () => {
  assert.equal(keysFor("Add keywords"), "Ctrl+K");
  assert.equal(keysFor("Undo"), "Ctrl+Z");
  assert.equal(keysFor("nothing like this"), "");
});

test("every item of the photo menu finds its key", () => {
  for (const [item, what] of Object.entries(MENU_ITEMS)) assert.ok(keysFor(what), `${item}: “${what}” is in no shortcut`);
  assert.equal(keysFor(MENU_ITEMS.trash), "Delete");
  assert.equal(keysFor(MENU_ITEMS.trashInAlbum), "Ctrl+Delete");
  assert.equal(keysFor(MENU_ITEMS.copy), "Ctrl+C");
});
