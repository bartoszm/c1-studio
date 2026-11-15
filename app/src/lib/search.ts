// Search-box tokens: `iso:>1600 f:<2.8 lens:23 orient:portrait street` asks for the photos
// those fields allow, and the words left over are the free text. Kept apart from the Svelte
// state so the rules can be tested on their own.

import type { Filter, Range } from "./api";

/** What a token sets. */
export type Key = "camera" | "make" | "lens" | "iso" | "aperture" | "focal" | "shutter" | "ext" | "size" | "mp" | "width" | "height" | "orient";

/** Every name a token can go by. */
export const KEYS: Record<string, Key> = {
  camera: "camera",
  model: "camera",
  make: "make",
  lens: "lens",
  iso: "iso",
  f: "aperture",
  aperture: "aperture",
  focal: "focal",
  mm: "focal",
  shutter: "shutter",
  ss: "shutter",
  ext: "ext",
  format: "ext",
  size: "size",
  mp: "mp",
  megapixels: "mp",
  width: "width",
  height: "height",
  orient: "orient",
  orientation: "orient",
};

/** Names standing for several extensions. */
export const FORMATS: Record<string, string[]> = {
  raw: ["raf", "arw", "nef", "cr2", "cr3", "crw", "dng", "orf", "rw2", "pef", "srw", "iiq", "3fr", "rwl", "x3f", "erf", "kdc", "mos", "nrw", "sr2", "srf"],
  jpeg: ["jpg", "jpeg"],
  jpg: ["jpg", "jpeg"],
  tiff: ["tif", "tiff"],
  tif: ["tif", "tiff"],
  heif: ["heic", "heif"],
  heic: ["heic", "heif"],
};

export const ORIENTATIONS = ["landscape", "portrait", "square", "panorama"];

/** Help for the search box: each token with an example. */
export const HELP: { token: string; means: string }[] = [
  { token: "camera:x100", means: "camera model contains" },
  { token: "make:fuji", means: "camera maker contains" },
  { token: "lens:23", means: "lens contains" },
  { token: "iso:>1600", means: "ISO; also <, >=, <=, 200-800" },
  { token: "f:<2.8", means: "aperture" },
  { token: "focal:35-50", means: "focal length, mm" },
  { token: "shutter:>1/30", means: "exposure; 2s for seconds" },
  { token: "format:raw", means: "file type: raf, jpg, tiff, raw…" },
  { token: "-format:jpg", means: "not that file type" },
  { token: "size:>30mb", means: "file size; kb, mb (default), gb" },
  { token: "mp:<12", means: "megapixels as shown (cropped)" },
  { token: "width:>=6000", means: "pixels as shown; also height:" },
  { token: "orient:portrait", means: "landscape, portrait, square, panorama" },
  { token: '"a phrase"', means: "other words search names, keywords, captions, places" },
];

/** The filter fields each key sets, so a token typed again replaces what was there. */
export const FIELDS: Record<Key, (keyof Filter)[]> = {
  camera: ["camera"],
  make: ["make"],
  lens: ["lens"],
  iso: ["iso"],
  aperture: ["aperture"],
  focal: ["focal"],
  shutter: ["shutter"],
  ext: ["ext", "ext_not"],
  size: ["file_size"],
  mp: ["megapixels"],
  width: ["width"],
  height: ["height"],
  orient: ["orientation", "orientation_not"],
};

type RangeKey = "iso" | "aperture" | "focal" | "shutter" | "size" | "mp" | "width" | "height";
const RANGE_FIELD: Record<RangeKey, keyof Filter> = {
  iso: "iso",
  aperture: "aperture",
  focal: "focal",
  shutter: "shutter",
  size: "file_size",
  mp: "megapixels",
  width: "width",
  height: "height",
};

const NUMBER = /^\d+(\.\d+)?$|^\.\d+$/;
const plain = (s: string) => (NUMBER.test(s) ? Number(s) : undefined);

