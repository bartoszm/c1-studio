# Progress log

The plan (in Polish) is in [PLAN.md](PLAN.md). Stages are numbered as there.

## 2026-10-08 — stage 0, first increment

### Done

- **Monorepo** `c1-studio`: a Cargo workspace. The Python `c1cat` was imported with its
  git history into `legacy/c1cat-py` (via `git subtree`) to serve as the oracle for
  differential tests.
- **`c1model`**, the contract:
  - canonical `Photo`, `Collection`, `KeywordNode`;
  - `Change`/`Op` with `base` values for conflict detection;
  - the `PhotoSource` trait;
  - `MemorySource` (the reference implementation);
  - `conformance::{check_read, check_write}`.
- **`source-c1`**, Capture One read-only:
  - catalog discovery (`C1_CATALOG`, then `%APPDATA%\c1studio\config.json`, then
    `C:\c1_catalogs\*\*.cocatalogdb`);
  - a torn-copy-safe snapshot (retries on a hot journal or a change mid-copy);
  - extraction ported from `mirror.py` (variants, collections and membership, keyword
    library with usage counts, styles and local/retouch layer counts);
  - thumbnail and original tiers;
  - Capture One process detection.
- **`c1index`**, the library:
  - a redb store (row → photo, fingerprints, keys, collections, keywords, meta);
  - a tantivy index whose schema covers every filter of the Python selector;
  - a custom facet collector that counts per dimension straight from the columnar
    fields;
  - `Filter`, which mirrors `selector.py`;
  - incremental sync by source cursor plus per-photo fingerprint, with deletions
    detected on full snapshots;
  - sorting and paging.
- **`c1cat`** (Rust CLI): `sync`, `status`, `find` (`--ids`/`--json`), `count`, `facets`,
  with the same selection flags as the Python CLI.

### Measured on the real catalog (99 716 photos, debug build with optimised deps)

| What | Time |
|---|---|
| Snapshot: copy plus extraction of the whole catalog | 2.4 s (Python in WSL: ~23 s) |
| First full sync into the library: store plus index | 13 s |
| Re-sync, catalog unchanged | 0.18 s |
| Facets (rating, camera, keyword tree) over 28 k matches | 3.7 ms |
| Facets (city, country) over all 99 716 | 8.8 ms |

### Found along the way

- **GPS in degrees-and-minutes form** (`50,2.2566833N`) is used by ~800 images. The
  Python `parse_gps` silently dropped all of them: 6 photos with GPS before the fix, 808
  after. Fixed in Rust only; the legacy oracle still has the bug, so `--gps` and
  `--near` will differ in the differential tests by design.
- `ZVERSIONINFO` keeps one row per Capture One version that ever opened the catalog.
  The current schema is the newest row (`ORDER BY Z_PK DESC`), not the first.
- Two catalogs exist under `C:\c1_catalogs` (`fuji`, `other`).
  `%APPDATA%\c1studio\config.json` points at `fuji`.

### Tests

23 unit and integration tests pass (`cargo test --workspace`); `cargo clippy` is clean.
Two more tests run against the live catalog and are `#[ignore]`d by default:
`cargo test -p source-c1 --test live -- --ignored`.

### Deviations from the plan

- `PhotoSource` is synchronous, not async. Every Capture One operation is blocking
  SQLite or file I/O, and the UI will call it from a worker thread. A REST source can
  block internally.
- Change detection uses the catalog file's cursor (mtime and size) plus per-photo
  fingerprints. Capture One has no usable per-row modification dates: they hold a .NET
  `MinValue` sentinel.
- The Python oracle runs in WSL (where it already works) rather than getting a Windows
  platform layer.

## 2026-10-10 — stage 0, the speed gate

### Done

- **`c1cat bench`** builds a synthetic library and times it against the gate.
  - The library has 250 k photos and 2 000 keywords. It is shaped like a real catalog:
    photos come in shoots that share a camera, a place and keywords; use is skewed
    towards a few keywords, cameras and places; most photos are unrated.
  - It is fed through the normal `Library::sync`. It lives in
    `%LOCALAPPDATA%\c1studio\bench\<photos>-<keywords>-<seed>` and is reused on later
    runs.
  - Every query asks for the first 200 photos by date, plus counts over the 8 sidebar
    dimensions (rating, colour, year, camera, lens, keyword tree, album, country).
  - Each filter is built around a real photo in the library, so it always has at least
    one hit.
  - A failed gate exits non-zero.
- **Facet counting fixed.** Ordinals were turned back into strings with one random
  dictionary lookup each; on a 2 000-node keyword tree that took 150 ms per query. The
  collector now reads the dictionary once, in ordinal order
  (`sorted_ords_to_term_cb`).
- **Segments are searched in parallel** (tantivy's multithread executor). This mainly
  helps queries that match most of the library.
- A test now compares facet counts with a brute-force count, across two segments and
  thousands of distinct keywords.
- Four source files had been saved with a BOM and double-encoded UTF-8: `find` printed
  `â˜…` for each star. They are repaired.

### Gate: passed (release build, i7-1365U, 250 000 photos)

| Query | p50 | p95 | Gate |
|---|---|---|---|
| Open the library | 50 ms | | < 1 s ✓ |
| Browse everything, 8 counts | 6.1 ms | 6.7 ms | — |
| 1 filter | 2.2 ms | 5.9 ms | — |
| 3–5 filters | 0.8 ms | 1.7 ms | p95 < 10 ms ✓ |
| Text plus 1–3 filters | 0.5 ms | 1.4 ms | p95 < 30 ms ✓ |

Before the two fixes, 3–5 filters took 30.9 ms at p95 and browsing everything took
224 ms at p50. Roaring bitmaps, the plan's fallback, are not needed.

On the real catalog, counts over all 99 716 photos dropped from 8.8 ms to 3.6 ms.

The synthetic build itself takes about 14 s for 250 k photos (sync 13.6 s), and the
library is 135 MB of store plus 99 MB of index in 13 segments.

### Differential test: Rust matches the Python oracle

`tools/difftest/difftest.py` sends the same selector flags through the Rust
`c1cat find --ids` and through the legacy Python selector in WSL (`oracle.py`, which
imports `legacy/c1cat-py` and runs every case in one process). It compares the sets of
variant ids.

- 75 cases cover every selector flag, combinations of them, and cases built from real
  photos (ids, image ids, folders, exact filenames).
- The result on the real catalog: 72 identical, 3 different by design (`--gps has`,
  `--gps none`, `--near`: the GPS parsing bug above; Python sees 6 photos with GPS,
  Rust 808).
- One more bug in the oracle: its comparison regex takes whole numbers only, so
  `--aperture '<=2.8'` (the example in its own help) and `--rating '>2.5'` fail there.
  Those two cases go to Python as `0-2.8` and `3-5`. Fixed in Rust only.
- WSL and Windows are both in Europe/Warsaw time, so `--year`, `--month`, `--from` and
  `--to` agree.

Run it with `python tools/difftest/difftest.py` after `cargo build --release -p c1cat`;
`-k NAME` runs a subset. A run takes ~18 s.

### Next (rest of stage 0)

1. The preview tier: `.cop` JPEG-XL with white balance taken from develop settings
   (`images.py`).
2. The Tauri 2 + Svelte app skeleton in `app/`, and CI (`cargo test`, `clippy`, a
   `rustfmt.toml` matching the ~120-column style).
