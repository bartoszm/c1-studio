// The Rust side as typed calls. Shapes follow serde's defaults for the Rust types in
// crates/c1index (Filter, QueryOptions, QueryResult) and crates/c1model (Photo).

import { invoke } from "@tauri-apps/api/core";

export type Dim = "rating" | "color" | "year" | "camera" | "lens" | "keyword_tree" | "collection" | "country" | "city";

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
};

export type Sort = "DateDesc" | "Date" | "RatingDesc" | "ImportedDesc" | "Filename";

export type QueryOptions = { sort?: Sort; limit?: number; offset?: number; facets?: Dim[] };

export type QueryResult = {
  total: number;
  rows: number[];
  facets: Partial<Record<Dim, [string, number][]>>;
  micros: number;
};

export type Status = { catalog: string; source: string; photos: number; writes: string };

export type Photo = {
  id: { source: string; ext: string };
  file: { name: string; ext: string; path: string | null };
  captured_at: number | null;
  exif: {
    camera_model: string | null;
    lens: string | null;
    iso: number | null;
    aperture: number | null;
    shutter: number | null;
    focal: number | null;
  };
  rating: number | null;
  color: Color | null;
  keywords: string[][];
  place: { city: string | null; country: string | null };
};

export type Collection = { id: string; name: string; path: string };

export const status = () => invoke<Status>("status");
export const query = (filter: Filter, options: QueryOptions) => invoke<QueryResult>("query", { filter, options });
export const photo = (row: number) => invoke<Photo | null>("photo", { row });
export const collections = () => invoke<Collection[]>("collections");
export const sync = () => invoke("sync");

// WebView2 on Windows reaches custom protocols as http://<scheme>.localhost.
const base = navigator.userAgent.includes("Windows") ? "http://c1img.localhost" : "c1img://localhost";
export const thumbUrl = (row: number) => `${base}/thumb/${row}`;
export const previewUrl = (row: number) => `${base}/preview/${row}`;
