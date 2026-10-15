// The Rust side as typed calls. Shapes follow serde's defaults for the Rust types in
// crates/c1index (Filter, QueryOptions, QueryResult) and crates/c1model (Photo).

import { invoke } from "@tauri-apps/api/core";

export type Dim =
  | "rating" | "color" | "year" | "month" | "week" | "camera" | "lens" | "keyword_tree" | "collection" | "folder" | "country" | "city"
  | "place_tree" | "flag" | "shape" | "hue" | "light" | "focus" | "group";

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
  /** Photos with a position (GPS), or without one. */
  gps?: boolean;
  /** Photos with a place (any of country, state, city, location), or without one. */
  placed?: boolean;
  /** Photos without a position of their own (none, or one given from their place's name). */
  approx?: boolean;
  /** Capture time. */
  captured?: TimeRange;
  /** A trip's span: kept apart from `captured`, so days picked inside a trip narrow it. */
  trip?: TimeRange;
  camera?: string;
  country?: string;
  keywords_under?: string[];
  collections?: string[];
  /** Folder subtree, `H:/fotos/2026`. */
  folder?: string;
  /** From the analysis, each any of its values. */
  shape?: string[];
  hues?: string[];
  light?: string[];
  focus?: string[];
  /** One duplicate group or series: `dup:<row>`, `series:<row>`. */
  group?: string;
  duplicates?: boolean;
  series?: boolean;
  /** Measured by the analysis, or not yet. */
  analysed?: boolean;
  /** Only photos with a position inside this area (the map's view). */
  within?: BBox;
};
export type BBox = { min_lat: number; max_lat: number; min_lon: number; max_lon: number };
/** One marker on the map: a photo (count 1) or a cluster; `best` is the photo shown for it. */
/** `rows`: all its photos, when there are several all at one spot. */
export type Cluster = { lat: number; lon: number; count: number; best: number; bounds: BBox; rows?: number[] };
/** How precisely a position from a place's name stands for the photo. */
export type Level = "Country" | "State" | "City" | "Location";
/** A marker for photos placed only by their place's name: one place (`place`) or several. */
export type PlaceCluster = {
  lat: number;
  lon: number;
  count: number;
  best: number;
  bounds: BBox;
  level: Level;
  /** When all its places sit at one point: what they share (`Poland|Malopolska|Kraków`). */
  place: string | null;
  /** Those places, then. */
  paths?: string[];
  places: number;
};
export type MapResult = {
  located: number;
  in_view: number;
  clusters: Cluster[];
  /** Photos placed by their place's name, shown while zoomed out about as far as it reaches. */
  approx: PlaceCluster[];
  approx_located: number;
  approx_in_view: number;
  /** In view but not shown: the map is zoomed in closer than their place. */
  approx_hidden: number;
  /** Photos with a place and no position of their own. */
  place_only: number;
  /** Of them, those whose place no position is known for (yet). */
  unresolved: number;
  /** Photos with neither a position nor a place. */
  unplaced: number;
};

