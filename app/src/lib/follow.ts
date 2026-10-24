// A filter that follows an edit. Photos edited until none is left in the filter (the two
// photos of a place both moved to another city) take the filter with them: the fields
// they no longer match get the values they have now. Filters that are a list of work to
// do (no keywords, no place, not in an album…) never follow: emptying them is the point.

import type { Color, Edit, Filter, Merged, Range, Selection } from "./api";
import { ratingFor } from "./criteria";
import { keyOf } from "./places";

/** Fields whose emptying is the goal, not something to undo by looking elsewhere. */
export function isWorkList(f: Filter): boolean {
  const some = (list?: unknown[]) => !!list?.length;
  return (
    !!f.text ||
    !!f.unkeyworded ||
    f.uncollected === true ||
    f.placed === false ||
    f.located === false ||
    f.gps === false ||
    !!f.no_country_code ||
    f.analysed === false ||
    !!f.history ||
    f.bin === "Only" ||
    some(f.colors_not) ||
    some(f.cameras_not) ||
    some(f.keywords_under_not) ||
    some(f.not_in_collections)
  );
}

/** The values the photos have, and whether some have none. */
const valuesOf = <T>(m: Merged<T>): T[] => (m.same !== null ? [m.same] : m.top.map(([v]) => v));
/** Every value is listed: `top` is cut at twelve. */
const complete = <T>(m: Merged<T>) => m.same !== null || m.values <= m.top.length;

function inRange(r: Range, v: number): boolean {
  if (r.min !== undefined && (r.min_exclusive ? v <= r.min : v < r.min)) return false;
  if (r.max !== undefined && (r.max_exclusive ? v >= r.max : v > r.max)) return false;
  return true;
}

/** Whether every photo matches a `Presence` filter, by the values they have merged. */
function allMatch<T, F>(p: "None" | "Any" | { Values: F }, m: Merged<T>, has: (f: F, v: T) => boolean): boolean {
  if (p === "None") return m.values === 0;
  if (p === "Any") return m.empty === 0;
  return m.empty === 0 && complete(m) && valuesOf(m).every((v) => has(p.Values, v));
}

/** What a `Presence` filter becomes to hold the photos: none, one value, or all of theirs.
 *  Null when some have a value and some none, or the values are not all known. */
function presenceFor<T>(m: Merged<T>): "None" | { values: T[] } | null {
  if (m.values === 0) return "None";
  if (m.empty > 0 || !complete(m)) return null;
  return { values: valuesOf(m) };
}

/** The place key the photos share, down to `depth` levels at most; "" when none. */
function commonPlace(keys: string[], depth: number): string {
  if (!keys.length) return "";
  const split = keys.map((k) => k.split("|"));
  const common: string[] = [];
  for (let i = 0; i < depth; i++) {
    const level = split[0][i];
    if (level === undefined || split.some((s) => s[i] !== level)) break;
    common.push(level);
  }
  return common.join("|");
}

const under = (key: string, path: string) => key === path || key.startsWith(path + "|");

/**
 * The filter `f` changed to hold the photos `sel` describes (`n` of them), now that an edit
 * took them all out of it; null when it should stay as it is: a work list, Match any, or a
 * field broken that no value of theirs can mend (a place cleared, a keyword removed).
 * `edit` is the edit when one is known: the keywords it added stand in for one removed.
 */
export function followFilter(f: Filter, sel: Selection, n: number, edit?: Edit, any = false): Filter | null {
  if (!n || any || isWorkList(f)) return null;
  const next: Filter = { ...f };
  let changed = false;

  if (f.rating !== undefined && !allMatch(f.rating, sel.rating, inRange)) {
    const to = presenceFor(sel.rating);
    if (!to) return null;
    // Only the stars they have now: an operator kept (≥ 4 set to 2 giving ≥ 2) would take
    // in every photo rated above them too.
    if (to === "None") next.rating = "None";
    else if (to.values.length === 1) next.rating = ratingFor("=", to.values[0]);
    else next.rating = { Values: { min: Math.min(...to.values), max: Math.max(...to.values) } };
    changed = true;
  }

  if (f.color !== undefined && !allMatch(f.color, sel.color, (cs: Color[], c) => cs.includes(c))) {
    const to = presenceFor(sel.color);
    if (!to) return null;
    next.color = to === "None" ? "None" : { Values: to.values };
    changed = true;
  }

  const placeKeys = sel.places.map(([p]) => keyOf(p));
  if (f.place_under && !placeKeys.every((k) => under(k, f.place_under!))) {
    const to = commonPlace(placeKeys, f.place_under.split("|").length);
    if (!to) return null;
    next.place_under = to;
    changed = true;
  }
  if (f.country !== undefined) {
    const countries = new Set(sel.places.map(([p]) => p.country ?? ""));
    if (countries.size !== 1 || !countries.has(f.country)) {
      const [to] = countries;
      if (countries.size !== 1 || !to) return null;
      next.country = to;
      changed = true;
    }
  }

  if (f.keywords_under?.length) {
    // A path none of them has a keyword under is one they left. (One some have is not
    // known to be on all of them; the photos are looked for in the new filter anyway.)
    const onNone = (path: string) => !sel.keywords.some(([k]) => under(k, path));
    const gone = f.keywords_under.filter(onNone);
    if (gone.length) {
      const added = edit && "AddKeywords" in edit ? edit.AddKeywords.filter((k) => !f.keywords_under!.includes(k)) : [];
      if (!added.length) return null;
      next.keywords_under = [...f.keywords_under.filter((p) => !gone.includes(p)), ...added];
      changed = true;
    }
  }

  if (f.collections?.length && edit && "AddToAlbum" in edit) {
    const inAll = (id: string) => sel.albums.some(([a, c]) => a === id && c >= n);
    if (!f.collections.some(inAll)) {
      next.collections = [edit.AddToAlbum];
      changed = true;
    }
  }

  return changed ? next : null;
}
