import { test } from "node:test";
import assert from "node:assert/strict";
import { typingIn } from "../src/lib/keys";

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
