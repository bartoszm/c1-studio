// How photos are ordered, as the header shows it: a field to sort by and a direction that
// says in words what it does for that field ("Newest first", "Sharpest first", "A to Z").
import type { Filter, Sort } from "./api";

export type SortField = "date" | "rating" | "label" | "imported" | "name" | "sharpness" | "brightness" | "match";

export type FieldInfo = {
  key: SortField;
  label: string;
  /** The usual direction, and the way round: what each says. */
  first: string;
  second: string;
  /** The index's sorts for the usual direction and the other. */
  sorts: [Sort, Sort];
};

export const FIELDS: FieldInfo[] = [
  { key: "date", label: "Date taken", first: "Newest first", second: "Oldest first", sorts: ["DateDesc", "Date"] },
  { key: "rating", label: "Rating", first: "Highest first", second: "Lowest first", sorts: ["RatingDesc", "Rating"] },
  { key: "label", label: "Colour label", first: "Red to purple", second: "Purple to red", sorts: ["ColorLabel", "ColorLabelRev"] },
  { key: "imported", label: "Imported", first: "Last imported", second: "First imported", sorts: ["ImportedDesc", "Imported"] },
  { key: "name", label: "File name", first: "A to Z", second: "Z to A", sorts: ["Filename", "FilenameDesc"] },
  { key: "sharpness", label: "Sharpness", first: "Sharpest first", second: "Softest first", sorts: ["SharpnessDesc", "Sharpness"] },
  { key: "brightness", label: "Brightness", first: "Brightest first", second: "Darkest first", sorts: ["BrightnessDesc", "Brightness"] },
  { key: "match", label: "Best match", first: "Best match first", second: "Worst match first", sorts: ["Match", "MatchWorst"] },
];

export const DEFAULT_FIELD: SortField = "date";

export const info = (field: SortField): FieldInfo => FIELDS.find((f) => f.key === field) ?? FIELDS[0];

/** The index's sort for a field and direction (`reversed`: the other way round from usual). */
export const toSort = (field: SortField, reversed: boolean): Sort => info(field).sorts[reversed ? 1 : 0];

/** What the direction button says. */
export const words = (field: SortField, reversed: boolean): string => (reversed ? info(field).second : info(field).first);

/** Whether the filter picks anything Best match can rank by: a colour (not just "muted"), light or focus. */
export function matchAvailable(f: Filter): boolean {
  return !!f.hues?.some((h) => h !== "muted") || !!f.light?.length || !!f.focus?.length;
}

/** The field the grid is sorted by: Best match has nothing to match without a pick, so the date stands in. */
export const effectiveField = (field: SortField, f: Filter): SortField =>
  field === "match" && !matchAvailable(f) ? DEFAULT_FIELD : field;

export type Stored = { field: SortField; reversed: boolean };

/** A stored choice back: the current form `{field, reversed}`, or the old single value of the first dropdown. */
export function restore(text: string | null, old: string | null): Stored {
  try {
    const v = JSON.parse(text ?? "null");
    if (v && FIELDS.some((f) => f.key === v.field)) return { field: v.field, reversed: v.reversed === true };
  } catch {
    // Falls through to the old value.
  }
  for (const f of FIELDS) {
    const at = f.sorts.indexOf(old as Sort);
    if (at >= 0) return { field: f.key, reversed: at === 1 };
  }
  return { field: DEFAULT_FIELD, reversed: false };
}
