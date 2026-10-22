// Multi-criteria filtering, as pure functions over the UI's Filter: a value is
// off → included → excluded → off; ratings compare (≥, =, ≤); and the picked criteria
// match all together (the default) or any one of them.

import type { Filter, Presence, Range } from "./api";

export type Match = "all" | "any";
export type RatingOp = "≥" | "=" | "≤";
export const RATING_OPS: RatingOp[] = ["≥", "=", "≤"];
/** `null`: not picked. */
export type Pick = "in" | "out" | null;

export function pickOf(inc: readonly string[] | undefined, exc: readonly string[] | undefined, value: string): Pick {
  if (inc?.includes(value)) return "in";
  if (exc?.includes(value)) return "out";
  return null;
}

/** Click on a value: off → included → excluded → off. Empty lists become `undefined`. */
export function cycle(
  inc: readonly string[] | undefined,
  exc: readonly string[] | undefined,
  value: string,
): { inc: string[] | undefined; exc: string[] | undefined } {
  const was = pickOf(inc, exc, value);
  const drop = (l: readonly string[] | undefined) => {
    const left = (l ?? []).filter((v) => v !== value);
    return left.length ? left : undefined;
  };
  const add = (l: readonly string[] | undefined) => [...(l ?? []), value];
  if (was === null) return { inc: add(inc), exc: drop(exc) };
  if (was === "in") return { inc: drop(inc), exc: add(exc) };
  return { inc: drop(inc), exc: drop(exc) };
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

/**
 * What to ask the index. All together is the filter as it is: every field narrows. Any
 * one of them turns the includes (rating, colour, cameras, keywords, albums and the
 * "none" picks) into alternatives; everything else, the excludes among it, still holds.
 */
export function compile(f: Filter, mode: Match): Filter {
  if (mode === "all") return f;
  const base: Filter = { ...f };
  const groups: Filter[] = [];
  if (base.rating !== undefined) {
    groups.push({ rating: base.rating });
    delete base.rating;
  }
  if (base.color !== undefined) {
    groups.push({ color: base.color });
    delete base.color;
  }
  if (base.cameras?.length) {
    groups.push({ cameras: base.cameras });
    delete base.cameras;
  }
  for (const path of base.keywords_under ?? []) groups.push({ keywords_under: [path] });
  delete base.keywords_under;
  for (const id of base.collections ?? []) groups.push({ collections: [id] });
  delete base.collections;
  if (base.unkeyworded) {
    groups.push({ unkeyworded: true });
    delete base.unkeyworded;
  }
  if (base.uncollected !== undefined) {
    groups.push({ uncollected: base.uncollected });
    delete base.uncollected;
  }
  if (base.placed !== undefined) {
    groups.push({ placed: base.placed });
    delete base.placed;
  }
  if (base.located !== undefined) {
    groups.push({ located: base.located });
    delete base.located;
  }
  // One alternative is no choice: leave the filter as it was.
  if (groups.length < 2) return f;
  return { ...base, all_of: [...(base.all_of ?? []), { any_of: groups }] };
}

/** How many picks are alternatives to each other, to know when Match is worth showing. */
export function criteriaCount(f: Filter): number {
  return (
    (f.rating !== undefined ? 1 : 0) +
    (f.color !== undefined ? 1 : 0) +
    (f.cameras?.length ? 1 : 0) +
    (f.keywords_under?.length ?? 0) +
    (f.collections?.length ?? 0) +
    (f.unkeyworded ? 1 : 0) +
    (f.uncollected !== undefined ? 1 : 0) +
    (f.placed !== undefined ? 1 : 0) +
    (f.located !== undefined ? 1 : 0)
  );
}

/** Light classes a photo can have several of (the rest, dark/medium/bright, are one each). */
export const LIGHT_PROBLEMS = ["clipped highlights", "blocked shadows", "low contrast"];

/** Which analysis cards ask for every pick ("red and blue") rather than any of them. */
export type AllOf = { hues?: boolean; problems?: boolean };

/**
 * The filter with the picks of the cards set to "all" made into `all_of` alternatives, one
 * per value: "red and blue" is a photo that is red and is blue. One pick needs no change.
 * Brightness picks (dark, medium, bright) stay "any": a photo has only one.
 */
export function allOfPicks(f: Filter, all: AllOf): Filter {
  const out: Filter = { ...f };
  const each: Filter[] = [];
  if (all.hues && (f.hues?.length ?? 0) > 1) {
    for (const hue of f.hues!) each.push({ hues: [hue] });
    delete out.hues;
  }
  const problems = (f.light ?? []).filter((l) => LIGHT_PROBLEMS.includes(l));
  if (all.problems && problems.length > 1) {
    for (const p of problems) each.push({ light: [p] });
    const rest = (f.light ?? []).filter((l) => !LIGHT_PROBLEMS.includes(l));
    if (rest.length) out.light = rest;
    else delete out.light;
  }
  return each.length ? { ...out, all_of: [...(out.all_of ?? []), ...each] } : f;
}
