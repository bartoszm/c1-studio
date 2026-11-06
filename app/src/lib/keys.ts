/** What `typingIn` looks at: an element's tag, input type and editability. */
type Target = { tagName?: string; type?: string; isContentEditable?: boolean };

/** Keys typed into a text field belong to the field, not to the shortcuts. */
export function typingIn(e: { target: EventTarget | null }): boolean {
  const t = e.target as Target | null;
  if (!t?.tagName) return false;
  if (t.tagName === "INPUT") return t.type !== "checkbox";
  return t.tagName === "TEXTAREA" || t.tagName === "SELECT" || !!t.isContentEditable;
}

/** Enter or Space on a grid cell (`.cell` in Grid.svelte). A clicked cell keeps DOM focus while
 *  the arrows move the focused photo, so if the browser activated it, the clicked photo would be
 *  picked again: `onkey` in App.svelte prevents that. */
export function activatesCell(e: { key: string; target: EventTarget | null }): boolean {
  if (e.key !== "Enter" && e.key !== " ") return false;
  return !!(e.target as Element | null)?.closest?.(".cell");
}

/** What a key does in the viewer: close it, step by one, show the next (`g`) or the previous
 *  (Shift+G) grid, turn the grid (`o`), or nothing: typed into a field, handled already, or a dialog or menu is
 *  open over it (`blocked`). */
export function viewerKey(
  e: { key: string; target: EventTarget | null; defaultPrevented: boolean; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; shiftKey?: boolean },
  blocked: boolean,
): "close" | 1 | -1 | "grid" | "grid-back" | "turn" | null {
  if (blocked || e.defaultPrevented || typingIn(e)) return null;
  if (e.key === "Escape") return "close";
  if (e.key === "ArrowRight") return 1;
  if (e.key === "ArrowLeft") return -1;
  if (e.ctrlKey || e.metaKey || e.altKey) return null;
  // The letter whatever Caps Lock says; Shift alone steps back.
  const k = e.key.toLowerCase();
  if (k === "g") return e.shiftKey ? "grid-back" : "grid";
  if (k === "o") return "turn";
  return null;
}
