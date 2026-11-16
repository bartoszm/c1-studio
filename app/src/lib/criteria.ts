// Multi-criteria filtering, as pure functions over the UI's Filter. A click on a value
// picks only it, Ctrl+click adds it to the section's picks, Alt+click leaves it out;
// ratings compare (≥, =, ≤). Sections narrow together; the values picked in one are
// alternatives, but for keywords and albums (a photo has several), which can ask for
// all of them or any.

import type { Filter, Presence, Range } from "./api";
import { under } from "./places";

export type RatingOp = "≥" | "=" | "≤";
export const RATING_OPS: RatingOp[] = ["≥", "=", "≤"];
/** `null`: not picked. */
export type Pick = "in" | "out" | null;

/** What a click does to a section's picks: this value only, this one too, or leave it out. */
export type PickMode = "only" | "add" | "out";

/** Ctrl (⌘) adds, Alt leaves out, a plain click picks only this. */
export function modeOf(e: { ctrlKey: boolean; metaKey: boolean; altKey: boolean }): PickMode {
  if (e.ctrlKey || e.metaKey) return "add";
  if (e.altKey) return "out";
  return "only";
}

export function pickOf(inc: readonly string[] | undefined, exc: readonly string[] | undefined, value: string): Pick {
  if (inc?.includes(value)) return "in";
  if (exc?.includes(value)) return "out";
  return null;
}

type Picks = { inc: string[] | undefined; exc: string[] | undefined };
const list = (l: readonly string[]) => (l.length ? [...l] : undefined);

/**
 * A value picked in a section. `only`: just this value, and nothing left out; when it
 * already is, the section lets go. `add`: this one on or off beside the others. `out`:
 * this one left out, or no longer. A value is never both in and out.
 */
export function pick(inc: readonly string[] | undefined, exc: readonly string[] | undefined, value: string, mode: PickMode): Picks {
  const i = (inc ?? []).filter((v) => v !== value);
  const o = (exc ?? []).filter((v) => v !== value);
  const was = pickOf(inc, exc, value);
  if (mode === "only") {
    const alone = was === "in" && !i.length && !o.length;
    return alone ? { inc: undefined, exc: undefined } : { inc: [value], exc: undefined };
  }
  if (mode === "add" && was !== "in") i.push(value);
  if (mode === "out" && was !== "out") o.push(value);
  return { inc: list(i), exc: list(o) };
}

/** A place tree's node: picked in or out itself, or in a place that is ("under-…"). */
export type PlacePick = Pick | "under-in" | "under-out";

/** Where `path` stands among the picked places: its own pick, else the one of the
 *  nearest picked place it is in. */
export function placePick(inc: readonly string[] | undefined, exc: readonly string[] | undefined, path: string): PlacePick {
  const own = pickOf(inc, exc, path);
  if (own) return own;
  const nearest = (l: readonly string[] | undefined) =>
    Math.max(-1, ...(l ?? []).filter((p) => under(path, p)).map((p) => p.length));
  const i = nearest(inc);
  const o = nearest(exc);
  if (i < 0 && o < 0) return null;
  return i > o ? "under-in" : "under-out";
}

/** Picks of one kind (`own`: places in, or out) without those another of them already
 *  says: Kraków when Poland is, unless a pick of the `other` kind lies between (Poland,
 *  except Małopolskie, but Kraków). As `normalise_places` in the index. */
export function tidyPlaces(own: readonly string[], other: readonly string[]): string[] {
  const between = (p: string, q: string) => other.some((o) => o !== q && o !== p && under(p, o) && under(o, q));
  return own.filter((p, i) => !own.some((q, j) => (p === q ? j < i : under(p, q) && !between(p, q))));
}

/**
 * A place picked, as `pick`. Places are alternatives, and the deepest pick over a place
 * decides: adding Poland lets go of Kraków; leaving out a place in an included one gives
 * Poland except Kraków, adding one in a left-out place not Poland, but Kraków.
 */
export function pickPlace(inc: readonly string[] | undefined, exc: readonly string[] | undefined, path: string, mode: PickMode): Picks {
  const { inc: i, exc: o } = pick(inc, exc, path, mode);
  if (mode === "only") return { inc: i, exc: o };
  return { inc: list(tidyPlaces(i ?? [], o ?? [])), exc: list(tidyPlaces(o ?? [], i ?? [])) };
}

/** The rating a click on `n` stars asks for under `op`. */
export function ratingFor(op: RatingOp, n: number): Presence<Range> {
  if (op === "≥") return { Values: { min: n } };
  if (op === "≤") return { Values: { max: n } };
  return { Values: { min: n, max: n } };
}

