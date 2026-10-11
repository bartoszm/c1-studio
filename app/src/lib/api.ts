// The Rust side as typed calls. Shapes follow serde's defaults for the Rust types in
// crates/c1index (Filter, QueryOptions, QueryResult) and crates/c1model (Photo).

import { invoke } from "@tauri-apps/api/core";

export type Dim =
  | "rating" | "color" | "year" | "month" | "week" | "camera" | "lens" | "keyword_tree" | "collection" | "folder" | "country" | "city"
  | "place_tree" | "flag";

export type Range = { min?: number; max?: number; min_exclusive?: boolean; max_exclusive?: boolean };
export type Presence<T> = "None" | "Any" | { Values: T };
/** Inclusive Unix milliseconds; either end may be left open. */
export type TimeRange = { from?: number; to?: number };
export type Color = "Red" | "Orange" | "Yellow" | "Green" | "Blue" | "Pink" | "Purple";

/** Only the fields set narrow the result; the rest take Rust's defaults. */
export type Filter = {
  text?: string;
  rating?: Presence<Range>;
  color?: Presence<Color[]>;
  year?: number;
  /** `YYYY-MM`, a local month. */
  month?: string;
  /** `YYYY-MM-Www`: the days of an ISO week inside a month. */
  week?: string;
  /** A place and everything in it: `Poland|Malopolska`, a missing level as `—`. */
  place_under?: string;
  /** Only photos with a country but no ISO country code. */
  no_country_code?: boolean;
  /** Capture time. */
  captured?: TimeRange;
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

export type Status = {
  catalog: string;
  source: string;
  photos: number;
  writes: string;
  sync_error: string | null;
  compatibility: Compatibility;
};

/** How far the app is checked against a source's format, each way. */
export type Support = "Tested" | "Untested" | "Off";
export type Compatibility = { format: string; read: Support; write: Support; note: string | null };

/** What to open: which provider (`capture-one`), and where. An empty provider lets the
 *  first that recognises the location open it. */
export type SourceSpec = { provider: string; location: string };
export type SourceEntry = SourceSpec & { kind: string; name: string; available: boolean };
export type ProviderView = {
  id: string;
  name: string;
  pick: { File: { label: string; extensions: string[] } } | "Folder";
};
export type SourcesView = {
  current: SourceEntry | null;
  compatibility: Compatibility | null;
  error: string | null;
  recent: SourceEntry[];
  providers: ProviderView[];
};

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
    job_id: string | null;
    instructions: string | null;
    source: string | null;
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
export type SmartRule = { match_all: boolean; conditions: Condition[] };
export type Collection = {
  id: string;
  name: string;
  kind: CollectionKind;
  parent: string | null;
  path: string;
  rule: SmartRule | null;
  /** Capture One's own (Recent Imports): never renamed, deleted or given albums here. */
  system: boolean;
};
/** An album change. A new one's id is `new:<uuid>`; send "" and the app makes one. */
export type CollectionChange =
  | { Create: { id: string; name: string; kind: "Album" | "Smart" | "Group"; parent: string | null; rule: SmartRule | null } }
  | { Rename: { id: string; name: string } }
  | { SetRule: { id: string; rule: SmartRule } }
  | { Delete: { id: string } }
  /** Into a group or project; null is the top level. */
  | { Move: { id: string; parent: string | null } };

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
  compatibility: Compatibility;
  apply: PendingEdit[];
  unchanged: PendingEdit[];
  conflicts: PendingEdit[];
  blocked: [PendingEdit, string][];
  /** Keyword tree changes, written first: number and what they do. */
  remaps: [number, string][];
  blocked_remaps: [number, string, string][];
  /** Album changes, written after the keyword tree and before photo edits. */
  collections: [number, string][];
  blocked_collections: [number, string, string][];
};

/** A keyword tree change. Paths join levels with `|`. */
export type RemapRequest =
  | { Rename: { path: string; name: string } }
  | { Join: { from: string[]; into: string } }
  | { Group: { paths: string[]; under: string | null } }
  | { Ungroup: { path: string } };
export type Edit =
  | { Rating: number | null }
  | { Color: Color | null }
  | { AddKeywords: string[] }
  | { RemoveKeywords: string[] }
  /** Location fields by name; "" clears one. */
  | { Place: Partial<Record<PlaceField, string>> }
  | { Iptc: Partial<Record<CaptionField, string>> }
  /** Latitude, longitude; null clears. */
  | { Geo: [number, number] | null }
  /** Into an album or out of it, by collection id. */
  | { AddToAlbum: string }
  | { RemoveFromAlbum: string };
