/** What `typingIn` looks at: an element's tag, input type and editability. */
type Target = { tagName?: string; type?: string; isContentEditable?: boolean };

/** Keys typed into a text field belong to the field, not to the shortcuts. */
export function typingIn(e: { target: EventTarget | null }): boolean {
  const t = e.target as Target | null;
  if (!t?.tagName) return false;
  if (t.tagName === "INPUT") return t.type !== "checkbox";
  return t.tagName === "TEXTAREA" || t.tagName === "SELECT" || !!t.isContentEditable;
}
