// Words for the Analysis tab.

/** A group's badge: its size, and how many of it the filter picks when not all. */
export function groupBadge(g: { size: number; matched: number }): string {
  const size = g.size.toLocaleString();
  return g.matched < g.size ? `${g.matched.toLocaleString()} of ${size}` : size;
}

/** What a chart with no bars says: whether the filter picks nothing, or nothing it picks
 *  is measured yet. */
export function emptyNote(total: number, measured: number): string {
  if (total === 0) return "Nothing matches the filter.";
  if (measured === 0) return `Not measured yet (${total.toLocaleString()} photo${total === 1 ? "" : "s"}).`;
  return "Nothing measured here yet.";
}