/** What the analysis found for a photo, with its classes and groups (id, size). */
export type LookView = {
  analysis: {
    width: number;
    height: number;
    palette: { rgb: [number, number, number]; share: number }[];
    chroma: number;
    brightness: number;
    contrast: number;
    highlights: number;
    shadows: number;
    sharpness: number;
  };
  shape: string;
  hues: string[];
  light: string[];
  focus: string;
  duplicates: [string, number] | null;
  series: [string, number] | null;
};
export type AnalysisState = { running: boolean; done: number; total: number; failed: number };
/** Photos without a thumbnail (once counted), and how far making theirs has got. */
export type ThumbsState = {
  running: boolean;
  done: number;
  total: number;
  made: number;
  failed: number;
  missing: number | null;
  /** Why the last run could not make some, most common first, with how many each. */
  reasons: [string, number][];
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
/** Who made a logged change: this app, Capture One (seen by a sync), a GPX track, or a
 * backup restored from this app. */
export type Origin = "App" | "Source" | "Gpx" | "Restore";
type PhotoRef = { photo: { source: string; ext: string }; name: string };
/** One entry of the change log (`c1index::Event`). */
export type LogEvent = { at: number; origin: Origin } & (
  | { change: Change }
  | { added: PhotoRef }
  | { removed: PhotoRef }
  | { collection: { change: CollectionChange; label: string } }
  | { remap: { label: string } }
  | { written: { photos: number; changes: number; albums: number; keywords: number } }
  | { undone: { label: string } }
);
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
/** The photos the filter selects that have a position, clustered for the map's view. */
export const map = (filter: Filter, view: { bounds: BBox; zoom: number }) => invoke<MapResult>("map", { filter, view });
export type Approx = { lat: number; lon: number; level: Level };
/** A place no position is known for at the level it names. */
export type Unresolved = { path: string; place: Photo["place"]; photos: number; have: Level | null; want: Level };
/** Places the gazetteer lacks, to look up online. */
export const unresolvedPlaces = (filter: Filter) => invoke<Unresolved[]>("unresolved_places", { filter });
/** What online lookups found for places (null: nothing; not asked again). */
export const setPlacePoints = (items: { path: string; found: Approx | null }[]) =>
  invoke<void>("set_place_points", { items });
/** Gives the photos at each place (of those `filter` selects, with no position) the
 *  place's position, kept marked as approximate. Returns the ops that undo it. */
export const promotePlaces = (filter: Filter, paths: string[]) =>
  invoke<[number, Op][]>("promote_places", { filter, paths });
/** Gives the photos at `rows` with no position their place's, kept marked as approximate. */
export const promoteRows = (rows: number[]) => invoke<[number, Op][]>("promote_rows", { rows });
export type FillGroup = { place: Photo["place"]; rows: number[] };
/** For the photos at `rows` with a position, the place to fill their empty fields with. */
export const fillPlan = (rows: number[]) => invoke<FillGroup[]>("fill_plan", { rows });
/** Fills the empty place fields of the photos at `rows` from their positions. */
export const fillPlaces = (rows: number[]) => invoke<[number, Op][]>("fill_places", { rows });
export type NeighbourSettings = { max_gap_minutes: number; positions: boolean; places: boolean };
export type NeighbourPlan = {
  candidates: number;
  positions: number;
  places: number;
  sample: [number, number, number][];
  place_counts: [string, number][];
};
export const neighbourPlan = (filter: Filter, settings: NeighbourSettings) =>
  invoke<NeighbourPlan>("neighbour_plan", { filter, settings });
/** Places the photos as the plan showed; returns the ops that undo it. */
export const neighbourApply = (filter: Filter, settings: NeighbourSettings) =>
  invoke<[number, Op][]>("neighbour_apply", { filter, settings });
export type GpxSettings = { path: string; offset_minutes: number; max_gap_minutes: number; overwrite: boolean };
export type GpxPlan = {
  fixes: number;
  from: number;
  to: number;
  candidates: number;
  matched: number;
  sample: [number, number, number][];
};
/** The system dialog for a GPX file; null if cancelled. */
export const pickGpx = () => invoke<string | null>("pick_gpx");
export const gpxPlan = (filter: Filter, settings: GpxSettings) => invoke<GpxPlan>("gpx_plan", { filter, settings });
/** Places the photos the track covers; returns the ops that undo it. */
export const gpxApply = (filter: Filter, settings: GpxSettings) => invoke<[number, Op][]>("gpx_apply", { filter, settings });
/** A duplicate group or series: its size, how many of it the filter picks, the first few of those. */
export type Group = { id: string; size: number; matched: number; rows: number[] };
export type GroupPage = { total: number; groups: Group[] };
/** A page of the duplicate groups (or series) among the photos the filter selects, biggest
 *  first, with how many there are in all. */
/** Groups the Analysis tab shows at a time. */
export const GROUP_PAGE = 60;
export const groups = (filter: Filter, series: boolean, offset = 0, limit = GROUP_PAGE) =>
  invoke<GroupPage>("groups", { filter, series, offset, limit });
export const smartCounts = (filter: Filter) => invoke<[string, number][]>("smart_counts", { filter });

/** Where home is: never part of a trip. No city: the whole country, so only trips abroad. */
export type Home = { country: string | null; city: string | null };
export type TripSettings = {
  home: Home | null;
  gap_hours: number;
  bridge_days: number;
  min_photos: number;
  suggest_days: number;
  suggest_photos: number;
};
/** found: photos placed away from home; suggested: a busy stretch with no place; kept:
 *  touched by the user; dismissed: not a trip. */
export type TripKind = "found" | "suggested" | "kept" | "dismissed";
export type Trip = {
  id: number;
  /** First and last photo, Unix ms. */
  start: number;
  end: number;
  name: string | null;
  kind: TripKind;
  /** Its main places, for when it has no name. */
  label: string;
  /** Photos in its span, and of them the ones the filter selects. */
  photos: number;
  count: number;
  /** The photo shown for it. */
  cover: number | null;
};
export type TripList = { settings: TripSettings; home_guess: Home | null; trips: Trip[] };
export type TripEdit =
  | { op: "rename"; id: number; name: string | null }
  | { op: "keep"; id: number }
  | { op: "dismiss"; id: number }
  | { op: "forget"; id: number }
  | { op: "merge"; ids: number[] }
  | { op: "split"; id: number; at: number }
  | { op: "create"; start: number; end: number; name: string | null };
/** Every trip, counted under `filter` (its own trip aside). */
export const trips = (filter: Filter) => invoke<TripList>("trips", { filter });
/** Photos per day under `filter`: [local midnight in ms, count], in date order. */
export const timeline = (filter: Filter) => invoke<[number, number][]>("timeline", { filter });
/** A photo on a route: when and where it was taken. */
export type RoutePoint = { at: number; lat: number; lon: number; row: number };
/** The photos `filter` selects that have a position, in the order they were taken. */
export const route = (filter: Filter) => invoke<RoutePoint[]>("route", { filter });
export const setTripSettings = (settings: TripSettings) => invoke<void>("set_trip_settings", { settings });
/** Returns the trip to select afterwards, if any. */
export const editTrips = (edit: TripEdit) => invoke<number | null>("edit_trips", { edit });
export const photo = (row: number) => invoke<Photo | null>("photo", { row });
/** What the change log holds about a photo, newest first. */
/** `albums`: what the albums the entries name were called when last seen, by id. */
export const photoHistory = (row: number) =>
  invoke<{ events: LogEvent[]; albums: Record<string, string> }>("photo_history", { row });
/** The newest entries of the change log. */
export const changeLog = (limit: number) => invoke<LogEvent[]>("change_log", { limit });
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
/** The places of the photos at `rows`, each with the photos that have it, most used first;
 *  those with no location fields share the empty place. */
export const placeCounts = (rows: number[]) => invoke<[Photo["place"], number[]][]>("place_counts", { rows });
/** The albums the photos at `rows` are in, with how many of them each holds. */
export const albumCounts = (rows: number[]) => invoke<[string, number][]>("album_counts", { rows });
/** Queues an album change; returns its number (for undo) and the change as queued (for redo). */
export const changeCollection = (change: CollectionChange) =>
  invoke<[number, CollectionChange]>("change_collection", { change });
export const undoCollection = (seq: number) => invoke<void>("undo_collection", { seq });
export const look = (row: number) => invoke<LookView | null>("look", { row });
export const analysisState = () => invoke<AnalysisState>("analysis_state");
export const thumbsState = () => invoke<ThumbsState>("thumbs_state");
/** Makes a thumbnail for every photo without one, in the background ("thumbs" events). */
export const fillThumbnails = () => invoke<void>("fill_thumbnails");
/** Opens the events log (background work and what failed) in the system's text viewer. */
export const openLog = () => invoke<void>("open_log");

/** Thumbnails that failed to load try again: one may have been made for them since. */
let retries = 0;
export function retryBrokenImages() {
  retries++;
  for (const img of document.querySelectorAll<HTMLImageElement>("img[data-broken]")) {
    delete img.dataset.broken;
    img.src = img.src.replace(/[?#].*$/, "") + `?r=${retries}`;
  }
}
/** Marks images that fail to load, so they hide and can be tried again. */
export function watchBrokenImages(): () => void {
  const failed = (e: Event) => {
    if (e.target instanceof HTMLImageElement && e.target.src.startsWith(base)) e.target.dataset.broken = "";
  };
  const loaded = (e: Event) => {
    if (e.target instanceof HTMLImageElement) delete e.target.dataset.broken;
  };
  document.addEventListener("error", failed, true);
  document.addEventListener("load", loaded, true);
  return () => {
    document.removeEventListener("error", failed, true);
    document.removeEventListener("load", loaded, true);
  };
}
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
