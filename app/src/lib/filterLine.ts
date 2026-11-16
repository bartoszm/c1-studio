// The filter line above the grid: one pill per section, in the sidebar's order, that says
// how its values combine ("X-T5 or X100VI", "Food and Kraków", "Poland except Kraków").
// Pills narrow together, but those in the "either" group, which are alternatives to each
// other ("Kraków or black & white"). Kept apart from the Svelte state so it can be tested.

import type { Filter } from "./api";
import { combinePicks, joinOf, joined, type Combine, type CombineKey, type Join } from "./criteria";
import type { SectionKey } from "./sections";

/** One value of the filter, as its field's chips say it. `rest` is what the field holds
 *  without it (undefined lets go of the field). */
export type Term = {
  key: keyof Filter;
  text: string;
  /** The value itself, for a field that can show it or leave it out (a click flips it). */
  value?: string;
  what?: string;
  out?: boolean;
  title?: string;
  dot?: string;
  stars?: boolean;
  rest?: unknown;
};

/** Values of one field, joined by `join` (a switch when `combine` says which). */
export type Part = { terms: Term[]; join: Join; combine?: CombineKey };

export type Pill = {
  id: string;
  label?: string;
  /** What is picked in: parts that narrow together ("and" between them). */
  parts: Part[];
  /** What is left out: "except …", or "not …" when nothing is picked in. */
  outs: Term[];
  /** Every field the pill holds, to drop them all. */
  keys: (keyof Filter)[];
  /** One of the "either" group: an alternative to the group's other pills. */
  inEither?: boolean;
};

/** The sidebar section a field's pill sits with. Fields not listed have a pill of their own. */
const SECTION: Partial<Record<keyof Filter, SectionKey | "search">> = {
  text: "search",
  collections: "albums", not_in_collections: "albums", uncollected: "albums",
  // A trip and a range of days in it narrow together: one pill for when.
  year: "date", month: "date", week: "date", captured: "date", trip: "date",
  places_under: "places", places_under_not: "places", placed: "places", located: "places",
  gps: "places", approx: "places", country: "places", no_country_code: "places",
  folder: "folders",
  keywords_under: "keywords", keywords_under_not: "keywords", unkeyworded: "keywords",
  rating: "rating",
  color: "color", colors_not: "color",
  hues: "hue",
  camera: "camera", cameras: "camera", cameras_not: "camera", make: "camera", makes: "camera", makes_not: "camera",
  lens: "lens", lenses: "lens", lenses_not: "lens",
  ext: "ext", ext_not: "ext",
  orientation: "orientation", orientation_not: "orientation",
};

const LABEL: Partial<Record<SectionKey | "search", string>> = {
  search: "Search", albums: "Album", date: "Date", places: "Place", folders: "Folder", keywords: "Keyword",
  color: "Colour", hue: "Main colour", camera: "Camera", lens: "Lens", ext: "Format", orientation: "Orientation",
};

/** Fields whose several values a photo can have all of, and the section switch that joins them. */
const COMBINED: Partial<Record<keyof Filter, CombineKey>> = {
  keywords_under: "keywords", collections: "albums", hues: "hues", light: "problems",
};

/** The pill a field's picks show in: its section's, or its own. */
export const pillOf = (key: keyof Filter): string => SECTION[key] ?? key;

/** The fields a pill holds: its section's, or the one field with a pill of its own. */
export function fieldsOf(id: string): (keyof Filter)[] {
  const mine = (Object.keys(SECTION) as (keyof Filter)[]).filter((k) => SECTION[k] === id);
  return mine.length ? mine : [id as keyof Filter];
}

const holds = (f: Filter, id: string) => fieldsOf(id).some((k) => f[k] !== undefined);

/** The "either" group as kept: pills without picks leave it. One can wait there for a
 *  second (Shift+click Kraków, then black & white); the same list when nothing left. */
