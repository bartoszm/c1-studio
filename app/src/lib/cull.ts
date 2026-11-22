// Choosing the best of alike photos, one duplicate group or series at a time: what
// becomes of the photos not kept, which of a big group are side by side, and the keys.

import type { Color, CullStep, Mark, RankState, Removal, Rest } from "./api";
import { COMPARE_MAX } from "./compare";
import { COLOR_NAMES } from "./format";
import { typingIn } from "./keys";

/** What becomes of the photos of a group that are not kept. */
export type RestChoice = "trash" | "hide" | "rating" | "colour" | "nothing";
/** What the photos kept are marked with. */
export type KeeperChoice = "nothing" | "rating" | "colour";

export type CullSettings = {
  rest: RestChoice;
  /** The rating and the colour tag the rest gets, when that is the choice. */
  restRating: number;
  restColour: Color;
  keeper: KeeperChoice;
  keeperRating: number;
  keeperColour: Color;
};

export const REST_CHOICES: RestChoice[] = ["trash", "hide", "rating", "colour", "nothing"];
export const KEEPER_CHOICES: KeeperChoice[] = ["nothing", "rating", "colour"];

export const CULL_DEFAULTS: CullSettings = {
  rest: "trash",
  restRating: 1,
  restColour: "Red",
  keeper: "nothing",
  keeperRating: 5,
  keeperColour: "Green",
};

/** The settings as saved, with anything missing or unreadable at its default. */
export function cullSettings(saved: unknown): CullSettings {
  const s = (saved && typeof saved === "object" ? saved : {}) as Partial<Record<keyof CullSettings, unknown>>;
  const d = CULL_DEFAULTS;
  const stars = (v: unknown, fallback: number) => (typeof v === "number" && Number.isInteger(v) && v >= 1 && v <= 5 ? v : fallback);
  const colour = (v: unknown, fallback: Color) => (COLOR_NAMES.includes(v as Color) ? (v as Color) : fallback);
  return {
    rest: REST_CHOICES.includes(s.rest as RestChoice) ? (s.rest as RestChoice) : d.rest,
    restRating: stars(s.restRating, d.restRating),
    restColour: colour(s.restColour, d.restColour),
    keeper: KEEPER_CHOICES.includes(s.keeper as KeeperChoice) ? (s.keeper as KeeperChoice) : d.keeper,
    keeperRating: stars(s.keeperRating, d.keeperRating),
    keeperColour: colour(s.keeperColour, d.keeperColour),
  };
}

/** What is done to the rest. A source that deletes nothing has no trash: they are hidden. */
export function restAction(s: CullSettings, removes: Removal | undefined): Rest {
  switch (s.rest) {
    case "trash": return removes === "None" ? "Hide" : "Trash";
    case "hide": return "Hide";
    case "rating": return { Rating: s.restRating };
    case "colour": return { Color: s.restColour };
    case "nothing": return "Nothing";
  }
}

export function keeperMark(s: CullSettings): Mark {
  if (s.keeper === "rating") return { Rating: s.keeperRating };
  if (s.keeper === "colour") return { Color: s.keeperColour };
  return "Nothing";
}

/** What a choice for the rest is called, in the menu and in the settings. */
export function restLabel(choice: RestChoice, s: CullSettings, removes?: Removal): string {
  switch (choice) {
    case "trash": return removes === "None" ? "Hide (no trash here)" : "To the trash";
    case "hide": return "Hide";
    case "rating": return `Rate ${"★".repeat(s.restRating)}`;
    case "colour": return `Tag ${s.restColour.toLowerCase()}`;
    case "nothing": return "Leave as they are";
  }
}

/** The photos kept: the active one and the ones marked, in the group's order. */
export function keepers(rows: number[], active: number | undefined, marked: Iterable<number>): number[] {
  const keep = new Set(marked);
  if (active !== undefined) keep.add(active);
  return rows.filter((r) => keep.has(r));
}

export const restOf = (rows: number[], keep: number[]): number[] => rows.filter((r) => !keep.includes(r));

/** The photos side by side, at most `max` of the group's `rows`: the ones `shown` before
 *  while the active one is among them (one may have been taken out, or two swapped), else
 *  `max` in a row, moved no further than the active one needs. */
export function shownRows(rows: number[], active: number | undefined, shown: number[] = [], max = COMPARE_MAX): number[] {
  const at = active === undefined ? -1 : rows.indexOf(active);
  const still = shown.filter((r) => rows.includes(r));
  if (still.length >= Math.min(2, rows.length) && still.length <= max && (at < 0 || still.includes(rows[at]))) {
    // A small group is whole again once a photo comes back to it.
    if (rows.length > max || still.length === rows.length) return still;
  }
  if (rows.length <= max) return rows.slice();
  const first = still.length ? rows.indexOf(still[0]) : 0;
  let start = Math.min(Math.max(first, 0), rows.length - max);
  if (at >= 0 && at < start) start = at;
  if (at >= start + max) start = at - max + 1;
  return rows.slice(start, start + max);
}

/** The group `delta` after the one at `at` of `total`: the first and the last stay put. */
export const stepGroup = (at: number, total: number, delta: number): number =>
  Math.min(Math.max(at + delta, 0), Math.max(total - 1, 0));

/** Where choosing the best is: the groups gone through, the one shown, the ones picked from. */
export type CullPlace = { groups: string[]; at: number; done: string[] };

/** The group to begin at: the first with no pick yet, so that going through them again
 *  takes up where it was left; the first of all when every one has a pick. */
export function firstUnpicked(list: { picked: boolean }[]): number {
  return Math.max(0, list.findIndex((g) => !g.picked));
}

