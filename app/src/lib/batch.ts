// Editing several photos at once: what the details panel's "Edit N photos" block shows for
// each field, how long lists are cut, and the footer while an edit runs.
import type { Merged } from "./api";

/** Whether a reply for `rows` is about the photos picked now. */
export const sameRows = (rows: number[], picked: { size: number; has: (row: number) => boolean }) =>
  rows.length === picked.size && rows.every((r) => picked.has(r));

/** What a field shows over the selection: the value all of them share, or a placeholder
 *  saying how they differ. `m` is null while the selection is being read. */
export function mergedField<T>(m: Merged<T> | null | undefined, picked: number, text: (v: T) => string = String) {
  const all = picked.toLocaleString();
  if (!m) return { shown: "", placeholder: "…", title: "Reading the selection…" };
  if (m.same !== null) return { shown: text(m.same), placeholder: "", title: `The same on all ${all}` };
  const title = `Type to set it on all ${all}`;
  if (!m.values) return { shown: "", placeholder: `empty on ${all}`, title };
  const placeholder =
    m.values === 1
      ? `on ${(picked - m.empty).toLocaleString()} of ${all}`
      : `${m.values.toLocaleString()} values${m.empty ? `, ${m.empty.toLocaleString()} empty` : ""}`;
  return { shown: "", placeholder, title };
}

/** A long list cut to its first `n` rows, those on only some photos first; `more` is how
 *  many are left out. */
export function collapse<T>(rows: T[], partial: (row: T) => boolean, open: boolean, n = 8): { shown: T[]; more: number } {
  const sorted = [...rows.filter(partial), ...rows.filter((r) => !partial(r))];
  return open || sorted.length <= n ? { shown: sorted, more: 0 } : { shown: sorted.slice(0, n), more: sorted.length - n };
}

/** The footer while an edit runs: on `n` photos, or `null` when not known. */
export function editingNote(kind: "edit" | "undo" | "redo", n: number | null): string {
  const verb = { edit: "Editing", undo: "Undoing the edit on", redo: "Redoing the edit on" }[kind];
  if (n === null) return kind === "edit" ? "Editing…" : `${verb.split(" ")[0]}…`;
  return `${verb} ${n.toLocaleString()} photo${n === 1 ? "" : "s"}…`;
}

/** The panel's scroll that keeps a field where it was: it moved from `before` to `after`
 *  (its top on screen) while what is above it changed. */
export const scrollFix = (before: number | null, after: number | null, scrollTop: number) =>
  before === null || after === null ? scrollTop : scrollTop + (after - before);
