// The result's photos, a page at a time. After an edit the pages on screen are read again
// into a new map, which replaces the old one in one go: until then the old one is shown.

export const PAGE = 500;

export const rowAt = (pages: ReadonlyMap<number, number[]>, i: number): number | undefined =>
  pages.get(Math.floor(i / PAGE))?.[i % PAGE];

/** The pages holding positions [from, to) of each range, among the first `total`. */
export function pagesFor(ranges: [number, number][], total: number): number[] {
  const out = new Set<number>();
  for (const [from, to] of ranges) {
    const end = Math.min(to, total);
    if (from >= end) continue;
    for (let page = Math.floor(Math.max(0, from) / PAGE); page * PAGE < end; page++) out.add(page);
  }
  return [...out].sort((a, b) => a - b);
}

/** A new map with `first` and the pages `wanted` read again; the old map is left as it is. */
export async function reloadPages(
  first: [number, number[]],
  wanted: number[],
  read: (page: number) => Promise<number[]>,
): Promise<Map<number, number[]>> {
  const rest = wanted.filter((page) => page !== first[0]);
  const rows = await Promise.all(rest.map(read));
  return new Map([first, ...rest.map((page, k): [number, number[]] => [page, rows[k]])]);
}
