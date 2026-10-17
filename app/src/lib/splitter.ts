// The height of the strip of photos under the map and the analysis, which the user drags.

export const STRIP_DEFAULT = 190;
export const STRIP_MIN = 80;
/** The map (or the charts) keeps at least this much. */
export const ROOM_MIN = 280;

/** `px` kept between the least a strip can be and what leaves the map its room. */
export function clampStrip(px: number, pane: number): number {
  if (!Number.isFinite(px)) return STRIP_DEFAULT;
  const most = Math.max(STRIP_MIN, pane - ROOM_MIN);
  return Math.round(Math.min(most, Math.max(STRIP_MIN, px)));
}

/** The strip's height after dragging the handle from `startY` to `y` (up makes it taller). */
export const dragged = (start: number, startY: number, y: number, pane: number) =>
  clampStrip(start + (startY - y), pane);

/** A stored height back, or the default for anything that is not one. */
export function storedStrip(text: string | null): number {
  const n = Number(text);
  return text !== null && Number.isFinite(n) && n >= STRIP_MIN ? Math.round(n) : STRIP_DEFAULT;
}