export function sameRating(a: Filter["rating"], b: Filter["rating"]): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** The operator and stars a rating filter reads as, when it is one a click can make. */
export function ratingParts(r: Filter["rating"]): { op: RatingOp; n: number } | null {
  if (!r || typeof r === "string") return null;
  const { min, max } = r.Values;
  if (min !== undefined && max === undefined) return { op: "≥", n: min };
  if (max !== undefined && min === undefined) return { op: "≤", n: max };
  if (min !== undefined && min === max) return { op: "=", n: min };
  return null;
}

export function ratingText(r: NonNullable<Filter["rating"]>): string {
  if (r === "None") return "unrated";
  if (r === "Any") return "rated";
  const { min, max } = r.Values;
  if (min !== undefined && min === max) return `★ = ${min}`;
  if (min !== undefined && max !== undefined) return `★ ${min}–${max}`;
  return min !== undefined ? `★ ≥ ${min}` : `★ ≤ ${max}`;
}

/** Light classes a photo can have several of (the rest, dark/medium/bright, are one each). */
export const LIGHT_PROBLEMS = ["clipped highlights", "blocked shadows", "low contrast"];

/** How a section's picks combine: a photo with any of them, or with every one. */
export type Join = "any" | "all";
/** The sections whose values a photo can have several of, so "all" means something. */
export type CombineKey = "keywords" | "albums" | "hues" | "problems";
/** Each section's join where it is not its default. */
export type Combine = Partial<Record<CombineKey, Join>>;
/** Keywords and albums ask for every pick, as Capture One's own filters do; the Analysis
 *  cards for any, as their other cards do. */
export const DEFAULT_JOIN: Record<CombineKey, Join> = { keywords: "all", albums: "all", hues: "any", problems: "any" };

export const joinOf = (c: Combine, key: CombineKey): Join => c[key] ?? DEFAULT_JOIN[key];

/** Which Analysis cards ask for every pick, as their switch shows it. */
export type AllOf = { hues?: boolean; problems?: boolean };
export const allOfCards = (c: Combine): AllOf => ({ hues: joinOf(c, "hues") === "all", problems: joinOf(c, "problems") === "all" });

/** The other join of a section, as the `Combine` keeps it (the default is left out). */
export function flipJoin(c: Combine, key: CombineKey): Combine {
  const to: Join = joinOf(c, key) === "all" ? "any" : "all";
  const next = { ...c };
  if (to === DEFAULT_JOIN[key]) delete next[key];
  else next[key] = to;
  return next;
}

/** The section's included values, those a join combines. */
export function joined(f: Filter, key: CombineKey): string[] {
  switch (key) {
    case "keywords": return f.keywords_under ?? [];
    case "albums": return f.collections ?? [];
    case "hues": return f.hues ?? [];
    case "problems": return (f.light ?? []).filter((l) => LIGHT_PROBLEMS.includes(l));
  }
}

/** Sections whose join can only matter with two picks or more: the rest go back to their
 *  default, so the next picks are not silently combined the other way. */
export function keptCombine(f: Filter, c: Combine): Combine {
  const kept = Object.fromEntries(Object.entries(c).filter(([k]) => joined(f, k as CombineKey).length > 1));
  return Object.keys(kept).length === Object.keys(c).length ? c : kept;
}

/**
 * What to ask the index, with each section's picks joined its way. The index takes
 * keywords and albums as "every one", and the Analysis cards' values as "any": the
 * others become `all_of` / `any_of` groups, one per value. "Red and blue" is a photo
 * that is red and is blue. One pick needs no change. Brightness picks (dark, medium,
 * bright) stay "any": a photo has only one.
 */
export function combinePicks(f: Filter, c: Combine): Filter {
  const out: Filter = { ...f };
  const groups: Filter[] = [];
  const many = (key: CombineKey) => joined(f, key).length > 1;
  if (joinOf(c, "hues") === "all" && many("hues")) {
    for (const hue of f.hues!) groups.push({ hues: [hue] });
    delete out.hues;
  }
  if (joinOf(c, "problems") === "all" && many("problems")) {
    for (const p of joined(f, "problems")) groups.push({ light: [p] });
    const rest = (f.light ?? []).filter((l) => !LIGHT_PROBLEMS.includes(l));
    if (rest.length) out.light = rest;
    else delete out.light;
  }
  if (joinOf(c, "keywords") === "any" && many("keywords")) {
    groups.push({ any_of: f.keywords_under!.map((k) => ({ keywords_under: [k] })) });
    delete out.keywords_under;
  }
  // Smart albums too: the index makes each into its rule, inside the alternatives.
  if (joinOf(c, "albums") === "any" && many("albums")) {
    groups.push({ any_of: f.collections!.map((id) => ({ collections: [id] })) });
    delete out.collections;
  }
  return groups.length ? { ...out, all_of: [...(out.all_of ?? []), ...groups] } : f;
}
