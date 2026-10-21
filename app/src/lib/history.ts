// The change log as the History tab words it: a group of entries, and what a pick of it
// is about.

import type { HistoryPick, LogEvent, LogGroup, LogKind, NetWhat } from "./api";
import { eventText } from "./format";

const DAY = 86_400_000;

/** What each kind of entry is called in a sentence. */
export const KIND_LABELS: Record<LogKind, string> = {
  rating: "rating",
  color: "colour",
  keywords: "keywords",
  iptc: "caption",
  place: "place",
  geo: "GPS position",
  albums: "albums",
  added: "added",
  removed: "removed",
  deleted: "deleted",
  collection: "album change",
  remap: "keyword tree change",
  written: "written",
  undone: "undone",
};

/** The kinds the list's chips offer, in order. */
export const LIST_KINDS: LogKind[] = ["rating", "color", "keywords", "iptc", "place", "geo", "albums", "added", "removed", "collection", "remap", "written"];

const photos = (n: number) => `${n.toLocaleString()} photo${n === 1 ? "" : "s"}`;

/** One row of the log in a line: a single entry as `eventText` words it, a batch as
 *  `rating changed on 1,240 photos`. */
export function groupText(g: LogGroup, albumName?: (id: string) => string, provider = ""): string {
  if (g.entries === 1) return eventText(g.sample, albumName, provider);
  switch (g.kind) {
    case "added":
    case "removed":
      return `${photos(g.photos)} ${g.kind}`;
    case "deleted":
      return `${photos(g.photos)} deleted for good from the trash`;
    default:
      // Several changes to one photo (a keyword added, another removed) are not "on 0 photos".
      return g.photos <= 1
        ? `${KIND_LABELS[g.kind]}: ${g.entries.toLocaleString()} changes`
        : `${KIND_LABELS[g.kind]} changed on ${photos(g.photos)}`;
  }
}

/** The local day starting at `ms`, as an inclusive range of milliseconds: from its
 *  midnight to the last millisecond before the next one. With `last`, through that day. */
export function dayRange(ms: number, last = ms): { from: number; to: number } {
  const start = new Date(ms);
  const end = new Date(last);
  return {
    from: new Date(start.getFullYear(), start.getMonth(), start.getDate()).getTime(),
    to: new Date(end.getFullYear(), end.getMonth(), end.getDate() + 1).getTime() - 1,
  };
}

const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });

/** `12 Mar 2026`, or `12 Mar – 20 Mar 2026` for several days. */
export function spanText(from: number, to: number | null): string {
  if (to === null) return `since ${day(from)}`;
  // `to` is the last millisecond of a day: the day it falls on is the last one.
  return Math.floor(from / DAY) === Math.floor(to / DAY) || day(from) === day(to) ? day(from) : `${day(from)} – ${day(to)}`;
}

const path = (p: string) => p.split("|").join(" › ");

function whatText(what: NetWhat, albumName?: (id: string) => string): string {
  if (what === "Any") return "changed";
  if (what === "Added") return "added";
  if ("Field" in what) return `${KIND_LABELS[what.Field]} changed`;
  if ("KeywordAdded" in what) return `“${path(what.KeywordAdded)}” added`;
  if ("KeywordRemoved" in what) return `“${path(what.KeywordRemoved)}” removed`;
  const name = (id: string) => (albumName ? albumName(id) : id);
  if ("AlbumIn" in what) return `put into ${name(what.AlbumIn)}`;
  return `taken out of ${name(what.AlbumOut)}`;
}

/** What a pick is, for the chip above the grid: `rating changed, 12 Mar – 20 Mar 2026`. */
export function pickText(pick: HistoryPick, albumName?: (id: string) => string): string {
  if ("Changed" in pick) {
    const { from, to, what } = pick.Changed;
    return `${whatText(what, albumName)}, ${spanText(from, to)}`;
  }
  // The span of numbers holds entries of other kinds and origins, so it is no count.
  const { first, last, kind } = pick.Entries;
  const of = first === last ? "one log entry" : "a batch of log entries";
  return kind ? `photos of ${of} (${KIND_LABELS[kind]})` : `photos of ${of}`;
}

/** The entry's kind, for a chip that picks it. */
export function kindOf(e: LogEvent): LogKind {
  if ("change" in e) {
    const op = Object.keys(e.change.op)[0];
    if (op === "SetRating") return "rating";
    if (op === "SetColor") return "color";
    if (op === "AddKeywords" || op === "RemoveKeywords") return "keywords";
    if (op === "SetIptc") return "iptc";
    if (op === "SetPlace") return "place";
    if (op === "SetGeo") return "geo";
    return "albums";
  }
  if ("added" in e) return "added";
  if ("removed" in e) return "removed";
  if ("deleted" in e) return "deleted";
  if ("collection" in e) return "collection";
  if ("remap" in e) return "remap";
  if ("written" in e) return "written";
  return "undone";
}
