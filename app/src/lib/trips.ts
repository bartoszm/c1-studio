// How trips read: their name, days and dates.

import type { Trip } from "./api";

/** The user's name, else its places. With neither: a suggestion has no place at all; a
 *  trip found by positions alone (no place names) is unnamed. */
export function tripName(t: Pick<Trip, "name" | "label" | "kind">): string {
  return t.name ?? (t.label || (t.kind === "suggested" ? "No place" : "Unnamed trip"));
}

/** Calendar days from its first photo to its last, both counted. */
export function tripDays(t: Pick<Trip, "start" | "end">): number {
  const day = (ms: number) => {
    const d = new Date(ms);
    return Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 86_400_000;
  };
  return day(t.end) - day(t.start) + 1;
}

/** `22 Oct – 8 Nov`, `3–7 May`, `12 Apr`; with the year when asked. */
export function tripDates(t: Pick<Trip, "start" | "end">, year = false): string {
  const a = new Date(t.start);
  const b = new Date(t.end);
  const y = year ? { year: "numeric" as const } : {};
  const dm = (d: Date, withYear: boolean) =>
    d.toLocaleDateString(undefined, { day: "numeric", month: "short", ...(withYear ? y : {}) });
  if (a.toDateString() === b.toDateString()) return dm(a, true);
  if (a.getFullYear() !== b.getFullYear()) return `${dm(a, year)} – ${dm(b, year)}`;
  if (a.getMonth() === b.getMonth()) {
    const month = b.toLocaleDateString(undefined, { month: "short", ...y });
    return `${a.getDate()}–${b.getDate()} ${month}`;
  }
  return `${dm(a, false)} – ${dm(b, true)}`;
}