export function tidyEither(f: Filter, either: readonly string[]): readonly string[] {
  const kept = either.filter((id) => holds(f, id));
  return kept.length === either.length ? either : kept;
}

/** The "either" group as asked: two pills or more, or none (one alone is no choice). */
export function keptEither(f: Filter, either: readonly string[]): string[] {
  const kept = either.filter((id) => holds(f, id));
  return kept.length > 1 ? kept : [];
}

/**
 * What to ask the index: each section's picks joined its way (`combinePicks`), and the
 * pills in the "either" group made alternatives to each other: Kraków or black & white,
 * and everything else as well.
 */
export function eitherPicks(f: Filter, either: readonly string[], combine: Combine): Filter {
  const group = keptEither(f, either);
  if (!group.length) return combinePicks(f, combine);
  const rest: Filter = { ...f };
  const branches = group.map((id) => {
    const branch: Filter = {};
    for (const k of fieldsOf(id)) {
      if (f[k] === undefined) continue;
      (branch as Record<string, unknown>)[k] = f[k];
      delete rest[k];
    }
    return combinePicks(branch, combine);
  });
  const base = combinePicks(rest, combine);
  return { ...base, all_of: [...(base.all_of ?? []), { any_of: branches }] };
}

/**
 * The terms (in the order the fields' chips list them) as pills. The "either" group goes
 * first, then the search, then sections in `order` (the sidebar's); the rest keep the
 * order of their terms.
 */
export function pills(terms: Term[], order: readonly SectionKey[], filter: Filter, combine: Combine, either: readonly string[] = []): Pill[] {
  const byId = new Map<string, Pill>();
  for (const t of terms) {
    const section = SECTION[t.key];
    const id = section ?? t.key;
    let pill = byId.get(id);
    if (!pill) {
      pill = { id, label: section ? LABEL[section] : t.what, parts: [], outs: [], keys: [] };
      byId.set(id, pill);
    }
    if (!pill.keys.includes(t.key)) pill.keys.push(t.key);
    // The pill's label says it already.
    const term = t.what === pill.label ? { ...t, what: undefined } : t;
    if (t.out) {
      pill.outs.push(term);
      continue;
    }
    const last = pill.parts[pill.parts.length - 1];
    if (last && last.terms[0].key === t.key) {
      last.terms.push(term);
      continue;
    }
    const combineKey = COMBINED[t.key];
    // Light's brightness picks are one each: only its problems can ask for all.
    const join: Join = combineKey && joined(filter, combineKey).length > 1 ? joinOf(combine, combineKey) : "any";
    pill.parts.push({ terms: [term], join, combine: combineKey });
  }
  const group = keptEither(filter, either);
  const rank = (id: string) => {
    const at = id === "search" ? -1 : order.indexOf(id as SectionKey);
    const i = at < 0 && id !== "search" ? order.length : at;
    return group.includes(id) ? i - 1000 : i;
  };
  // A stable sort: pills outside the sidebar keep the order their terms came in.
  return [...byId.values()]
    .sort((a, b) => rank(a.id) - rank(b.id))
    .map((p) => (group.includes(p.id) ? { ...p, inEither: true } : p));
}

/** The "either" group as one line of text. */
export const eitherText = (ps: Pill[]) => `either ${ps.map(pillText).join(" or ")}`;

/** The word between two values of a part. */
export const joinWord = (join: Join) => (join === "all" ? "and" : "or");

/** The pill as one line of text: for its tooltip and screen readers. */
export function pillText(p: Pill): string {
  const term = (t: Term) => (t.what ? `${t.what} ${t.text}` : t.text);
  const ins = p.parts.map((part) => part.terms.map(term).join(` ${joinWord(part.join)} `)).join(" and ");
  const outs = p.outs.map(term).join(", ");
  const body = ins && outs ? `${ins} except ${outs}` : ins || `not ${outs}`;
  return p.label ? `${p.label}: ${body}` : body;
}
