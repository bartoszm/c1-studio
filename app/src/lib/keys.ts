/** What `typingIn` looks at: an element's tag, input type and editability. */
type Target = { tagName?: string; type?: string; isContentEditable?: boolean };

/** Keys typed into a text field belong to the field, not to the shortcuts. */
export function typingIn(e: { target: EventTarget | null }): boolean {
  const t = e.target as Target | null;
  if (!t?.tagName) return false;
  if (t.tagName === "INPUT") return t.type !== "checkbox";
  return t.tagName === "TEXTAREA" || t.tagName === "SELECT" || !!t.isContentEditable;
}

/** What a key does in the viewer: close it, step by one, or nothing: typed into a field,
 *  handled already, or a dialog or menu is open over it (`blocked`). */
export function viewerKey(
  e: { key: string; target: EventTarget | null; defaultPrevented: boolean },
  blocked: boolean,
): "close" | 1 | -1 | null {
  if (blocked || e.defaultPrevented || typingIn(e)) return null;
  if (e.key === "Escape") return "close";
  if (e.key === "ArrowRight") return 1;
  if (e.key === "ArrowLeft") return -1;
  return null;
}
