// Comparing 2 to 4 picked photos: which ones, the keys, and the shared zoom.

import { typingIn } from "./keys";

/** More than four makes each photo too small to judge. */
export const COMPARE_MAX = 4;

/** How two photos are shown: next to each other, or one over the other. Three or four are
 *  always side by side. */
export type CompareMode = "side" | "split" | "overlay" | "difference" | "blink";

export const MODES: { mode: CompareMode; key: string; label: string; title: string }[] = [
  { mode: "side", key: "s", label: "Side by side", title: "The photos next to each other (s)" },
  { mode: "split", key: "w", label: "Split", title: "One photo over the other, cut by a line to drag (w)" },
  { mode: "overlay", key: "o", label: "Overlay", title: "The second photo over the first, as see-through as the slider says (o)" },
  { mode: "difference", key: "d", label: "Difference", title: "Only what differs between the two is bright; the same is black (d)" },
  { mode: "blink", key: "b", label: "Blink", title: "One photo at a time: Space switches, to catch small changes (b)" },
];

/** The photos to compare, focused first, then in the order they were picked, at most
 *  `COMPARE_MAX`; `left` counts the picked ones that do not fit. */
export function compareRows(picked: Iterable<number>, focus?: number): { rows: number[]; left: number } {
  const all = [...picked];
  const first = focus !== undefined && all.includes(focus) ? [focus] : [];
  const rows = [...first, ...all.filter((r) => r !== focus)].slice(0, COMPARE_MAX);
  return { rows, left: all.length - rows.length };
}

/** Photo `drop` taken out of the comparison (never below two). The active photo stays
 *  active; when it is the one taken out, the photo after it is, or the one before when it
 *  was the last. */
export function dropRow(rows: number[], drop: number, active: number): { rows: number[]; active: number } | null {
  if (rows.length <= 2 || drop < 0 || drop >= rows.length) return null;
  const next = rows.filter((_, i) => i !== drop);
  if (drop !== active) return { rows: next, active: next.indexOf(rows[active]) };
  return { rows: next, active: Math.min(drop, next.length - 1) };
}

/** Photo `row` gone (deleted, or out of the album shown) after a wait: out of the comparison
 *  too. Found by its id, as the active photo may have changed meanwhile. Null when fewer
 *  than two are left: the comparison closes. */
export function removeRow(rows: number[], row: number, active: number): { rows: number[]; active: number } | null {
  const i = rows.indexOf(row);
  return i < 0 ? { rows, active } : dropRow(rows, i, active);
}

export type CompareAction =
  | { kind: "close" }
  | { kind: "swap" }
  | { kind: "active"; delta: 1 | -1 }
  | { kind: "drop" }
  | { kind: "zoom" }
  | { kind: "sync" }
  | { kind: "mode"; mode: CompareMode };

/** What a key does in the comparison, or nothing: typed into a field, handled already, or
 *  a dialog or menu is open over it (`blocked`). The modes beyond side by side need two photos. */
export function compareKey(
  e: { key: string; target: EventTarget | null; defaultPrevented: boolean; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; shiftKey?: boolean; repeat?: boolean },
  blocked: boolean,
  count: number,
): CompareAction | null {
  if (blocked || e.defaultPrevented || typingIn(e) || e.ctrlKey || e.metaKey || e.altKey) return null;
  if (e.key === "Escape") return { kind: "close" };
  if (e.key === "Tab") return { kind: "active", delta: e.shiftKey ? -1 : 1 };
  if (e.key === "ArrowRight") return { kind: "active", delta: 1 };
  if (e.key === "ArrowLeft") return { kind: "active", delta: -1 };
  const key = e.key.toLowerCase();
  // Held down, it would flip the photos at the key's repeat rate.
  if (e.key === " " && count === 2 && !e.repeat) return { kind: "swap" };
  if (key === "x" && count > 2) return { kind: "drop" };
  if (key === "z") return { kind: "zoom" };
  if (key === "l") return { kind: "sync" };
  const mode = MODES.find((m) => m.key === key);
  if (mode && (mode.mode === "side" || count === 2)) return { kind: "mode", mode: mode.mode };
  return null;
}

/** Zoom and pan of a photo in its pane: scale `s` (1 fits the pane) and the shift `x`, `y`
 *  in pixels of the pane, with the top left corner as the origin of the scale. */
export type View = { s: number; x: number; y: number };
export const FIT: View = { s: 1, x: 0, y: 0 };
export const ZOOM_MAX = 16;

/** Never smaller than the pane, never moved past its edges. */
export function clampView(v: View, w: number, h: number): View {
  const s = Math.min(ZOOM_MAX, Math.max(1, v.s));
  const clamp = (t: number, size: number) => Math.min(0, Math.max(size * (1 - s), t));
  return { s, x: clamp(v.x, w), y: clamp(v.y, h) };
}

/** Zoomed by `factor` about the point (`px`, `py`) of the pane, which stays where it is. */
export function zoomAt(v: View, factor: number, px: number, py: number, w: number, h: number): View {
  const s = Math.min(ZOOM_MAX, Math.max(1, v.s * factor));
  const k = s / v.s;
  return clampView({ s, x: px - (px - v.x) * k, y: py - (py - v.y) * k }, w, h);
}

export function panBy(v: View, dx: number, dy: number, w: number, h: number): View {
  return clampView({ s: v.s, x: v.x + dx, y: v.y + dy }, w, h);
}

/** The scale at which one pixel of an image (`nw` × `nh`) is one pixel of the screen, when
 *  it fits a pane of `w` × `h`: what `z` and a double-click zoom to. Never below 2, so a
 *  small preview still zooms in. */
export function oneToOne(nw: number, nh: number, w: number, h: number, dpr = 1): number {
  if (!nw || !nh || !w || !h) return 2;
  const fit = Math.min(w / nw, h / nh);
  return Math.min(ZOOM_MAX, Math.max(2, 1 / (fit * dpr)));
}
