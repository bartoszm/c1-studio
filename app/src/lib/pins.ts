// The map's pins.

/** What dragging a stack of photos at one spot moves: those of it picked in the grid,
 *  two or more of them (one picked is most likely just looked at); else all of it. */
export function movingOf(moves: number[], picked: Iterable<number>): number[] {
  const chosen = new Set(picked);
  const some = moves.filter((r) => chosen.has(r));
  return some.length >= 2 && some.length < moves.length ? some : moves;
}
