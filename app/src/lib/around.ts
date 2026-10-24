// Photos taken around one photo: its shoot (photos one after another with no long gap),
// a span either side of it, or its day. The details panel offers them under Captured.

import type { TimeRange } from "./api";

export type Around = "session" | "day" | { minutes: number };

const MINUTE = 60_000;

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
  if (what === "day") {
    const from = new Date(at);
    from.setHours(0, 0, 0, 0);
    const to = new Date(from);
    to.setDate(to.getDate() + 1);
    return { from: from.getTime(), to: to.getTime() - 1 };
  }
  return { from: at - what.minutes * MINUTE, to: at + what.minutes * MINUTE };
}

/** Both ranges at once: what lies in each. Open ends take the other's. */
export function within(a: TimeRange | undefined, b: TimeRange): TimeRange {
  if (!a) return b;
  const pick = (x: number | undefined, y: number | undefined, f: (...n: number[]) => number) =>
    x === undefined ? y : y === undefined ? x : f(x, y);
  return { from: pick(a.from, b.from, Math.max), to: pick(a.to, b.to, Math.min) };
}

/** A range from a local midnight to the end of a day, as the date picks make. */
export function wholeDays(r: TimeRange): boolean {
  const midnight = (ms: number) => new Date(ms).setHours(0, 0, 0, 0) === ms;
  return (r.from === undefined || midnight(r.from)) && (r.to === undefined || midnight(r.to + 1));
}

/** How long a gap ends a shoot, in minutes, when Settings say nothing. */
export const SESSION_GAP = 30;
