// Photos taken around one photo: its shoot (photos one after another with no long gap),
// a span either side of it, or its day. The details panel offers them under Captured.

import type { TimeRange } from "./api";

export type Around = "session" | "day" | { minutes: number };

const MINUTE = 60_000;

// Local days, `YYYY-MM-DD` as date inputs give them; the date tree's and the Dates chip's.
const pad = (n: number) => String(n).padStart(2, "0");
export const dayOf = (ms: number) => {
  const d = new Date(ms);
  return `${String(d.getFullYear()).padStart(4, "0")}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
};
/** Midnight starting the day, `after` days on (setFullYear: `new Date` reads years below 100
 *  as 19xx). By the calendar, not 24 hours: a day a clock change is in is 23 or 25 hours. */
export function dayStart(day: string, after = 0) {
  const [y, m, d] = day.split("-").map(Number);
  const at = new Date(0);
  at.setFullYear(y, m - 1, d + after);
  at.setHours(0, 0, 0, 0);
  return at.getTime();
}

/** What the panel offers, in order: a label, what it asks, and its tooltip. */
export const AROUND: { label: string; what: Around; title: string }[] = [
  { label: "shoot", what: "session", title: "The photos taken one after another around this one, with no gap longer than set in Settings" },
  { label: "±15 min", what: { minutes: 15 }, title: "Taken up to 15 minutes before or after" },
  { label: "±1 h", what: { minutes: 60 }, title: "Taken up to an hour before or after" },
  { label: "±3 h", what: { minutes: 180 }, title: "Taken up to three hours before or after" },
  { label: "day", what: "day", title: "Taken the same day" },
];

/** The span around a photo taken at `at` (Unix ms), for all but the shoot, which the library
 *  works out. A day is a local one, as the date tree's. */
export function aroundRange(at: number, what: Exclude<Around, "session">): TimeRange {
  if (what === "day") return { from: dayStart(dayOf(at)), to: dayStart(dayOf(at), 1) - 1 };
  return { from: at - what.minutes * MINUTE, to: at + what.minutes * MINUTE };
}

/** Both ranges at once: what lies in each. Open ends take the other's. With nothing in
 *  both, `from` is past `to` (see `isEmpty`). */
export function within(a: TimeRange | undefined, b: TimeRange): TimeRange {
  if (!a) return b;
  const pick = (x: number | undefined, y: number | undefined, f: (...n: number[]) => number) =>
    x === undefined ? y : y === undefined ? x : f(x, y);
  return { from: pick(a.from, b.from, Math.max), to: pick(a.to, b.to, Math.min) };
}

/** Two ranges with nothing in common gave this one. */
export const isEmpty = (r: TimeRange) => r.from !== undefined && r.to !== undefined && r.from > r.to;

/** A range from a local midnight to the end of a day, as the date picks make. */
export function wholeDays(r: TimeRange): boolean {
  const midnight = (ms: number) => new Date(ms).setHours(0, 0, 0, 0) === ms;
  return (r.from === undefined || midnight(r.from)) && (r.to === undefined || midnight(r.to + 1));
}

/** How long a gap ends a shoot, in minutes, when Settings say nothing, and the most they take. */
export const SESSION_GAP = 30;
export const SESSION_GAP_MAX = 1440;
/** A gap from Settings, or from what is typed there: whole minutes, 1 to a day; null when
 *  it is no number at all. */
export function sessionGap(v: unknown): number | null {
  const n = typeof v === "string" ? (v.trim() ? +v : NaN) : v;
  if (typeof n !== "number" || !Number.isFinite(n)) return null;
  return Math.min(SESSION_GAP_MAX, Math.max(1, Math.round(n)));
}
