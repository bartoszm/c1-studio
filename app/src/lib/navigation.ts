// Back and forward through the filters, as in a browser: every filter left is kept with
// the photos that were picked in it, so going back finds them picked again.

import type { BBox, Filter } from "./api";
import type { Match } from "./criteria";
import type { SortField } from "./sorting";

/** A filter as it was shown, and the selection in it. */
export type Shown = {
  filter: Filter;
  sortField: SortField;
  sortReversed: boolean;
  match: Match;
  /** The trash shown, hidden photos shown: the same filter is other photos there. */
  bin: boolean;
  hidden: boolean;
  /** The map's tile the area is, and whether the grid followed the map (else the area
   *  was drawn): going back keeps the area, not what the map shows by then. */
  tile: BBox | null;
  follow: boolean;
  /** Rows picked, and the one focused. */
  picks: number[];
  focus?: number;
};

/** How many filters back are kept. */
export const KEPT = 50;
/** A step keeps this many picks at most; past that, only the focused one (select all on a
 *  big catalog, fifty times over, would be millions of rows). */
export const KEPT_PICKS = 10_000;

/** Same filter, order, match and mode: going there changes nothing on screen. */
export const sameView = (a: Shown, b: Shown) =>
  JSON.stringify([a.filter, a.sortField, a.sortReversed, a.match, a.bin, a.hidden]) ===
  JSON.stringify([b.filter, b.sortField, b.sortReversed, b.match, b.bin, b.hidden]);

const trimmed = (s: Shown): Shown => (s.picks.length > KEPT_PICKS ? { ...s, picks: [] } : s);

export class Trail {
  back: Shown[] = [];
  forward: Shown[] = [];
  /** The kind of the last step, for steps that fold into one (see `leave`). */
  private last?: string;

  /** `left` is what was shown before `now`: kept to go back to, and forward is forgotten.
   *  Steps of one `kind` in a row are one step (a search typed, tiles clicked on the map):
   *  going back skips them all, to what was shown before the first. */
  leave(left: Shown, now: Shown, kind?: string) {
    if (sameView(left, now)) return;
    const folds = kind !== undefined && kind === this.last && this.back.length > 0;
    this.last = kind;
    this.forward = [];
    if (folds) return;
    this.back.push(trimmed(left));
    if (this.back.length > KEPT) this.back.shift();
  }

  /** The filter to show instead of `now`, or null at the end. */
  goBack(now: Shown): Shown | null {
    this.last = undefined;
    const to = this.back.pop();
    if (to) this.forward.push(trimmed(now));
    return to ?? null;
  }

  goForward(now: Shown): Shown | null {
    this.last = undefined;
    const to = this.forward.pop();
    if (to) this.back.push(trimmed(now));
    return to ?? null;
  }
}
