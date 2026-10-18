// Photos the user hid: left out of every view unless "Show hidden" is on.
import type { Filter } from "./api";

/** The filter as the views ask it: hidden photos let in while they are shown. */
export const withHidden = (f: Filter, show: boolean): Filter => (show ? { ...f, hidden: "Include" } : f);

/** What `h` does to these photos: shows them again when every one is hidden, else hides them. */
export const hides = (rows: number[], hidden: ReadonlySet<number>): boolean =>
  !rows.length || !rows.every((r) => hidden.has(r));

/** How the status line says it: "2 photos hidden" / "1 photo shown again". */
export function hiddenNote(n: number, hidden: boolean): string {
  return `${n.toLocaleString()} photo${n === 1 ? "" : "s"} ${hidden ? "hidden" : "shown again"}`;
}
