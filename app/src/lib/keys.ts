/** What `typingIn` looks at: an element's tag, input type and editability. */
type Target = { tagName?: string; type?: string; isContentEditable?: boolean };

/** Keys typed into a text field belong to the field, not to the shortcuts. */
export function typingIn(e: { target: EventTarget | null }): boolean {
  const t = e.target as Target | null;
  if (!t?.tagName) return false;
  if (t.tagName === "INPUT") return t.type !== "checkbox";
  return t.tagName === "TEXTAREA" || t.tagName === "SELECT" || !!t.isContentEditable;
}

/** What a key does in the viewer: close it, step by one, show the next (`g`) or the previous
 *  (Shift+G) grid, turn the grid (`o`), or nothing: typed into a field, handled already, or a dialog or menu is
 *  open over it (`blocked`). */
export function viewerKey(
  e: { key: string; target: EventTarget | null; defaultPrevented: boolean; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean },
  blocked: boolean,
): "close" | 1 | -1 | "grid" | "grid-back" | "turn" | null {
  if (blocked || e.defaultPrevented || typingIn(e)) return null;
  if (e.key === "Escape") return "close";
  if (e.key === "ArrowRight") return 1;
  if (e.key === "ArrowLeft") return -1;
  if (e.ctrlKey || e.metaKey || e.altKey) return null;
  if (e.key === "g") return "grid";
  if (e.key === "G") return "grid-back";
  if (e.key === "o" || e.key === "O") return "turn";
  return null;
}
