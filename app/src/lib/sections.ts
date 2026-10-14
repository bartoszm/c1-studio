// The sidebar's sections: which are shown, in what order. Kept apart from the Svelte
// state so the rules can be tested on their own.

import type { Dim } from "./api";

/** Every section, in the order a fresh install shows them. */
export const SECTION_KEYS = ["albums", "date", "places", "folders", "keywords", "rating", "color", "hue", "camera"] as const;
export type SectionKey = (typeof SECTION_KEYS)[number];

export type SectionPref = { key: SectionKey; hidden: boolean };

/** The facet groups a section's counts come from. */
export const SECTION_DIMS: Record<SectionKey, Dim[]> = {
  albums: ["collection"],
  date: ["year", "month", "week"],
  places: ["place_tree"],
  folders: ["folder"],
  keywords: ["keyword_tree"],
  rating: ["rating"],
  color: ["color"],
  hue: ["hue"],
  camera: ["camera"],
};

export const defaultSections = (): SectionPref[] => SECTION_KEYS.map((key) => ({ key, hidden: false }));

const known = (key: unknown): key is SectionKey => (SECTION_KEYS as readonly unknown[]).includes(key);

/**
 * The stored list made whole: keys no longer known are dropped, repeats too, and a
 * section added since goes in after the one it follows by default (first, if none).
 */
export function mergeSections(stored: unknown): SectionPref[] {
  const out: SectionPref[] = [];
  if (Array.isArray(stored)) {
    for (const s of stored) {
      if (known(s?.key) && !out.some((o) => o.key === s.key)) out.push({ key: s.key, hidden: s.hidden === true });
    }
  }
  SECTION_KEYS.forEach((key, i) => {
    if (out.some((o) => o.key === key)) return;
    let at = 0;
    for (let j = i - 1; j >= 0; j--) {
      const k = out.findIndex((o) => o.key === SECTION_KEYS[j]);
      if (k >= 0) {
        at = k + 1;
        break;
      }
    }
    out.splice(at, 0, { key, hidden: false });
  });
  return out;
}

/** The list with the item at `from` moved to `to`. */
export function move<T>(list: T[], from: number, to: number): T[] {
  if (from === to || from < 0 || from >= list.length) return list;
  const out = [...list];
  const [item] = out.splice(from, 1);
  out.splice(Math.max(0, Math.min(out.length, to)), 0, item);
  return out;
}
