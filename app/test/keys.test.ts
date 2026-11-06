import { test } from "node:test";
import assert from "node:assert/strict";
import { activatesCell, typingIn, viewerKey } from "../src/lib/keys";

const on = (target: object | null) => ({ target: target as EventTarget | null });

test("keys typed into a text field belong to the field", () => {
  assert.ok(typingIn(on({ tagName: "INPUT", type: "text" })));
  assert.ok(typingIn(on({ tagName: "INPUT", type: "date" })));
  assert.ok(typingIn(on({ tagName: "TEXTAREA" })));
  assert.ok(typingIn(on({ tagName: "SELECT" })));
  assert.ok(typingIn(on({ tagName: "DIV", isContentEditable: true })));
});

test("keys elsewhere go to the shortcuts", () => {
  assert.ok(!typingIn(on({ tagName: "INPUT", type: "checkbox" })));
  assert.ok(!typingIn(on({ tagName: "BUTTON" })));
  assert.ok(!typingIn(on({ tagName: "DIV", isContentEditable: false })));
  assert.ok(!typingIn(on({})));
  assert.ok(!typingIn(on(null)));
});

test("other inputs are typed into too; only checkboxes are not", () => {
  assert.ok(typingIn(on({ tagName: "INPUT", type: "range" })));
  assert.ok(typingIn(on({ tagName: "INPUT", type: "radio" })));
  assert.ok(typingIn(on({ tagName: "INPUT" })));
});

const key = (k: string, target: object | null = { tagName: "BODY" }, defaultPrevented = false) => ({
  key: k,
  target: target as EventTarget | null,
  defaultPrevented,
});

test("the viewer steps with the arrows and closes on Escape", () => {
  assert.equal(viewerKey(key("ArrowRight"), false), 1);
  assert.equal(viewerKey(key("ArrowLeft"), false), -1);
  assert.equal(viewerKey(key("Escape"), false), "close");
  assert.equal(viewerKey(key("ArrowUp"), false), null);
  assert.equal(viewerKey(key("3"), false), null);
  // A button in the panel (a star, a keyword link) is not a field.
  assert.equal(viewerKey(key("ArrowRight", { tagName: "BUTTON" }), false), 1);
});

test("the viewer leaves keys typed into a field of the panel to the field", () => {
  const field = { tagName: "INPUT", type: "text" };
  assert.equal(viewerKey(key("ArrowLeft", field), false), null);
  assert.equal(viewerKey(key("ArrowRight", field), false), null);
  assert.equal(viewerKey(key("Escape", field), false), null);
  assert.equal(viewerKey(key("Escape", { tagName: "TEXTAREA" }), false), null);
});

test("the viewer leaves keys to a dialog or menu over it, or to whoever handled them", () => {
  assert.equal(viewerKey(key("Escape"), true), null);
  assert.equal(viewerKey(key("ArrowRight"), true), null);
  assert.equal(viewerKey(key("Escape", undefined, true), false), null);
});

test("the viewer steps the grid with g, back with Shift+G, not with a modifier", () => {
  assert.equal(viewerKey(key("g"), false), "grid");
  assert.equal(viewerKey({ ...key("G"), shiftKey: true }, false), "grid-back");
  assert.equal(viewerKey({ ...key("g"), ctrlKey: true }, false), null);
  assert.equal(viewerKey(key("g", { tagName: "INPUT", type: "text" }), false), null);
  assert.equal(viewerKey(key("g"), true), null);
  assert.equal(viewerKey(key("o"), false), "turn");
  assert.equal(viewerKey(key("O"), false), "turn");
  // Caps Lock gives "G" without Shift: still forward.
  assert.equal(viewerKey(key("G"), false), "grid");
  assert.equal(viewerKey({ ...key("G"), shiftKey: true }, false), "grid-back");
  assert.equal(viewerKey({ ...key("g"), altKey: true }, false), null);
  assert.equal(viewerKey({ ...key("o"), metaKey: true }, false), null);
  assert.equal(viewerKey({ ...key("O"), shiftKey: true }, false), "turn");
  assert.equal(viewerKey({ ...key("G"), ctrlKey: true, shiftKey: true }, false), null);
});

test("a held g or o acts once; a held arrow keeps stepping", () => {
  assert.equal(viewerKey({ ...key("g"), repeat: true }, false), null);
  assert.equal(viewerKey({ ...key("G"), shiftKey: true, repeat: true }, false), null);
  assert.equal(viewerKey({ ...key("o"), repeat: true }, false), null);
  assert.equal(viewerKey({ ...key("ArrowRight"), repeat: true }, false), 1);
});

test("Enter and Space never activate a grid cell, whatever photo has the focus", () => {
  const cell = { closest: (s: string) => (s === ".cell" ? {} : null) } as unknown as EventTarget;
  const button = { closest: () => null } as unknown as EventTarget;
  assert.ok(activatesCell({ key: "Enter", target: cell }));
  assert.ok(activatesCell({ key: " ", target: cell }));
  assert.ok(!activatesCell({ key: "ArrowRight", target: cell }));
  assert.ok(!activatesCell({ key: "Enter", target: button }));
  assert.ok(!activatesCell({ key: "Enter", target: null }));
});
