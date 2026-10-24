// Back and forward through the filters, as in a browser: every filter left is kept with
// the photos that were picked in it, so going back finds them picked again.

import type { Filter } from "./api";
import type { Match } from "./criteria";
import type { SortField } from "./sorting";

/** A filter as it was shown, and the selection in it. */
export type Shown = {
  filter: Filter;
  sortField: SortField;
  sortReversed: boolean;
  match: Match;
  /** Rows picked, and the one focused. */
  picks: number[];
  focus?: number;
};

/** How many filters back are kept. */
export const KEPT = 50;

/** Same filter, order and match: going there changes nothing on screen. */
export const sameView = (a: Shown, b: Shown) =>
  JSON.stringify([a.filter, a.sortField, a.sortReversed, a.match]) ===
  JSON.stringify([b.filter, b.sortField, b.sortReversed, b.match]);

export class Trail {
  back: Shown[] = [];
  forward: Shown[] = [];

  /** `left` is what was shown before `now`: kept to go back to, and forward is forgotten. */
  leave(left: Shown, now: Shown) {
    if (sameView(left, now)) return;
    this.back.push(left);
    if (this.back.length > KEPT) this.back.shift();
    this.forward = [];
  }

  /** The filter to show instead of `now`, or null at the end. */
  goBack(now: Shown): Shown | null {
    const to = this.back.pop();
    if (to) this.forward.push(now);
    return to ?? null;
  }

  goForward(now: Shown): Shown | null {
    const to = this.forward.pop();
    if (to) this.back.push(now);
    return to ?? null;
  }
}
