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

### Next (rest of stage 0)

1. `c1cat bench`: a synthetic library of 250 k photos × 2 000 keywords, checked against
   the gate (p95 < 10 ms for 3–5 filters with facets, < 30 ms for text with filters,
   open < 1 s).
2. A differential test script: the same selector flags through the Rust `c1cat find --ids`
   and through the Python oracle in WSL, comparing the id sets. Excluded by design:
   `--gps`/`--near` (GPS bug above). Also check WSL's timezone, which drives
   `--year`/`--month` in Python.
3. The preview tier: `.cop` JPEG-XL with white balance taken from develop settings
   (`images.py`).
4. The Tauri 2 + Svelte app skeleton in `app/`, and CI (`cargo test`, `clippy`).
