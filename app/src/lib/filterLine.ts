// The filter line above the grid: one pill per section, in the sidebar's order, that says
// how its values combine ("X-T5 or X100VI", "Food and Kraków", "Poland except Kraków").
// Pills always narrow together. Kept apart from the Svelte state so it can be tested.

import type { Filter } from "./api";
import { joinOf, joined, type Combine, type CombineKey, type Join } from "./criteria";
import type { SectionKey } from "./sections";

/** One value of the filter, as its field's chips say it. `rest` is what the field holds
 *  without it (undefined lets go of the field). */
export type Term = {
  key: keyof Filter;
  text: string;
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

/**
 * The terms (in the order the fields' chips list them) as pills. Sections go in `order`
 * (the sidebar's), after the search; the rest keep the order of their terms.
 */
export function pills(terms: Term[], order: readonly SectionKey[], filter: Filter, combine: Combine): Pill[] {
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
  const rank = (id: string) => {
    if (id === "search") return -1;
    const i = order.indexOf(id as SectionKey);
    return i < 0 ? order.length : i;
  };
  // A stable sort: pills outside the sidebar keep the order their terms came in.
  return [...byId.values()].sort((a, b) => rank(a.id) - rank(b.id));
}

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
