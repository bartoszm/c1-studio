// How photo facts read in the UI.

import type { LogEvent, Op, Origin, Photo } from "./api";

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
/** Swatches for the analysis' colour names. */
export const HUES: Record<string, string> = {
  black: "#111", white: "#eee", grey: "#888", brown: "#7a4a25", red: "#d33", orange: "#f08a24",
  yellow: "#e6d23a", green: "#3a9a45", teal: "#1fa0a0", blue: "#3060d0", purple: "#8040c0", pink: "#e060a8",
  "black & white": "linear-gradient(90deg, #000 50%, #fff 50%)",
};

export function opText(op: Op, albumName: (id: string) => string = (id) => id): { field: string; from: string; to: string } | null {
  if ("SetRating" in op) {
    const { base, value } = op.SetRating as { base: number | null; value: number | null };
    return { field: "rating", from: stars(base), to: stars(value) };
  }
  if ("SetColor" in op) {
    const { base, value } = op.SetColor as { base: string | null; value: string | null };
    const name = (c: string | null) => (c ? c.toLowerCase() : "no colour");
    return { field: "colour", from: name(base), to: name(value) };
  }
  if ("SetPlace" in op || "SetIptc" in op) {
    const place = "SetPlace" in op;
    const { base, value } = (place ? op.SetPlace : op.SetIptc) as Record<"base" | "value", Record<string, string | null>>;
    const names = Object.keys(value).filter((k) => (base[k] ?? null) !== (value[k] ?? null));
    const show = (o: Record<string, string | null>) => names.map((k) => o[k] ?? "—").join(", ") || "—";
    const label = names.map((k) => FIELD_NAMES[k] ?? k).join(", ");
    return { field: label || (place ? "location" : "caption"), from: show(base), to: show(value) };
  }
  if ("SetGeo" in op) {
    const { base, value } = op.SetGeo as Record<"base" | "value", { lat: number; lon: number } | null>;
    return { field: "GPS", from: geoText(base), to: geoText(value) };
  }
  if ("AddKeywords" in op || "RemoveKeywords" in op) {
    const add = "AddKeywords" in op;
    const { paths } = (add ? op.AddKeywords : op.RemoveKeywords) as { paths: string[][] };
    return { field: "keyword", from: "", to: `${add ? "+" : "−"} ${paths.map((p) => p.join("|")).join(", ")}` };
  }
  if ("AddToCollection" in op || "RemoveFromCollection" in op) {
    const add = "AddToCollection" in op;
    const { collection } = (add ? op.AddToCollection : op.RemoveFromCollection) as { collection: string };
    return { field: "album", from: "", to: `${add ? "into" : "out of"} ${albumName(collection)}` };
  }
  return null;
}

export const ORIGINS: Record<Origin, string> = { App: "here", Source: "Capture One", Gpx: "GPX", Restore: "backup" };

const count = (n: number, what: string) => `${n} ${what}${n === 1 ? "" : "s"}`;

/** A change log entry in one line: `rating: ★★ → ★★★★★`, `+ street`, `added`. */
export function eventText(e: LogEvent, albumName?: (id: string) => string): string {
  if ("change" in e) {
    const t = opText(e.change.op, albumName);
    if (!t) return "changed";
    return t.from ? `${t.field}: ${t.from} → ${t.to}` : `${t.field} ${t.to}`;
  }
  if ("added" in e) return `${e.added.name} added`;
  if ("removed" in e) return `${e.removed.name} removed`;
  if ("collection" in e) return e.collection.label;
  if ("remap" in e) return e.remap.label;
  if ("undone" in e) return `Undo: ${e.undone.label}`;
  if ("written" in e) {
    const { changes, albums, keywords } = e.written;
    const parts = [
      changes && count(changes, "edit"),
      albums && count(albums, "album change"),
      keywords && count(keywords, "keyword change"),
    ].filter(Boolean);
    return `${parts.join(", ") || "Nothing"} written to Capture One`;
  }
  return "";
}

/** The value an op sets, for a field edit. */
export function opValue(op: Op): unknown {
  const inner = Object.values(op)[0] as { value?: unknown } | undefined;
  return inner?.value;
}

/** How location and caption fields are called on screen. */
export const FIELD_NAMES: Record<string, string> = {
  rating: "rating",
  color: "colour",
  keywords: "keywords",
  keywords_exact: "keywords",
  country: "country",
  country_code: "country code",
  state: "state",
  city: "city",
  location: "location",
  geo: "GPS",
  description: "description",
  title: "title",
  headline: "headline",
  creator: "creator",
  copyright: "copyright",
  job_id: "job identifier",
  instructions: "instructions",
  source: "source",
};

/** `50.061400, 19.936600`, or a dash. */
export function geoText(geo: { lat: number; lon: number } | null): string {
  return geo ? `${+geo.lat.toFixed(6)}, ${+geo.lon.toFixed(6)}` : "—";
}

/** One coordinate: decimal (`-33.8688`, `33.8688S`) or degrees, minutes and seconds
 *  (`50°3'41.0"N`). The letter, if any, must fit the axis. */
function coordinate(text: string, latitude: boolean): number | null {
  const t = text.trim().toUpperCase().replace(/[′’]/g, "'").replace(/[″”]/g, '"');
  const m = t.match(/^([+-]?)(\d+(?:\.\d+)?)\s*(?:°\s*(?:(\d+(?:\.\d+)?)\s*'\s*(?:(\d+(?:\.\d+)?)\s*"?)?)?)?\s*([NSEW]?)$/);
  if (!m) return null;
  const [, sign, d, min, sec, letter] = m;
  if (letter && (latitude ? !"NS".includes(letter) : !"EW".includes(letter))) return null;
  let v = +d + (min ? +min / 60 : 0) + (sec ? +sec / 3600 : 0);
  if (sign === "-" || letter === "S" || letter === "W") v = -v;
  return Number.isFinite(v) ? v : null;
}

/** Typed GPS: `50.0614, 19.9366`, `50.0614 19.9366`, `33.8688S 151.2093E` or
 *  `50°3'41"N 19°56'12"E`. Empty clears (null); a string says what is wrong. */
export function parseGeo(text: string): [number, number] | null | string {
  const t = text.trim();
  if (!t) return null;
  const parts = t.includes(",") ? t.split(",") : t.split(/\s+(?=[+-]?\d)/);
  if (parts.length !== 2) return "Type latitude and longitude, e.g. 50.0614, 19.9366.";
  const lat = coordinate(parts[0], true);
  const lon = coordinate(parts[1], false);
  if (lat === null || lon === null) return "Type latitude and longitude, e.g. 50.0614, 19.9366.";
  if (Math.abs(lat) > 90 || Math.abs(lon) > 180) return "Latitude is -90 to 90, longitude -180 to 180.";
  return [lat, lon];
}
