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

export const status = () => invoke<Status>("status");
export const query = (filter: Filter, options: QueryOptions) => invoke<QueryResult>("query", { filter, options });
export const smartCounts = (filter: Filter) => invoke<[string, number][]>("smart_counts", { filter });
export const photo = (row: number) => invoke<Photo | null>("photo", { row });
export const collections = () => invoke<Collection[]>("collections");
export const sync = () => invoke("sync");

// WebView2 on Windows reaches custom protocols as http://<scheme>.localhost.
const base = navigator.userAgent.includes("Windows") ? "http://c1img.localhost" : "c1img://localhost";
export const thumbUrl = (row: number) => `${base}/thumb/${row}`;
export const previewUrl = (row: number) => `${base}/preview/${row}`;
