// The Rust side as typed calls. Shapes follow serde's defaults for the Rust types in
// crates/c1index (Filter, QueryOptions, QueryResult) and crates/c1model (Photo).

import { invoke } from "@tauri-apps/api/core";
import type { LeftOut } from "./keywordChoice";

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
  /** Photos with a location (the place's finest level), or without one. */
  located?: boolean;
  /** Photos without a position of their own (none, or one given from their place's name). */
  approx?: boolean;
  /** Capture time. */
  captured?: TimeRange;
  /** A trip's span: kept apart from `captured`, so days picked inside a trip narrow it. */
  trip?: TimeRange;
  camera?: string;
  /** Exact camera models: any of them / none of them. */
  cameras?: string[];
  cameras_not?: string[];
  /** None of these colour labels. */
  colors_not?: Color[];
  country?: string;
  keywords_under?: string[];
  /** None of these keywords, nor anything beneath them. */
  keywords_under_not?: string[];
  /** Photos with no keyword at all. */
  unkeyworded?: boolean;
  collections?: string[];
  not_in_collections?: string[];
  /** In no album (`true`), or in at least one (`false`). */
  uncollected?: boolean;
  /** Each also holds / at least one of these holds. */
  all_of?: Filter[];
  any_of?: Filter[];
  /** The photos a part of the change log is about (the History tab). */
  history?: HistoryPick;
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
  /** Photos the user hid: left out unless "Include" (or "Only"). */
  hidden?: "Exclude" | "Include" | "Only";
  /** Photos in the app's trash: left out of every view but the trash's ("Only"). */
  bin?: "Exclude" | "Include" | "Only";
  /** Photos in the source's own trash (Capture One's): left out unless asked for. */
  trashed?: "Exclude" | "Include" | "Only";
  /** One photo per duplicate group or series (its sharpest), unless one group is picked. */
  stacked?: boolean;
  /** Only photos whose original cannot be reached now, or only those that can. */
  offline?: boolean;
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
    /** Per name in `HUE_NAMES`: share of the pixels, biggest patch, mean chroma. */
    hue_stats: { share: number; patch: number; chroma: number }[];
  };
  /** The lines the classes were drawn with. */
  lines: ClassSettings;
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

export type Sort =
  | "DateDesc" | "Date" | "RatingDesc" | "Rating" | "ImportedDesc" | "Imported" | "Filename" | "FilenameDesc"
  | "SharpnessDesc" | "Sharpness" | "BrightnessDesc" | "Brightness"
  | "ColorLabel" | "ColorLabelRev" | "Match" | "MatchWorst";

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
  /** Positions in `rows` of the photos whose file is offline. */
  offline: number[];
  facets: Partial<Record<Dim, [string, number][]>>;
  micros: number;
};

/** How the app names the open source in its text. */
export type Wording = {
  /** As a label: "Capture One", "Photo folders". */
  name: string;
  /** In a sentence: "Write to Capture One", "write to the photo folders". */
  word: string;
  /** What holds the photos, after "the": "catalog", "library". */
  noun: string;
  /** What a backup holds, after "a copy of the": "catalog", "edits and albums kept for the folders". */
  backup: string;
  /** The program that keeps the source open and has to be closed before writing. */
  owner: string | null;
};

export type Status = {
  catalog: string;
  source: string;
  wording: Wording;
  photos: number;
  writes: WriteStatus;
  sync_error: string | null;
  compatibility: Compatibility;
  /** The provider's id: `capture-one`, `folders`. */
  provider: string;
  /** The source can be asked to look at its files again. */
  can_rescan: boolean;
  /** Roots of a folder source that could not be reached at the last scan. */
  offline_roots: string[];
  /** How far a scan has got, while one runs. */
  progress: SourceProgress | null;
  /** What emptying the trash does at the source. */
  removes: Removal;
  /** A position belongs to the image, so moving one variant moves the others (Capture One). */
  geo_per_asset: boolean;
  /** Photos in the app's trash. */
  binned: number;
};

