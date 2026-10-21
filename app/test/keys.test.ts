import { test } from "node:test";
import assert from "node:assert/strict";
import { typingIn, viewerKey } from "../src/lib/keys";

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
