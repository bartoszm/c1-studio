// How photo facts read in the UI.

import type { Op, Photo } from "./api";

export const COLORS: Record<string, string> = {
  red: "#d9534f", orange: "#e8913a", yellow: "#e8d23a", green: "#5cb85c",
  blue: "#4a90d9", pink: "#e87ab8", purple: "#9b6bd6", none: "transparent",
};

/** A space that keeps a unit with its number when the line wraps. */
const NB = String.fromCharCode(0xa0);

/** `23 mm · f/2 · 1/250 s · ISO 400`, leaving out what is unknown. */
export function exposure(p: Photo): string {
  const e = p.exif;
  return [
    e.focal && `${Math.round(e.focal)}${NB}mm`,
    e.aperture && `f/${+e.aperture.toFixed(1)}`,
    e.shutter && (e.shutter >= 1 ? `${+e.shutter.toFixed(1)}${NB}s` : `1/${Math.round(1 / e.shutter)}${NB}s`),
    e.iso && `ISO${NB}${e.iso}`,
  ].filter(Boolean).join("  ·  ");
}

export function date(ms: number | null): string {
  return ms === null ? "" : new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "medium" });
}

export function bytes(n: number | null): string {
  if (n === null) return "";
  return n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : `${Math.round(n / 1e3)} kB`;
}

/**
 * The folder holding a file, as the folder tree names it: `H:\fotos\a.raf` → `H:/fotos`
 * (crates/c1index/src/schema.rs, `folder_tree`).
 */
export function folderOf(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  parts.pop();
  return (/^[\\/]/.test(path) ? "/" : "") + parts.join("/");
}

export function camera(p: Photo): string {
  const { camera_make: make, camera_model: model } = p.exif;
  // Models often repeat the make ("Canon EOS 6D"); say it once.
  if (make && model && !model.toLowerCase().startsWith(make.toLowerCase().split(" ")[0])) return `${make} ${model}`;
  return model ?? make ?? "";
}

export function stars(n: number | null | undefined): string {
  return n ? "★".repeat(n) : "unrated";
}

/** What a field edit sets, as words: `{ field: "rating", from: "★★", to: "★★★★★" }`. */
export function opText(op: Op): { field: string; from: string; to: string } | null {
  if ("SetRating" in op) {
    const { base, value } = op.SetRating as { base: number | null; value: number | null };
    return { field: "rating", from: stars(base), to: stars(value) };
  }
  if ("SetColor" in op) {
    const { base, value } = op.SetColor as { base: string | null; value: string | null };
    const name = (c: string | null) => (c ? c.toLowerCase() : "no colour");
    return { field: "colour", from: name(base), to: name(value) };
  }
  if ("AddKeywords" in op || "RemoveKeywords" in op) {
    const add = "AddKeywords" in op;
    const { paths } = (add ? op.AddKeywords : op.RemoveKeywords) as { paths: string[][] };
    return { field: "keyword", from: "", to: `${add ? "+" : "−"} ${paths.map((p) => p.join("|")).join(", ")}` };
  }
  return null;
}

/** The value an op sets, for a field edit. */
export function opValue(op: Op): unknown {
  const inner = Object.values(op)[0] as { value?: unknown } | undefined;
  return inner?.value;
}
