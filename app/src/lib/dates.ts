// The date tree: years › months › weeks, from the index's facets. A week is the days of
// one ISO week inside one month (`2025-01-W05`), so a week across two months shows
// under each with its own days, and counts add up.

import type { Node } from "./tree";

const DAY = 86_400_000;

/** ISO 8601 week number of a local date: weeks start on Monday, week 1 holds 4 January. */
export function isoWeek(date: Date): number {
  const d = new Date(date.getFullYear(), date.getMonth(), date.getDate());
  d.setDate(d.getDate() + 3 - ((d.getDay() + 6) % 7)); // the Thursday of its week
  const jan4 = new Date(d.getFullYear(), 0, 4);
  return 1 + Math.round(((d.getTime() - jan4.getTime()) / DAY - 3 + ((jan4.getDay() + 6) % 7)) / 7);
}

/** The first and last day of the month that fall in a week key's ISO week. */
export function weekDays(key: string): [number, number] | null {
  const m = key.match(/^(\d{4})-(\d{2})-W(\d{2})$/);
  if (!m) return null;
  const [year, month, week] = [+m[1], +m[2] - 1, +m[3]];
  const last = new Date(year, month + 1, 0).getDate();
  let first = 0;
  let end = 0;
  for (let day = 1; day <= last; day++) {
    if (isoWeek(new Date(year, month, day)) !== week) continue;
    first ||= day;
    end = day;
  }
  return first ? [first, end] : null;
}

const monthName = (year: number, month: number, style: "long" | "short") =>
  new Date(year, month, 1).toLocaleDateString(undefined, { month: style });

/** `W5 · 27–31 Jan`. */
export function weekName(key: string): string {
  const days = weekDays(key);
  const week = +key.slice(9);
  if (!days) return `W${week}`;
  const mon = monthName(+key.slice(0, 4), +key.slice(5, 7) - 1, "short");
  return `W${week} · ${days[0] === days[1] ? days[0] : `${days[0]}–${days[1]}`} ${mon}`;
}

/** `Week 5, 27–31 Jan 2025`, for the filter summary. */
export function weekText(key: string): string {
  return `${weekName(key).replace(/^W(\d+) ·/, "Week $1,")} ${key.slice(0, 4)}`;
}

/** The tree, newest year first, months and weeks in calendar order. Keys are `d:` and
 *  the facet value: `d:2025`, `d:2025-01`, `d:2025-01-W05`. */
export function dateTree(years: [string, number][], months: [string, number][], weeks: [string, number][]): Node[] {
  const node = (value: string, name: string, count: number, children: Node[] = []): Node => ({
    key: "d:" + value, value, name, icon: "date", count, children,
  });
  const weeksOf = new Map<string, [string, number][]>();
  for (const w of weeks) {
    const month = w[0].slice(0, 7);
    weeksOf.set(month, [...(weeksOf.get(month) ?? []), w]);
  }
  const monthsOf = new Map<string, [string, number][]>();
  for (const m of months) {
    const year = m[0].slice(0, 4);
    monthsOf.set(year, [...(monthsOf.get(year) ?? []), m]);
  }
  const firstDay = (key: string) => weekDays(key)?.[0] ?? 0;
  const weekNodes = (month: string) =>
    (weeksOf.get(month) ?? [])
      .sort((a, b) => firstDay(a[0]) - firstDay(b[0]))
      .map(([week, n]) => node(week, weekName(week), n));
  const monthNodes = (year: string) =>
    (monthsOf.get(year) ?? [])
      .sort((a, b) => a[0].localeCompare(b[0]))
      .map(([month, n]) => node(month, monthName(+year, +month.slice(5) - 1, "long"), n, weekNodes(month)));
  return [...years].sort((a, b) => +b[0] - +a[0]).map(([year, n]) => node(year, year, n, monthNodes(year)));
}
