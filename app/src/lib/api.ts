// The Rust side as typed calls. Shapes follow serde's defaults for the Rust types in
// crates/c1index (Filter, QueryOptions, QueryResult) and crates/c1model (Photo).

import { invoke } from "@tauri-apps/api/core";

export type Dim =
  | "rating" | "color" | "year" | "camera" | "lens" | "keyword_tree" | "collection" | "folder" | "country" | "city";

export type Range = { min?: number; max?: number; min_exclusive?: boolean; max_exclusive?: boolean };
export type Presence<T> = "None" | "Any" | { Values: T };
export type Color = "Red" | "Orange" | "Yellow" | "Green" | "Blue" | "Pink" | "Purple";

/** Only the fields set narrow the result; the rest take Rust's defaults. */
export type Filter = {
  text?: string;
  rating?: Presence<Range>;
  color?: Presence<Color[]>;
  year?: number;
  camera?: string;
  country?: string;
  keywords_under?: string[];
  collections?: string[];
  /** Folder subtree, `H:/fotos/2026`. */
  folder?: string;
};

export type Sort = "DateDesc" | "Date" | "RatingDesc" | "ImportedDesc" | "Filename";

export type QueryOptions = {
  sort?: Sort;
  limit?: number;
  offset?: number;
  facets?: Dim[];
  /** Count each group as if its own value weren't picked, so the alternatives stay visible. */
  facets_ignore_own?: boolean;
};

export type QueryResult = {
  total: number;
  rows: number[];
  facets: Partial<Record<Dim, [string, number][]>>;
  micros: number;
};

export type Status = { catalog: string; source: string; photos: number; writes: string };

export type GeoOrigin = "Exif" | "Manual" | "Gpx" | "Geocoded";

export type Photo = {
  id: { source: string; ext: string };
  asset: string;
  variant_index: number;
  file: { name: string; ext: string; path: string | null; size: number | null };
  captured_at: number | null;
  imported_at: number | null;
  exif: {
    camera_make: string | null;
    camera_model: string | null;
    lens: string | null;
    iso: number | null;
    aperture: number | null;
    shutter: number | null;
    focal: number | null;
    width: number | null;
    height: number | null;
  };
  rating: number | null;
  color: Color | null;
  keywords: string[][];
  iptc: {
    title: string | null;
    headline: string | null;
    description: string | null;
    creator: string | null;
    copyright: string | null;
  };
  place: {
    city: string | null;
    state: string | null;
    country: string | null;
    country_code: string | null;
    location: string | null;
  };
  geo: { lat: number; lon: number; alt: number | null; origin: GeoOrigin } | null;
  collections: string[];
  edit: { edited: boolean; styles: string[]; local_layers: number; retouch_layers: number };
  trashed: boolean;
};

export type CollectionKind = "Album" | "Smart" | "Group" | "Project" | "Folder" | "Other";
export type Condition =
  | { KeywordContains: string }
  | { Rating: { min: number; max: number } }
  | { Year: number }
  | { Unsupported: string };
export type Collection = {
  id: string;
  name: string;
  kind: CollectionKind;
  parent: string | null;
  path: string;
  rule: { match_all: boolean; conditions: Condition[] } | null;
};

/** An edit as Rust's `c1model::Op` serialises it; only the field edits are sent from here. */
export type Op =
  | { SetRating: { base: number | null; value: number | null } }
  | { SetColor: { base: Color | null; value: Color | null } }
  | Record<string, unknown>;
export type Change = { photo: { source: string; ext: string }; op: Op };
export type PendingEdit = {
  key: string;
  row: number | null;
  name: string | null;
  change: Change;
  at: number;
  /** Set when Capture One changed the same field meanwhile: base → its value. */
  theirs: Op | null;
};
export type WriteStatus = "Ready" | "ReadOnly" | { Locked: { by: string } };
export type WritePlan = {
  status: WriteStatus;
  apply: PendingEdit[];
  unchanged: PendingEdit[];
  conflicts: PendingEdit[];
  blocked: [PendingEdit, string][];
};
export type Edit = { Rating: number | null } | { Color: Color | null };

export const status = () => invoke<Status>("status");
export const query = (filter: Filter, options: QueryOptions) => invoke<QueryResult>("query", { filter, options });
export const smartCounts = (filter: Filter) => invoke<[string, number][]>("smart_counts", { filter });
export const photo = (row: number) => invoke<Photo | null>("photo", { row });
export const collections = () => invoke<Collection[]>("collections");
export const sync = () => invoke("sync");
/** Returns the ops that undo it. */
export const edit = (rows: number[], edit: Edit) => invoke<[number, Op][]>("edit", { rows, edit });
export const editOps = (changes: [number, Op][]) => invoke<[number, Op][]>("edit_ops", { changes });
export const pending = () => invoke<PendingEdit[]>("pending");
export const discard = (keys: string[]) => invoke<void>("discard", { keys });
export const resolve = (keys: string[], keepMine: boolean) => invoke<void>("resolve", { keys, keepMine });
export const writePlan = () => invoke<WritePlan>("write_plan");
export type AutosaveState =
  | { state: "off" | "idle" | "pending" }
  | { state: "waiting"; by: string }
  | { state: "attention"; conflicts: number; blocked: number }
  | { state: "saved"; written: number; at: number }
  | { state: "failed"; error: string };
/** Whether autosave is on, and what it last reported. */
export const autosave = () => invoke<[boolean, AutosaveState]>("autosave");
export const setAutosave = (on: boolean) => invoke<void>("set_autosave", { on });
export const writeApply = () => invoke<{ written: number; backup: string | null }>("write_apply");

// WebView2 on Windows reaches custom protocols as http://<scheme>.localhost.
const base = navigator.userAgent.includes("Windows") ? "http://c1img.localhost" : "c1img://localhost";
export const thumbUrl = (row: number) => `${base}/thumb/${row}`;
export const previewUrl = (row: number) => `${base}/preview/${row}`;
