// Editing several photos at once: what the details panel's "Edit N photos" block shows for
// each field, how long lists are cut, and the footer while an edit runs.
import type { Merged } from "./api";

/** Whether a reply for `rows` is about the photos picked now. */
/** Stepping in the viewer to `row`: a single pick (or none) follows it, so edits land on
 *  the photo shown; several picked stay picked. Says whether the pick moved. */
export function followStep(picked: { size: number; clear(): void; add(row: number): unknown }, row: number | undefined): boolean {
  if (picked.size > 1 || row === undefined) return false;
  picked.clear();
  picked.add(row);
  return true;
}

export const sameRows =(rows: number[], picked: { size: number; has: (row: number) => boolean }) =>
  rows.length === picked.size && rows.every((r) => picked.has(r));

/** What a field shows over the selection: the value all of them share, or a placeholder
 *  saying how they differ (as the Location fields do) and a tooltip with the values and
 *  their counts. `m` is null while the selection is being read. */
export function mergedField<T>(m: Merged<T> | null | undefined, picked: number, text: (v: T) => string = String) {
  const all = picked.toLocaleString();
  if (!m) return { shown: "", placeholder: "…", title: "Reading the selection…" };
  if (m.same !== null) return { shown: text(m.same), placeholder: "", title: `The same on all ${all}` };
  const set = `Type to set it on all ${all}`;
  if (!m.values) return { shown: "", placeholder: `empty on ${all}`, title: set };
  const lines = m.top.map(([v, n]) => `${text(v)}: ${n.toLocaleString()}`);
  if (m.values > m.top.length) lines.push(`… ${(m.values - m.top.length).toLocaleString()} more`);
  if (m.empty) lines.push(`(empty): ${m.empty.toLocaleString()}`);
  lines.push(set);
  const placeholder =
    m.values === 1 && m.top.length
      ? `${text(m.top[0][0])} (${m.top[0][1].toLocaleString()}/${all})`
      : `${m.values.toLocaleString()} values${m.empty ? ` (${m.empty.toLocaleString()} empty)` : ""}`;
  return { shown: "", placeholder, title: lines.join("\n") };
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