/** A value as the index holds it, from what was typed. */
const VALUE: Record<RangeKey, (s: string) => number | undefined> = {
  iso: plain,
  aperture: (s) => plain(s.replace(/^f\/?/i, "")),
  focal: (s) => plain(s.replace(/\s*mm$/i, "")),
  shutter: (s) => {
    const frac = /^(\d+(?:\.\d+)?)\/(\d+(?:\.\d+)?)$/.exec(s);
    if (frac) return +frac[2] > 0 ? +frac[1] / +frac[2] : undefined;
    return plain(s.replace(/\s*(s|sec|")$/i, ""));
  },
  size: (s) => {
    const m = /^(\d+(?:\.\d+)?|\.\d+)\s*(b|k|kb|m|mb|g|gb)?$/i.exec(s);
    if (!m) return undefined;
    const unit = (m[2] ?? "mb").toLowerCase()[0];
    return Math.round(+m[1] * (unit === "b" ? 1 : unit === "k" ? 1e3 : unit === "g" ? 1e9 : 1e6));
  },
  mp: plain,
  width: plain,
  height: plain,
};

const EXAMPLE: Record<RangeKey, string> = {
  iso: "iso:>1600",
  aperture: "f:<2.8",
  focal: "focal:35-50",
  shutter: "shutter:>1/30",
  size: "size:>30mb",
  mp: "mp:>20",
  width: "width:>=6000",
  height: "height:>=4000",
};

/**
 * How far a typed value reaches either way. The catalog keeps f/2.8 as 2.7999…, and a
 * shutter of 1/250 as 0.004 or 0.0039…: every bound is moved out by it, so `<=2.8` keeps
 * 2.8000001 and `>2.8` leaves out 2.7999. An exact size or megapixel count is never meant
 * either, but `mp:<12` is: for those (`about`), only a value or span reaches out.
 */
type Slack = { abs?: number; rel?: number; about?: boolean };
const SLACK: Record<RangeKey, Slack> = {
  iso: {},
  aperture: { abs: 0.05 },
  focal: { abs: 0.5 },
  shutter: { rel: 0.02 },
  size: { rel: 0.05, about: true },
  mp: { abs: 0.5, about: true },
  width: {},
  height: {},
};

/** Whether a range's bounds were moved out: always, or for `about` keys when it has both. */
const slackened = (key: RangeKey, both: boolean) => !SLACK[key].about || both;

/** `v` moved out (`dir` 1 up, -1 down) by the key's slack. */
function out(key: RangeKey, v: number, dir: 1 | -1, both = true): number {
  if (!slackened(key, both)) return v;
  const { abs, rel } = SLACK[key];
  if (abs) return v + dir * abs;
  if (rel) return v * (1 + dir * rel);
  return v;
}

/** The typed value back from a bound moved out with [`out`]. */
function back(key: RangeKey, b: number, dir: 1 | -1, both: boolean): number {
  if (!slackened(key, both)) return b;
  const { abs, rel } = SLACK[key];
  if (abs) return b - dir * abs;
  if (rel) return b / (1 + dir * rel);
  return b;
}

/** `>=4`, `<=2.8`, `>3`, `<2.8`, `=3`, `2-4`, `3`, as `Range::parse` takes them, with slack. */
export function parseRange(key: RangeKey, text: string): Range | undefined {
  const t = text.trim();
  const value = VALUE[key];
  const dash = t.indexOf("-", 1);
  if (dash > 0) {
    const [a, b] = [value(t.slice(0, dash).trim()), value(t.slice(dash + 1).trim())];
    if (a === undefined || b === undefined) return undefined;
    return { min: out(key, Math.min(a, b), -1), max: out(key, Math.max(a, b), 1) };
  }
  for (const op of [">=", "<=", ">", "<", "="]) {
    if (!t.startsWith(op)) continue;
    const v = value(t.slice(op.length).trim());
    if (v === undefined) return undefined;
    switch (op) {
      case ">=": return { min: out(key, v, -1, false) };
      case "<=": return { max: out(key, v, 1, false) };
      case ">": return { min: out(key, v, 1, false), min_exclusive: true };
      case "<": return { max: out(key, v, -1, false), max_exclusive: true };
      default: return { min: out(key, v, -1), max: out(key, v, 1) };
    }
  }
  const v = value(t);
  return v === undefined ? undefined : { min: out(key, v, -1), max: out(key, v, 1) };
}

/** A range as typed: the slack taken back off. */
export function typedRange(key: RangeKey, r: Range): { min?: number; max?: number; minEx: boolean; maxEx: boolean } {
  const minEx = !!r.min_exclusive;
  const maxEx = !!r.max_exclusive;
  const both = r.min !== undefined && r.max !== undefined;
  return {
    min: r.min === undefined ? undefined : back(key, r.min, minEx ? 1 : -1, both),
    max: r.max === undefined ? undefined : back(key, r.max, maxEx ? -1 : 1, both),
    minEx,
    maxEx,
  };
}

const round = (v: number, places = 2) => +v.toFixed(places);

/** A value as people say it: `f/2.8`, `1/250 s`, `30 MB`. */
export function valueText(key: RangeKey, v: number): string {
  switch (key) {
    case "aperture": return `f/${round(v, 1)}`;
    case "focal": return `${round(v, 1)} mm`;
    case "shutter": return v > 0 && v < 1 ? `1/${Math.round(1 / v)} s` : `${round(v, 1)} s`;
    case "size": return v >= 1e9 ? `${round(v / 1e9, 1)} GB` : v >= 1e6 ? `${round(v / 1e6, 1)} MB` : `${Math.round(v / 1e3)} kB`;
    case "mp": return `${round(v, 1)} MP`;
    case "width":
    case "height": return `${Math.round(v)} px`;
    default: return String(round(v));
  }
}

/** A chip's words for a range: `≥ 1600`, `< f/2.8`, `1/250 s`, `35 mm – 50 mm`. */
export function rangeChip(key: RangeKey, r: Range): string {
  const t = typedRange(key, r);
  const v = (x: number) => valueText(key, x);
  if (t.min !== undefined && t.max !== undefined) {
    if (!t.minEx && !t.maxEx && Math.abs(t.min - t.max) < 1e-9 * Math.max(1, Math.abs(t.max))) return v(t.min);
    return `${v(t.min)} – ${v(t.max)}`;
  }
  if (t.min !== undefined) return `${t.minEx ? ">" : "≥"} ${v(t.min)}`;
  if (t.max !== undefined) return `${t.maxEx ? "<" : "≤"} ${v(t.max)}`;
  return "any";
}

/** The range keys, with the filter field each sets and its chip's name. */
export const RANGES: { key: RangeKey; field: keyof Filter; what: string }[] = [
  { key: "iso", field: "iso", what: "ISO" },
  { key: "aperture", field: "aperture", what: "Aperture" },
  { key: "focal", field: "focal", what: "Focal length" },
  { key: "shutter", field: "shutter", what: "Shutter" },
  { key: "size", field: "file_size", what: "File size" },
  { key: "mp", field: "megapixels", what: "Megapixels" },
  { key: "width", field: "width", what: "Width" },
  { key: "height", field: "height", what: "Height" },
];

export type SearchError = { token: string; message: string };

export type Parsed = {
  /** What the tokens set. */
  filter: Filter;
  /** The keys set, so committing them replaces what the filter had for each. */
  keys: Key[];
  /** The free text: words and quoted phrases that are not tokens (or name no known key). */
  rest: string;
  /** What stays in the box once the tokens are taken out: the free text, plus tokens not
   *  finished or not understood, for fixing. */
  left: string;
  errors: SearchError[];
};

// A token (`-key:value`, the value maybe quoted), a quoted phrase, or a word.
const PIECE = /(-?)([A-Za-z]+):("[^"]*"?|[^\s"]*)|"[^"]*"?|[^\s"]+/g;

