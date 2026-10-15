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

/** A list's number of groups, and what "Show more" says of the ones not shown yet. */
export function groupCount(total: number): string {
  return `${total.toLocaleString()} group${total === 1 ? "" : "s"}`;
}
export function moreLabel(total: number, shown: number): string {
  return `Show more (${(total - shown).toLocaleString()} left)`;
}
