// Picking photos: what inverting the pick (Ctrl+I) leaves picked.

/** The rows of `all` that are not in `now`, in their order; null when that is none
 *  (every photo is picked), so the pick stays as it is: an empty pick would let the next
 *  key edit the focused photo instead. */
export function inverted(all: number[], now: Set<number>): number[] | null {
  const rest = all.filter((r) => !now.has(r));
  return rest.length ? rest : null;
}
