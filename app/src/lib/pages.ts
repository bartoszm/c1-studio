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

/**
 * A result read again in place. Its first page comes with `first`; the pages holding
 * `ranges` (of a result of `total` photos) are read alongside it, not after it. Resolves to
 * the new result and its pages once all are in: the old map is never touched, so the grid
 * keeps showing it until then. A result that grew has its further pages read too; one that
 * shrank loses those past its end.
 */
export async function refreshPages<R extends { total: number; rows: number[] }>(
  first: Promise<R>,
  ranges: (total: number) => [number, number][],
  total: number,
  read: (page: number) => Promise<number[]>,
): Promise<{ result: R; pages: Map<number, number[]> }> {
  const early = pagesFor(ranges(total), total).filter((page) => page > 0);
  const reads = early.map(read);
  const result = await first;
  const rows = await Promise.all(reads);
  const pages = new Map<number, number[]>([[0, result.rows]]);
  early.forEach((page, k) => { if (page * PAGE < result.total) pages.set(page, rows[k]); });
  const late = pagesFor(ranges(result.total), result.total).filter((page) => !pages.has(page));
  const more = await Promise.all(late.map(read));
  late.forEach((page, k) => pages.set(page, more[k]));
  return { result, pages };
}
