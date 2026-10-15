// Helpers for the folders provider's dialog and footer: image store presets, a preview of
// what a pattern gives, the excludes as text, the size limit in GB. Pure, so they are
// tested without a window. The presets mirror source-folders' `pattern::preset`.

import type { ImageStore, Preset, RootDef, SourceProgress } from "./api";

/** Patterns for the thumbnail and preview tiers. */
export type Patterns = { thumb: string; preview: string };

/** The provider id the app and the Rust side use. */
export const FOLDERS = "folders";

/** The presets' patterns; the same for both tiers (`{tier}` tells them apart). */
const PATTERNS: Record<Exclude<Preset, "Custom">, string> = {
  Central: "{data}/images/{tier}/{hash:2}/{hash}.jpg",
  PerRoot: "{root}/.c1studio/{tier}/{relpath}.jpg",
  Beside: "{dir}/.previews/{stem}.{tier}.jpg",
};

export const PRESETS: { id: Preset; label: string; about: string }[] = [
  { id: "Central", label: "Central", about: "In the app's data folder. Your photo folders stay clean; the images can be trimmed to a size limit." },
  { id: "PerRoot", label: "Per root", about: "In a .c1studio folder under each root, so they travel with an external drive." },
  { id: "Beside", label: "Beside the file", about: "In a .previews folder next to each photo, named after it." },
  { id: "Custom", label: "Custom", about: "Patterns of your own, using the tokens below." },
];

/** What the tokens of a pattern stand for. */
export const TOKENS: [string, string][] = [
  ["{data}", "the source's data folder"],
  ["{root}", "the root folder the photo is under"],
  ["{dir}", "the folder the photo is in"],
  ["{relpath}", "its path below the root, with extension"],
  ["{name}", "file name, DSCF0001.RAF"],
  ["{stem}", "file name without extension"],
  ["{ext}", "extension, as written"],
  ["{hash}", "the photo's id (16 digits); {hash:2} its first 2"],
  ["{tier}", "thumb or preview"],
  ["{size}", "the tier's long edge in pixels"],
];

/** A preset's patterns; none for Custom, which has its own. */
export function presetPatterns(preset: Preset): Patterns | null {
  if (preset === "Custom") return null;
  return { thumb: PATTERNS[preset], preview: PATTERNS[preset] };
}

/** `store` switched to `preset`: a named preset brings its patterns, Custom keeps the ones
 * there are (to start from). The size limit stays. */
export function withPreset(store: ImageStore, preset: Preset): ImageStore {
  const patterns = presetPatterns(preset);
  return patterns ? { ...store, preset, ...patterns } : { ...store, preset };
}

/** Whether the images already made would have to move or be made again: the preset or its
 * patterns differ. A changed size limit alone does not. */
export function storeMoved(a: ImageStore, b: ImageStore): boolean {
  return a.preset !== b.preset || a.thumb !== b.thumb || a.preview !== b.preview;
}

/** An example photo for showing what a pattern gives. */
export type Example = { data: string; root: string; relpath: string; hash: string };

export const EXAMPLE: Example = {
  data: "…/work",
  root: "D:/Photos/Family",
  relpath: "2024/Trip/DSCF0001.RAF",
  hash: "9f3c1a2b4d5e6f70",
};

/** Long edges of the two tiers, as the provider makes them. */
const TIER_SIZE = { thumb: 450, preview: 2000 } as const;

/** `pattern` with its tokens replaced by what they stand for in `example`; an unknown token
 * stays as written (the provider's check says what is wrong with it). */
export function renderPattern(pattern: string, tier: "thumb" | "preview", example: Example = EXAMPLE): string {
  const slash = example.relpath.lastIndexOf("/");
  const name = example.relpath.slice(slash + 1);
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const ext = dot > 0 ? name.slice(dot + 1) : "";
  const dir = slash < 0 ? example.root : `${example.root}/${example.relpath.slice(0, slash)}`;
  return pattern.replace(/\{([a-z]+)(?::(\d+))?\}/g, (whole, token: string, n?: string) => {
    switch (token) {
      case "data": return example.data;
      case "root": return example.root;
      case "dir": return dir;
      case "relpath": return example.relpath;
      case "name": return name;
      case "stem": return stem;
      case "ext": return ext;
      case "hash": return n ? example.hash.slice(0, Number(n)) : example.hash;
      case "tier": return tier;
      case "size": return String(TIER_SIZE[tier]);
      default: return whole;
    }
  });
}

/** The excludes as the dialog's text: one glob per line. */
export function excludesToText(excludes: string[]): string {
  return excludes.join("\n");
}

/** The globs in the dialog's text: one per line, blank lines and repeats dropped. */
export function textToExcludes(text: string): string[] {
  const seen = new Set<string>();
  for (const line of text.split(/\r?\n/)) {
    const glob = line.trim();
    if (glob) seen.add(glob);
  }
  return [...seen];
}

/** The central store's limit in GB, for the field; empty for none. */
export function mbToGb(mb: number | null): string {
  if (mb === null) return "";
  return String(Math.round((mb / 1024) * 100) / 100);
}

/** The field's GB as megabytes; null (no limit) for an empty or not positive number, and
 * `undefined` for text that is not a number. */
export function gbToMb(gb: string): number | null | undefined {
  const text = gb.trim().replace(",", ".");
  if (!text) return null;
  const value = Number(text);
  if (!Number.isFinite(value)) return undefined;
  return value > 0 ? Math.max(1, Math.round(value * 1024)) : null;
}

/** "Scanning folders 1 234 / 5 678", or without the total while it is not known. */
export function progressText(p: SourceProgress, locale?: string): string {
  const n = new Intl.NumberFormat(locale);
  const done = n.format(p.done);
  return p.total === null ? `${p.phase} ${done}` : `${p.phase} ${done} / ${n.format(p.total)}`;
}

/** `roots` with `path` added, unless a root has that path already. `newId` makes the 8 hex
 * digits a root is known by; it is asked again until the id is not in use. */
export function addRoot(roots: RootDef[], path: string, newId: () => string): RootDef[] {
  const same = (a: string, b: string) => a.replace(/[\\/]+$/, "") === b.replace(/[\\/]+$/, "");
  if (roots.some((r) => same(r.path, path))) return roots;
  let id = newId();
  while (roots.some((r) => r.id === id)) id = newId();
  return [...roots, { id, path, label: null }];
}

/** "1 root offline", "3 roots offline"; empty for none. */
export function offlineText(roots: string[]): string {
  if (!roots.length) return "";
  return `${roots.length} root${roots.length === 1 ? "" : "s"} offline`;
}