export type PlaceField = "country" | "country_code" | "state" | "city" | "location";
/** IPTC text fields. `title` is IPTC's Object Name: a short identifier, not a caption. */
export type CaptionField =
  | "description" | "title" | "headline" | "creator" | "copyright" | "job_id" | "instructions" | "source";
/** What can be copied from one photo to others. `keywords` adds them; `keywords_exact`
 *  makes the targets' keywords the same. */
export type CopyField = "rating" | "color" | "keywords" | "keywords_exact" | PlaceField | "geo" | CaptionField;

export const status = () => invoke<Status>("status");
export const sources = () => invoke<SourcesView>("sources");
/** The system dialog for a provider's kind of source; null if cancelled. */
export const pickSource = (provider: string) => invoke<string | null>("pick_source", { provider });
/** Opens another source; the window should reload after. */
export const openSource = (spec: SourceSpec) => invoke<void>("open_source", { spec });
export const forgetSource = (spec: SourceSpec) => invoke<void>("forget_source", { spec });
export const query = (filter: Filter, options: QueryOptions) => invoke<QueryResult>("query", { filter, options });
export const smartCounts = (filter: Filter) => invoke<[string, number][]>("smart_counts", { filter });
export const photo = (row: number) => invoke<Photo | null>("photo", { row });
export const collections = () => invoke<Collection[]>("collections");
export const sync = () => invoke("sync");
/** Returns the ops that undo it. */
export const edit = (rows: number[], edit: Edit) => invoke<[number, Op][]>("edit", { rows, edit });
/** Pastes fields copied from `from` onto the photos at `rows`; `fill` only fills the
 *  fields they leave empty (keywords are added). Returns the ops that undo it. */
export const paste = (rows: number[], from: Photo, fields: CopyField[], fill = false) =>
  invoke<[number, Op][]>("paste", { rows, from, fields, fill });
/** Photos in view with a country but no ISO code, by country. */
export const missingCountryCodes = (filter: Filter) =>
  invoke<[string, number[]][]>("missing_country_codes", { filter });
/** Sets ISO codes on photos as one edit; returns the ops that undo it. */
export const setCountryCodes = (items: [number[], string][]) => invoke<[number, Op][]>("set_country_codes", { items });
/** Renames or joins a place: every photo at or under `from` moves to `to` (same depth);
 *  `code` sets the ISO code with a new country. Returns the ops that undo it. */
export const movePlace = (from: string, to: string, code: string | null) =>
  invoke<[number, Op][]>("move_place", { from, to, code });
export const editOps = (changes: [number, Op][]) => invoke<[number, Op][]>("edit_ops", { changes });
/** Every keyword path in the catalog with its use count. */
export const keywords = () => invoke<[string, number][]>("keywords");
/** Queues a keyword tree change; returns its number, for undo. */
export const remapKeywords = (request: RemapRequest) => invoke<number>("remap_keywords", { request });
export const undoRemap = (seq: number) => invoke<void>("undo_remap", { seq });
export const remaps = () => invoke<[number, string][]>("remaps");
/** The keywords on the photos at `rows`, with how many of them carry each. */
export const keywordCounts = (rows: number[]) => invoke<[string, number][]>("keyword_counts", { rows });
/** The albums the photos at `rows` are in, with how many of them each holds. */
export const albumCounts = (rows: number[]) => invoke<[string, number][]>("album_counts", { rows });
/** Queues an album change; returns its number (for undo) and the change as queued (for redo). */
export const changeCollection = (change: CollectionChange) =>
  invoke<[number, CollectionChange]>("change_collection", { change });
export const undoCollection = (seq: number) => invoke<void>("undo_collection", { seq });
/** A copy of the catalog kept before writing to it. */
export type BackupInfo = { id: string; path: string; created_ms: number; size: number; note: string | null };
/** The backups, newest first, and whether the catalog can be written now (a restore needs it). */
export const backups = () => invoke<{ list: BackupInfo[]; status: WriteStatus }>("backups");
/** Puts a backup back as the catalog; returns the backup of the catalog it replaced. */
export const restoreBackup = (id: string) => invoke<BackupInfo>("restore_backup", { id });
/** Queued album changes: number, what it does, the collection it is about. */
export const collectionChanges = () => invoke<[number, string, string][]>("collection_changes");
export const keywordUsers = (paths: string[]) => invoke<number>("keyword_users", { paths });
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
