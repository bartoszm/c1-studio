// The analysis settings as the dialog shows them: each a number the user can read
// (percent, seconds), kept by the backend in its own unit (a share 0..1, milliseconds).
import type { ClassPreview, ClassSettings, ColourMode } from "./api";

/** The settings that are numbers (the colour mode is a choice of its own). */
export type NumberKey = { [K in keyof ClassSettings]: ClassSettings[K] extends number ? K : never }[keyof ClassSettings];

export type Field = {
  key: NumberKey;
  label: string;
  hint: string;
  /** Shown = stored × scale. */
  scale: number;
  unit: string;
  min: number;
  max: number;
  step: number;
  /** Shown only in these colour modes (all when left out). */
  modes?: ColourMode[];
};

export type Group = {
  title: string;
  /** What it classes: the preview counts of these `<dimension>|<class>` keys, in order. */
  classes: string[];
  fields: Field[];
};

const pct = (key: NumberKey, label: string, hint: string, max = 100): Field => ({
  key, label, hint, scale: 100, unit: "%", min: 0, max, step: 1,
});

export const GROUPS: Group[] = [
  {
    title: "Colour",
    classes: ["hue|black & white", "hue|muted"],
    fields: [
      { ...pct("colour_share", "A colour counts when it covers", "of the photo's pixels. Lower it to catch smaller areas; raise it for photos that are mostly that colour."), modes: ["Covers"] },
      { ...pct("patch_share", "A colour counts with a patch of", "of the photo in one piece, vivid enough (below). A red coat on a grey street counts; red specks scattered over a wall do not."), step: 0.5, modes: ["Patch"] },
      { key: "patch_chroma", label: "A patch is vivid from chroma", hint: "Colourfulness (Lab) of the patch's pixels. A vivid patch also keeps a grey photo out of black & white.", scale: 1, unit: "", min: 0, max: 100, step: 1 },
      pct("colour_floor", "Else the biggest colour counts from", "of the photo (its biggest patch, in patch mode); below that the photo is muted."),
      { key: "mono_chroma", label: "Black & white below chroma", hint: "Mean colourfulness (Lab). Toned prints stay black & white up to here, unless they have a vivid patch.", scale: 1, unit: "", min: 0, max: 60, step: 0.5 },
    ],
  },
  {
    title: "Light",
    classes: ["light|dark", "light|medium", "light|bright", "light|clipped highlights", "light|blocked shadows", "light|low contrast"],
    fields: [
      pct("dark_below", "Dark below brightness", "Mean brightness, 0 black to 100 white."),
      pct("bright_above", "Bright above brightness", ""),
      pct("clipped_share", "Clipped highlights from", "of the pixels at pure white."),
      pct("blocked_share", "Blocked shadows from", "of the pixels at pure black."),
      { key: "flat_below", label: "Low contrast below", hint: "Spread of brightness (standard deviation), 0 to 0.5.", scale: 1, unit: "", min: 0, max: 0.5, step: 0.01 },
    ],
  },
  {
    title: "Focus",
    classes: ["focus|sharp", "focus|soft", "focus|blurry"],
    fields: [
      { key: "soft_below", label: "Soft below sharpness", hint: "Detail in the sharpest part of the photo. The defaults suit the fuji catalog's grain; a catalog with finer grain may want lower values.", scale: 1, unit: "", min: 0, max: 20000, step: 10 },
      { key: "blurry_below", label: "Blurry below sharpness", hint: "", scale: 1, unit: "", min: 0, max: 20000, step: 10 },
    ],
  },
  {
    title: "Shape",
    classes: ["shape|landscape", "shape|portrait", "shape|square", "shape|panorama"],
    fields: [
      { key: "square_ratio", label: "Square within ratio", hint: "Width over height: 1.1 calls a 6:5 photo square.", scale: 1, unit: "", min: 1, max: 3, step: 0.05 },
      { key: "panorama_ratio", label: "Panorama from ratio", hint: "2 is 2:1 and wider.", scale: 1, unit: "", min: 1, max: 10, step: 0.1 },
    ],
  },
  {
    title: "Alike",
    classes: [],
    fields: [
      { key: "duplicate_bits", label: "Duplicates differ by up to", hint: "Bits of the picture hash, 0 to 3: the same instant, the same picture.", scale: 1, unit: "bits", min: 0, max: 3, step: 1 },
      { key: "series_bits", label: "A series moves by up to", hint: "Bits of the picture hash from one frame to the next.", scale: 1, unit: "bits", min: 0, max: 64, step: 1 },
      { key: "series_gap_ms", label: "A series waits up to", hint: "Between one frame and the next.", scale: 0.001, unit: "s", min: 0, max: 60, step: 0.5 },
      { key: "series_min", label: "A series has at least", hint: "", scale: 1, unit: "photos", min: 2, max: 20, step: 1 },
    ],
  },
];

/** Whether a field is shown in the current colour mode. */
export const inMode = (f: Field, mode: ColourMode) => !f.modes || f.modes.includes(mode);

/** Floats from the backend are f32: 0.17 comes back as 0.1666…, so shown values are rounded. */
const round = (n: number) => Math.round(n * 1000) / 1000;

export const shown = (f: Field, settings: ClassSettings): number => round(settings[f.key] * f.scale);

/** The settings with one field set from what the user typed; an empty or invalid entry leaves them as they were. */
export function withField(settings: ClassSettings, f: Field, typed: number | null): ClassSettings {
  if (typed === null || !Number.isFinite(typed)) return settings;
  const value = Math.min(f.max, Math.max(f.min, typed)) / f.scale;
  return { ...settings, [f.key]: f.scale === 0.001 ? Math.round(value) : value };
}

/** `current` with just the `fields` (and, with `mode`, the colour mode) taken from `draft`: what one
 *  editor applies, so the fields another editor (the Alike tab, the Settings dialog) changed since
 *  it loaded stay as they are. */
export function mergeFields(current: ClassSettings, draft: ClassSettings, fields: Field[], mode = false): ClassSettings {
  const out = { ...current };
  for (const f of fields) out[f.key] = draft[f.key];
  if (mode) out.colour_mode = draft.colour_mode;
  return out;
}

export const sameSettings = (a: ClassSettings, b: ClassSettings) =>
  (Object.keys(a) as (keyof ClassSettings)[]).every((k) => {
    const [x, y] = [a[k], b[k]];
    return typeof x === "number" && typeof y === "number" ? round(x) === round(y) : x === y;
  });

/** "12 blurry · 340 soft": the classes of a group with photos in them, in the group's order. */
export function summary(group: Group, preview: ClassPreview | null): string {
  if (!preview) return "";
  if (group.title === "Alike") {
    return `${preview.duplicates} in duplicate groups · ${preview.series} in series`;
  }
  const classes = group.title === "Colour" ? colourClasses(preview, group) : group.classes;
  return classes
    .map((key) => [key.split("|")[1], preview.counts[key] ?? 0] as const)
    .filter(([, n]) => n > 0)
    .map(([name, n]) => `${n} ${name}`)
    .join(" · ");
}

/** Colours are many (every name the analysis gives): all of them, biggest first, after black & white and muted. */
function colourClasses(preview: ClassPreview, group: Group): string[] {
  const hues = Object.keys(preview.counts).filter((k) => k.startsWith("hue|") && !group.classes.includes(k));
  hues.sort((a, b) => preview.counts[b] - preview.counts[a]);
  return [...group.classes, ...hues];
}