const unquote = (v: string) => v.replace(/^"/, "").replace(/"$/, "").trim();
const unique = <T,>(list: T[]) => [...new Set(list)];

/** The box's text as filter fields and free text. */
export function parseSearch(text: string): Parsed {
  const filter: Filter = {};
  const keys: Key[] = [];
  const rest: string[] = [];
  const left: string[] = [];
  const errors: SearchError[] = [];
  const fail = (token: string, message: string) => {
    errors.push({ token, message });
    left.push(token);
  };
  for (const m of text.matchAll(PIECE)) {
    const [token, minus, name, raw] = m;
    const key = name ? KEYS[name.toLowerCase()] : undefined;
    if (!key) {
      rest.push(token);
      left.push(token);
      continue;
    }
    const value = unquote(raw ?? "");
    // Still being typed: nothing yet, and no complaint.
    if (!value) {
      left.push(token);
      continue;
    }
    if (minus && key !== "ext" && key !== "orient") {
      fail(token, `${name}: cannot be excluded; only format: and orient: can`);
      continue;
    }
    switch (key) {
      case "camera":
      case "make":
      case "lens":
        filter[key] = value;
        break;
      case "ext": {
        const list = value.toLowerCase().split(",").map((e) => e.trim().replace(/^\./, "")).filter(Boolean);
        const bad = list.find((e) => !/^[a-z0-9]+$/.test(e));
        if (bad !== undefined || !list.length) {
          fail(token, `${name}: takes file types like raf, jpg or raw`);
          continue;
        }
        const exts = list.flatMap((e) => FORMATS[e] ?? [e]);
        const field = minus ? "ext_not" : "ext";
        filter[field] = unique([...(filter[field] ?? []), ...exts]);
        break;
      }
      case "orient": {
        const words = value.toLowerCase().split(",").map((w) => w.trim()).filter(Boolean);
        const found = words.map((w) => ORIENTATIONS.find((o) => o.startsWith(w)));
        if (!words.length || found.some((o) => o === undefined)) {
          fail(token, `${name}: takes ${ORIENTATIONS.join(", ")}`);
          continue;
        }
        const field = minus ? "orientation_not" : "orientation";
        filter[field] = unique([...(filter[field] ?? []), ...(found as string[])]);
        break;
      }
      default: {
        const range = parseRange(key, value);
        if (!range) {
          fail(token, `${name}: needs a number or range, like ${EXAMPLE[key]}`);
          continue;
        }
        (filter as Record<string, unknown>)[RANGE_FIELD[key]] = range;
      }
    }
    if (!keys.includes(key)) keys.push(key);
  }
  return { filter, keys, rest: rest.join(" "), left: left.join(" "), errors };
}

/** `base` with what the tokens set: each key's fields replaced by the tokens'. */
export function withSearch(base: Filter, parsed: Parsed): Filter {
  const out: Filter = { ...base };
  for (const key of parsed.keys) {
    for (const field of FIELDS[key]) delete out[field];
  }
  return { ...out, ...parsed.filter };
}