/** What deleting a photo for good does at its source: nothing (it cannot), its files go to
 * the recycle bin, or the source marks it deleted in its own trash (Capture One). */
export type Removal = "None" | "Files" | "MarksTrashed";

/** What emptying the trash did; `failed` holds the photos left in it, by name, with why. */
export type EmptyReport = { removed: number; failed: [string, string][]; backup: string | null; warnings: string[] };

/** How far a long piece of work in the source has got (`source-progress` events). */
export type SourceProgress = { phase: string; done: number; total: number | null; note: string | null };

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
/** A kind of source, and whether the app offers it (`%APPDATA%\c1studio\providers.json`). */
export type KindView = { id: string; name: string; offered: boolean };
export type SourcesView = {
  current: SourceEntry | null;
  compatibility: Compatibility | null;
  error: string | null;
  recent: SourceEntry[];
  /** The kinds offered, to open. */
  providers: ProviderView[];
  /** Every kind there is, to turn on or off. */
  kinds: KindView[];
};

export type GeoOrigin = "Exif" | "Manual" | "Gpx" | "Geocoded";

export type Photo = {
  id: { source: string; ext: string };
  asset: string;
  variant_index: number;
  file: { name: string; ext: string; path: string | null; size: number | null; offline?: boolean };
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
  /** Hidden by the user in C1 Studio (not a Capture One field). */
  hidden?: boolean;
  /** In C1 Studio's trash (not a Capture One field). */
  binned?: boolean;
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
/** Who made a logged change: this app, Capture One (seen by a sync), a GPX track, a
 * backup restored from this app, or a vision model's suggestion accepted here. */
export type Origin = "App" | "Source" | "Gpx" | "Restore" | "Ai";
type PhotoRef = { photo: { source: string; ext: string }; name: string };
/** One entry of the change log (`c1index::Event`). */
export type LogEvent = {
  /** Its place in the log, when the read knew it. */
  seq?: number;
  at: number;
  origin: Origin;
  /** The entry kept what it replaced (a removed photo, an album, rewritten keywords). */
  has_before?: boolean;
} & (
  | { change: Change }
  | { added: PhotoRef }
  | { removed: PhotoRef }
  | { deleted: PhotoRef }
  | { collection: { change: CollectionChange; label: string } }
  | { remap: { label: string } }
  | { written: { photos: number; changes: number; albums: number; keywords: number } }
  | { undone: { label: string } }
);
/** What sort of entry the log holds (`c1index::LogKind`). */
export type LogKind =
  | "rating" | "color" | "keywords" | "iptc" | "place" | "geo" | "albums"
  | "added" | "removed" | "deleted" | "collection" | "remap" | "written" | "undone";
/** Which entries to list (`c1index::LogQuery`). Times are inclusive milliseconds. */
export type LogQuery = { before?: number; from?: number; to?: number; origins?: Origin[]; kinds?: LogKind[]; limit?: number };
/** Consecutive entries of one batch: the same time, origin and kind. */
export type LogGroup = {
  first: number;
  last: number;
  at: number;
  origin: Origin;
  kind: LogKind;
  entries: number;
  photos: number;
  /** The newest entry, for wording a group of one. */
  sample: LogEvent;
  /** The photo's row, when the group is about one that is still in the library. */
  row: number | null;
};
export type LogPage = { groups: LogGroup[]; albums: Record<string, string>; next: number | null };
export type LogDays = { start: number | null; days: [number, number][] };
/** What a pick of "changed between two dates" is about (`c1index::NetWhat`). */
export type NetWhat =
  | "Any"
  | "Added"
  | { Field: LogKind }
  | { KeywordAdded: string }
  | { KeywordRemoved: string }
  | { AlbumIn: string }
  | { AlbumOut: string };
/** The photos a part of the change log is about: the filter's `history`. `to` of null is now. */
export type HistoryPick =
  | { Changed: { from: number; to: number | null; what: NetWhat } }
  | { Entries: { first: number; last: number; kind: LogKind | null; origin: Origin | null } };
export type Compare = {
  from: number | null;
  to: number | null;
  entries: number;
  /** Photos that ended up different in any way, or added. */
  changed: number;
  by_origin: Partial<Record<Origin, number>>;
  added: number;
  removed: number;
  deleted: number;
  /** Photos whose field ended up different, by field. */
  fields: [LogKind, number][];
  keywords_added: [string, number][];
  keywords_removed: [string, number][];
  albums: { id: string; name: string | null; photos_in: number; photos_out: number }[];
  album_changes: LogEvent[];
  keyword_changes: LogEvent[];
  /** Keyword tree changes in range that did not keep their photos: the keyword counts fall short by what they did. */
  keywords_unkept: number;
  written: number;
  /** When the log starts: nothing before is known. */
  log_start: number | null;
  /** Entries among them logged while History was off: they kept nothing of what they
   *  replaced, so keyword tree changes and album deletes among them are not counted by photo. */
  logged_off: number;
};
/** A stretch of the log written while History was off; `to` is null while it still is. */
export type HistoryOff = { from: number; to: number | null; entries: number };
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
/** How far opening a source has got; null once it is ready to use. Also sent as the
 *  `startup-progress` event. */
export type Startup = {
  step: "opening" | "reading" | "indexing" | "saving";
  done: number;
  total: number;
  /** While reading: how far the source has got by its own count (a folder scan). */
  source: SourceProgress | null;
  /** What holds the photos ("catalog", "library"), once the source is open. */
  noun: string | null;
};
export const startupProgress = () => invoke<Startup | null>("startup_progress");
export const sources = () => invoke<SourcesView>("sources");
/** The system dialog for a provider's kind of source; null if cancelled. */
export const pickSource = (provider: string) => invoke<string | null>("pick_source", { provider });
/** Opens another source; the window should reload after. */
export const openSource = (spec: SourceSpec) => invoke<void>("open_source", { spec });
/** The system dialog for a folder; null if cancelled. */
export const pickFolder = () => invoke<string | null>("pick_folder");
/** Asks the source to look at its files again, and reads it in the background. */
export const rescan = () => invoke<void>("rescan");

/** How a folder source keeps its generated images (`source_folders::Preset`). */
export type Preset = "Central" | "PerRoot" | "Beside" | "Custom";
export type ImageStore = { preset: Preset; thumb: string; preview: string; central_limit_mb: number | null };
export type RootDef = { id: string; path: string; label: string | null };
/** `Stem`: `DSCF0001.xmp`; `Name`: `DSCF0001.RAF.xmp`. */
export type SidecarNaming = "Stem" | "Name";
/** A folder source's definition file (`source_folders::Definition`). */
export type FoldersDefinition = {
  version: number;
  id: string;
  name: string;
  roots: RootDef[];
  excludes: string[];
  images: ImageStore;
  sidecar: SidecarNaming;
  write_xmp: boolean;
  modify_originals: boolean;
};
export type FoldersView = { location: string | null; definition: FoldersDefinition; is_open: boolean };
/** What becomes of the images already made when the image store changes. */
export type Migrate = "Move" | "Regenerate";
/** A definition for a new folder source, not saved yet. */
export const foldersNew = (name: string, roots: string[]) => invoke<FoldersView>("folders_new", { name, roots });
export const foldersLoad = (location: string) => invoke<FoldersView>("folders_load", { location });
/** Writes the definition (a new one to the app's folder) and returns where. The open source
 * reads it again and walks its roots; another is for `openSource`. */
export const foldersSave = (def: FoldersDefinition, location: string | null, migrate: Migrate | null) =>
  invoke<string>("folders_save", { def, location, migrate });
/** Why the image store would not do, or null. */
export const foldersCheckStore = (store: ImageStore) => invoke<string | null>("folders_check_store", { store });
export const forgetSource = (spec: SourceSpec) => invoke<void>("forget_source", { spec });
/** Offers these kinds of source from now on (at least one); the source open stays open. */
export const setProviders = (ids: string[]) => invoke<void>("set_providers", { ids });
export const query = (filter: Filter, options: QueryOptions) => invoke<QueryResult>("query", { filter, options });
/** Where a photo is among those the filter selects, in that order (null: not among them). */
export const position = (filter: Filter, options: { sort: Sort }, row: number) =>
  invoke<number | null>("position", { filter, options, row });
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
export type NeighbourSettings = {
  positions: boolean;
  position_gap_minutes: number;
  places: boolean;
  place_gap_minutes: number;
};
export type NeighbourPlan = {
  /** Photos with no position, or only their place's (≈). */
  unpositioned: number;
  positions: number;
  /** Photos with no location. */
  unlocated: number;
  places: number;
  sample: [number, number, number][];
  /** From (empty: no place), to, how many photos. */
  place_counts: [string, string, number][];
};
/** Positions from the GPS photos taken around photos without one of their own; places from
 *  a neighbour's that says the same and more. Nothing is written. */
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
/** Groups the Analysis tab shows at a time. */
export const GROUP_PAGE = 60;
/** A page of the duplicate groups (or series) among the photos the filter selects, biggest
 *  first, with how many there are in all. */
export type GroupSort = "Matched" | "Size" | "Newest" | "Oldest" | "Sharpest";
export const groups = (
  filter: Filter,
  series: boolean,
  offset = 0,
  limit = GROUP_PAGE,
  sort: GroupSort = "Matched",
  reverse = false,
) => invoke<GroupPage>("groups", { filter, series, sort, reverse, offset, limit });
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
/** Hides photos from every view (or shows them again); returns the rows whose mark changed. */
export const setHidden = (rows: number[], hidden: boolean) => invoke<number[]>("set_hidden", { rows, hidden });
/** Of the photos the filter selects, those that are hidden. */
export const hiddenRows = (filter: Filter) => invoke<number[]>("hidden_rows", { filter });
/** Of these photos, the ones that are hidden (whether or not hidden photos are shown). */
export const hiddenOf = (rows: number[]) => invoke<number[]>("hidden_of", { rows });
/** Moves photos to the app's trash, or restores them; returns the rows whose mark changed. */
export const setBinned = (rows: number[], binned: boolean) => invoke<number[]>("set_binned", { rows, binned });
/** Deletes the photos in the trash for good (only those at `rows`, if given). */
export const emptyBin = (rows?: number[]) => invoke<EmptyReport>("empty_bin", { rows: rows ?? null });
/** What the change log holds about a photo, newest first. */
/** `albums`: what the albums the entries name were called when last seen, by id. */
/** `history_off`: when History was off, oldest first. */
export const photoHistory = (row: number) =>
  invoke<{ events: LogEvent[]; albums: Record<string, string>; history_off: HistoryOff[] }>("photo_history", { row });
/** The newest entries of the change log. */
export const changeLog = (limit: number) => invoke<LogEvent[]>("change_log", { limit });
/** A page of the change log, newest first, in groups of one batch each. */
export const changeLogPage = (query: LogQuery) => invoke<LogPage>("change_log_page", { query });
/** What changed between two times (inclusive); `to` of null is now. */
export const historyCompare = (from: number, to: number | null) => invoke<Compare>("history_compare", { from, to });
/** Photos per import day under `filter`: [local midnight in ms, count], in date order. */
export const growth = (filter: Filter) => invoke<[number, number][]>("growth", { filter });
/** Entries logged per day, and when the log starts. */
export const logDays = () => invoke<LogDays>("log_days");
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
/** One field over several photos: the value all of them share, how many different values
 *  they have, how many have none, and the most used values with their counts. */
export type Merged<T> = { same: T | null; values: number; empty: number; top: [T, number][] };
/** What the photos at `rows` have: keywords and albums with how many carry each, places with
 *  the photos that have each (most used first; those with no location share the empty
 *  place), and each editable field merged. */
export type Selection = {
  keywords: [string, number][];
  albums: [string, number][];
  places: [Photo["place"], number[]][];
  rating: Merged<number>;
  color: Merged<Color>;
  geo: Merged<[number, number]>;
  iptc: Record<CaptionField, Merged<string>>;
};
export const selectionSummary = (rows: number[]) => invoke<Selection>("selection_summary", { rows });
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

// Describing photos with a vision model (crates/c1ai, app/src-tauri/src/ai.rs).
export type AiLanguage = "en" | "pl" | "both";
/** The API with a key, or Claude Code signed in with a Claude plan. */
export type AiAccess = "api" | "plan" | "gemini";
/** What a description holds (crates/c1ai/src/prompt.rs, `Fields`). */
export type AiFields = {
  caption: boolean; headline: boolean; subject: boolean; keywords: boolean; keywords_new: boolean;
  rating: boolean; scores: boolean; place: boolean;
};
export type AiSettings = {
  access: AiAccess; model: string; language: AiLanguage; effort: "low" | "medium" | "high"; parallel: number;
  fields: AiFields;
  /** The user's own instructions; null: the default ones. */
  instructions: string | null;
  /** Lessons from the corrections in the review (or written by hand), after the instructions. */
  lessons: string | null;
  /** When the lessons last changed, Unix ms. */
  lessons_at: number | null;
  /** A place level guessed at least this likely (%) starts ticked where the photo has none. */
  place_tick: number;
  /** The Gemini model agy is asked for. */
  gemini_model: string;
  /** Gemini gets contact sheets of up to 36 photos rather than one photo per call. */
  sheets: boolean;
  /** The image sent of each photo, alone or on a sheet. */
  image: AiImage;
  /** Keywords not offered to the model, nor matched in its answers. */
  left_out: LeftOut;
};
/** The source's thumbnail (for Capture One its render of the edits) or the preview. */
export type AiImage = "thumb" | "preview";
export type AiModel = { id: string; name: string; input: number; output: number };
/** Claude Code as a plan run would use it. */
export type AiPlan = { installed: boolean; loggedIn: boolean; authMethod: string | null; subscriptionType: string | null; error: string | null };
export type AiView = {
  settings: AiSettings;
  models: AiModel[];
  /** The key in use, masked, and where it comes from. */
  key: [string, "stored" | "environment"] | null;
  /** Only when the plan is chosen. */
  plan: AiPlan | null;
  /** With Gemini chosen: whether agy is installed. */
  gemini: boolean | null;
  /** Why agy's models could not be listed. */
  gemini_error: string | null;
  /** A run can start: a key, Claude Code signed in, or agy there. */
  ready: boolean;
  calls: number;
  /** US dollars spent in this library. */
  spent: number;
  default_instructions: string;
};
/** The prompt a run would send now. */
export type AiPrompt = { system: string; schema: unknown };
export type AiEstimate = {
  photos: number; skipped: number; cost: number; model: string; plan: boolean;
  /** One per photo, or per contact sheet. */
  calls: number;
  via: "Claude" | "Gemini";
};
export type DescribeState = {
  running: boolean; done: number; total: number; failed: number;
  /** Rows being described now. */
  working: number[];
  /** Unix ms. */
  started: number;
  cost: number; error: string | null;
  /** Photos go as contact sheets, a call each. */
  sheets: boolean;
  /** Calls made and to make: one per photo, or one per contact sheet. */
  calls_done: number; calls_total: number;
  /** The calls under way. */
  units: DescribeUnit[];
  /** What the run did, oldest first; kept after it ends. */
  log: DescribeLogLine[];
};
/** A call under way: one photo or a contact sheet. */
export type DescribeUnit = {
  /** From 1: a sheet's number. */
  call: number;
  rows: number[];
  sheet: boolean;
  /** Its image being made, sent and awaited, or about to be tried again. */
  phase: "preparing" | "waiting" | "retrying";
  /** Unix ms: when the phase began, and the call. */
  since: number; began: number;
  /** The try under way, from 1; 0 while preparing. */
  attempt: number;
  note: string | null;
};
export type DescribeLogLine = { at: number; call: number; ok: boolean; text: string };
export type Suggestion = {
  caption_en: string | null;
  caption_pl: string | null;
  headline: string | null;
  subject: string | null;
  /** Pipe paths the catalog has. */
  keywords: string[];
  /** Keywords the catalog lacks. */
  keywords_new: string[];
  rating: number | null;
  quality: number | null;
  composition: number | null;
  confidence: number;
  place: PlaceGuess | null;
};
/** A photo's place names. */
export type PlaceNames = Photo["place"];
/** A place level guessed, with the probability (0..1) that it is right. */
export type Guess = { name: string; p: number };
/** Where the model thinks the photo was taken, level by level, and a position. */
export type PlaceGuess = {
  country: Guess | null;
  state: Guess | null;
  city: Guess | null;
  location: Guess | null;
  /** Known when the gazetteer knows the country. */
  country_code: string | null;
  point: { lat: number; lon: number; p: number; radius_km: number } | null;
};
export type SuggestionStatus = "Proposed" | "Accepted" | "Rejected";
export type SuggestionRow = {
  row: number;
  at: number;
  provider: string;
  model: string;
  suggestion: Suggestion;
  warnings: string[];
  cost: number;
  status: SuggestionStatus;
  /** What the photo has now, for comparing. */
  current: {
    caption: string | null; headline: string | null; rating: number | null; keywords: string[];
    place: PlaceNames;
    /** It has a position: a guessed one is never given. */
    placed: boolean;
  } | null;
};
/** What the review showed a photo to have: a field taken is set only while it still has it. */
export type Seen = { caption: string | null; headline: string | null; rating: number | null; place: PlaceNames };
/** What the user takes from a suggestion; fields left out are not touched. */
export type Accept = {
  row: number; keywords: string[]; caption?: string; headline?: string; rating?: number;
  /** The place levels taken. */
  place?: Partial<PlaceNames>;
  /** A position taken, given only to a photo with none (marked ≈). */
  point?: [number, number];
  seen: Seen;
};
/** The ops that undo it, and the photos whose taken field changed since shown (left as is). */
export type Accepted = { undo: [number, Op][]; changed: number[] };
export const aiSettings = () => invoke<AiView>("ai_settings");
export const setAiSettings = (settings: AiSettings) => invoke<AiView>("set_ai_settings", { settings });
/** Stores the key in the Windows Credential Manager; an empty one removes it. */
export const setAiKey = (key: string) => invoke<AiView>("set_ai_key", { key });
export const aiPrompt = () => invoke<AiPrompt>("ai_prompt");
export const aiEstimate = (rows: number[], again: boolean) => invoke<AiEstimate>("ai_estimate", { rows, again });
/** Describes the photos in the background ("describe" events, then "describe-done"). */
export const startDescribe = (rows: number[], again: boolean) => invoke<void>("start_describe", { rows, again });
export const stopDescribe = () => invoke<void>("stop_describe");
export const describeState = () => invoke<DescribeState>("describe_state");
export const suggestions = (status: SuggestionStatus | null) => invoke<SuggestionRow[]>("suggestions", { status });
/** Queues what was taken. */
export const acceptSuggestions = (picks: Accept[]) => invoke<Accepted>("accept_suggestions", { picks });
export const setSuggestionStatus = (rows: number[], status: SuggestionStatus) =>
  invoke<void>("set_suggestion_status", { rows, status });
/** What was taken from a suggestion accepted, as the card had it. */
export type Taken = {
  /** Pipe paths ticked, those the photo had included. */
  keywords: string[]; caption: string | null; headline: string | null; rating: number | null;
  /** The levels taken; null where none was. */
  place: PlaceNames;
  /** The guessed position was taken. */
  point: boolean;
};
/** A suggestion accepted or rejected, kept until removed: what lessons are drawn from. */
export type Review = {
  seq: number;
  /** Its photo, while in the library. */
  row: number | null;
  at: number; key: string; model: string; prompt: number;
  status: SuggestionStatus;
  suggestion: Suggestion;
  taken: Taken | null;
};
/** The reviews, newest first, and how many could not be read. */
export const aiReviews = () => invoke<{ reviews: Review[]; unreadable: number }>("ai_reviews");
export const deleteAiReviews = (seqs: number[]) => invoke<void>("delete_ai_reviews", { seqs });
/** Lessons proposed from the corrections, to replace those in use, and why; `cut` lessons
 *  past the limit were left out. */
export type Refined = { lessons: string; rationale: string; cut: number };
/** Asks Claude for lessons from corrections (a line each): those since the lessons in use
 *  were saved, those `earlier`, and the count accepted without one. */
export const refineLessons = (reviews: string[], earlier: string[], unchanged: number) =>
  invoke<Refined>("refine_lessons", { reviews, earlier, unchanged });

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
/** Whether the History tab is on for the open catalog (it is unless turned off). */
export const historyEnabled = () => invoke<boolean>("history_enabled");
/** Off, the change log stops keeping what changes replaced and nothing reads it as history. */
export const setHistory = (on: boolean) => invoke<void>("set_history", { on });
/** Why the catalog's settings file could not be read when it was opened, if it could not. */
export const settingsProblem = () => invoke<string | null>("settings_problem");
export type WriteReport = { written: number; backup: string | null; warnings: string[] };
export const writeApply = () => invoke<WriteReport>("write_apply");

// WebView2 on Windows reaches custom protocols as http://<scheme>.localhost.
const base = navigator.userAgent.includes("Windows") ? "http://c1img.localhost" : "c1img://localhost";
export const thumbUrl = (row: number) => `${base}/thumb/${row}`;
export const previewUrl = (row: number) => `${base}/preview/${row}`;
/** The contact sheet an AI run (started at `run`, Unix ms) sent in call `call`, while kept. */
export const sheetUrl = (run: number, call: number) => `${base}/sheet/${run}/${call}`;

/** Where the analysis draws the lines between classes (shares are 0..1). */
/** The colour names the analysis measures, in the order of `hue_stats` (`c1analysis::HUES`). */
export const HUE_NAMES = ["brown", "red", "orange", "yellow", "green", "teal", "blue", "purple", "pink"] as const;
/** What makes a photo a colour: covering a share of it, or a vivid patch of it. */
export type ColourMode = "Covers" | "Patch";
export type ClassSettings = {
  mono_chroma: number;
  colour_share: number;
  colour_floor: number;
  colour_mode: ColourMode;
  patch_share: number;
  patch_chroma: number;
  dark_below: number;
  bright_above: number;
  clipped_share: number;
  blocked_share: number;
  flat_below: number;
  soft_below: number;
  blurry_below: number;
  square_ratio: number;
  panorama_ratio: number;
  duplicate_bits: number;
  series_bits: number;
  series_gap_ms: number;
  series_min: number;
};
/** What some settings would give: `<dimension>|<class>` → photos, and photos in groups. */
export type ClassPreview = {
  measured: number;
  counts: Record<string, number>;
  duplicates: number;
  series: number;
};
export const classSettings = () => invoke<ClassSettings>("class_settings");
export const defaultClassSettings = () => invoke<ClassSettings>("default_class_settings");
/** Classes the measured photos again; returns the settings kept (brought into range). */
export const setClassSettings = (settings: ClassSettings) =>
  invoke<ClassSettings>("set_class_settings", { settings });
export const previewClasses = (settings: ClassSettings) =>
  invoke<ClassPreview>("preview_classes", { settings });