/** After a pick from `group`: it is done, and the next one is shown, unless another group
 *  was gone to while the pick was made, which stays. */
export function afterPick(c: CullPlace, group: string): Pick<CullPlace, "at" | "done"> {
  const done = c.done.includes(group) ? c.done : [...c.done, group];
  return { at: c.groups[c.at] === group ? stepGroup(c.at, c.groups.length, 1) : c.at, done };
}

/** After a pick from `group` was taken back, or made again (`picked`): the group is shown,
 *  where it is still among the ones gone through, and marked as it is now. */
export function afterTurn(c: CullPlace, group: string, picked: boolean): Pick<CullPlace, "at" | "done"> {
  const i = c.groups.indexOf(group);
  const rest = c.done.filter((g) => g !== group);
  return { at: i < 0 ? c.at : i, done: picked && i >= 0 ? [...rest, group] : rest };
}

/** The place among the groups listed again (`ids`: they were worked out again, and may be
 *  named otherwise) of the group shown, known by the id a photo of it has now; without one,
 *  the place it had, as far as the list goes. */
export const placeAfter = (ids: string[], id: string | undefined, at: number): number => {
  const i = id === undefined ? -1 : ids.indexOf(id);
  return i >= 0 ? i : stepGroup(at, ids.length, 0);
};

/** The photo `delta` after the active one, round the group. */
/** Whether a `ranking` event (`now`, after `before`) says the group `shown` was the one
 *  just looked at: only then is it read again, not for every group of a long run. */
export const lookedAt = (before: Pick<RankState, "done"> | null, now: Pick<RankState, "done" | "group">, shown: string | undefined): boolean =>
  shown !== undefined && now.group === shown && now.done !== before?.done;

/** What the bar says of the `unseen` photos, of a group of `photos`, the local model has
 *  no view of: nothing while it has not looked at the group at all (`looked`: it was just
 *  asked to), since that is every group before its first look. */
export function unseenNote(unseen: number, photos: number, looked: boolean): string {
  if (unseen <= 0 || (unseen >= photos && !looked)) return "";
  return `${unseen.toLocaleString()} photo${unseen === 1 ? "" : "s"} not looked at`;
}

export function stepPhoto(rows: number[], active: number | undefined, delta: number): number | undefined {
  if (!rows.length) return undefined;
  const at = active === undefined ? -1 : rows.indexOf(active);
  if (at < 0) return rows[0];
  return rows[(at + delta + rows.length) % rows.length];
}

export type CullAction =
  | { kind: "keep" }
  | { kind: "mark" }
  | { kind: "group"; delta: 1 | -1 }
  | { kind: "photo"; delta: 1 | -1 }
  | { kind: "best" }
  /** Have the local model look: at this group, or at all of them. */
  | { kind: "rank"; all: boolean };

/** What a key does while culling, or nothing (the comparison under it has its own keys):
 *  typed into a field, handled already, or a dialog or menu is open (`blocked`). A held
 *  Enter keeps once: the groups after it are not answered by the same press. */
export function cullKey(
  e: { key: string; target: EventTarget | null; defaultPrevented: boolean; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; shiftKey?: boolean; repeat?: boolean },
  blocked: boolean,
): CullAction | null {
  if (blocked || e.defaultPrevented || typingIn(e) || e.ctrlKey || e.metaKey || e.altKey) return null;
  if (e.key === "Enter") return e.repeat ? null : { kind: "keep" };
  if (e.key === "ArrowDown") return { kind: "group", delta: 1 };
  if (e.key === "ArrowUp") return { kind: "group", delta: -1 };
  if (e.key === "ArrowRight") return { kind: "photo", delta: 1 };
  if (e.key === "ArrowLeft") return { kind: "photo", delta: -1 };
  if (e.key === "Tab") return { kind: "photo", delta: e.shiftKey ? -1 : 1 };
  if (e.repeat) return null;
  const key = e.key.toLowerCase();
  if (key === "k") return { kind: "mark" };
  if (key === "g") return { kind: "best" };
  if (key === "r") return { kind: "rank", all: !!e.shiftKey };
  return null;
}

const photos = (n: number) => `${n.toLocaleString()} photo${n === 1 ? "" : "s"}`;

/** What a pick did, for the footer: `Kept 1 photo, 3 to the trash`. The trash and the hiding
 *  are told from what was done (`step`), not what was asked: a source with no trash has the
 *  rest hidden. Variants that went with their image without being in the group are said. */
export function describePick(kept: number, rest: number, action: Rest, step: Pick<CullStep, "binned" | "hidden" | "skipped">): string {
  const parts = [`Kept ${photos(kept)}`];
  if (step.binned.length || step.hidden.length) {
    if (step.binned.length) {
      const beyond = step.binned.length - (rest - step.skipped.length);
      const more = beyond > 0 ? ` (${beyond.toLocaleString()} of them ${beyond === 1 ? "a variant" : "variants"} not shown here)` : "";
      parts.push(`${step.binned.length.toLocaleString()} to the trash${more}`);
    }
    if (step.hidden.length) parts.push(`${step.hidden.length.toLocaleString()} hidden`);
  } else if (action !== "Nothing" && action !== "Trash" && action !== "Hide" && rest) {
    parts.push("Rating" in action ? `${rest.toLocaleString()} rated ${"★".repeat(action.Rating)}` : `${rest.toLocaleString()} tagged ${action.Color.toLowerCase()}`);
  }
  if (step.skipped.length) {
    const n = step.skipped.length;
    parts.push(`${n.toLocaleString()} left: ${n === 1 ? "a variant" : "variants"} of a photo kept`);
  }
  return parts.join(", ");
}
