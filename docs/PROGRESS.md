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

### Preview tier: Capture One's `.cop`, rendered in Rust

- **`c1image`**, a new crate, turns any source's `ImageRef` into sRGB pixels:
  - `.cot` thumbnails and JPEGs are decoded with their EXIF orientation applied;
  - `.cop` previews get white balance, then highlight scaling (the 99.5th percentile
    becomes white, as in Python), then the sRGB curve;
  - for raws, it takes the camera's embedded JPEG: the RAF header offset, or the
    largest IFD preview in DNG/CR2/NEF/ARW, skipping lossless-JPEG raw data.
- **`source-c1`** now serves the preview tier. `.cop` is at
  `Cache\Previews\<ZIMAGE.ZSIDECARPATH>.cop`, with a case-insensitive fallback.
- **`c1cat image --tier thumb|preview|original -o DIR [selector]`** writes JPEGs, to
  check the result by eye.
- Speed: a 2560 px preview takes ~265 ms, a thumbnail ~5 ms.

Three findings, each checked against Capture One's own thumbnails:

- **jxl-oxide tone-maps by default.** Capture One declares a 2 000-nit intensity target.
  Asked for SDR, jxl-oxide applies Rec. 2408 tone mapping, which bends the linear data;
  its values came out ~7.7× libjxl's, and not proportionally. We ask for PQ output and
  invert PQ ourselves. The result equals libjxl's (imagecodecs) to within float rounding
  at every pixel checked.
- **White balance comes from the combined layer.** The other candidates are the
  adjustment layer (user overrides, set on 7 986 variants) and the default layer
  (Python's choice). Styles also change white balance, so the combined layer differs
  from both on 6 800 variants. On 300 such variants, combined matched Capture One's
  thumbnail best 144 times, against 80 for adjustment and 76 for default (median
  channel-balance error 0.198 / 0.225 / 0.234). The fallback order is combined, then
  adjustment, then default.
- **Previews are stored unrotated.** `ZROTATION` on the combined layer is the
  clockwise turn to apply (270 on 3 135 variants, 90 on 402, 180 on 22, the user's own
  rotation included). A few values carry straightening (`358.42`); they snap to the
  nearest quarter turn. `ImageRef` gained a `rotation` field for this. Python never
  rotated previews.

Not done here:

- **Originals are untested on real files.** Drive H: is offline; the RAF and TIFF parsers
  are only tested on synthetic files.
- **Originals use EXIF orientation, not `ZROTATION`.** A user's rotation in Capture One
  will not show on them yet.

### Previews take Capture One's look from its thumbnail

What Capture One keeps in `Cache\`:

| File | What it is |
|---|---|
| `Thumbnails\*.cot` | JPEG, 450 px on the long edge (all 3 000 sampled), with every edit applied. |
| `Previews\*.cop` | JPEG-XL, 2560 px, linear sensor data before white balance and tone. **Already cropped and straightened** (a 2.26 crop gives a 2560×1133 `.cop`); quarter turns are not applied. |
| `Previews\*.cof` | 1000×667 greyscale detail map, not a picture. Possibly free sharpness data for stage 3. |
| `Browser\*.bc` | 34-byte records. |

So Capture One stores no large render with edits: it develops the `.cop` on the GPU
whenever it shows one. The preview now borrows what it lacks from the `.cot`, which is
Capture One's own finished render:

- **Black and white** when the thumbnail is grey, or, without a thumbnail (4% of
  variants), when `ZBWENABLED` is set or the film curve is ACROS/Monochrome/Sepia.
  - The flags alone miss many: in a sample, 434 of 481 grey thumbnails had `ZBWENABLED`
    off.
  - A raw shot in ACROS with the curve on "Auto" takes the simulation from the raw file,
    which the catalog does not record.
- **The tone curve:** the preview's brightness distribution is matched to the
  thumbnail's, keeping each pixel's colour ratios.

Measured with `cargo run --release -p c1cat --example lookcheck` (120 random photos,
median absolute difference from the thumbnail per channel, out of 255):

| | before | + b&w | + b&w + tones |
|---|---|---|---|
| black-and-white photos (48) | 32.7 | 32.5 | 8.1 |
| colour photos (72) | 27.6 | 27.6 | 6.8 |

Two limits:

- **Matching aims at the thumbnail, which is itself the target**, so the numbers flatter
  it a little. But the match is one global curve, which cannot copy detail, and side by
  side the previews look like Capture One's.
- **A stale thumbnail skews the curve slightly.** For example, one straightened photo
  still has a thumbnail with black corners. Colour saturation and colour grading are not
  matched.

`ImageRef` gained `monochrome` and `look` (a path to the small render).

### The app skeleton: Tauri 2 + Svelte 5

`app/` holds the window (`app/src-tauri`, a workspace member) and the UI (`app/src`:
Svelte 5, Vite 8, TypeScript 5.9; `svelte-check` does not support TypeScript 7 yet).

- **Commands:** `status`, `query` (any `Filter` as JSON, plus `QueryOptions`), `photo`,
  `collections`, `sync`. `Filter`, `Range` and `QueryOptions` now take serde defaults,
  so the UI sends only the fields it sets.
- **The `c1img:` protocol:**
  - `/thumb/<row>` sends Capture One's `.cot` as stored, without decoding it;
  - `/preview/<row>` renders the `.cop` (~250 ms) and sends a JPEG;
  - on Windows the page reaches it as `http://c1img.localhost/...`.
- **Startup:** the app opens the catalog and library, and syncs in the background.
  A `library-changed` event refreshes the window when anything moved.
- **UI, dark like Capture One:**
  - a search bar;
  - the sidebar counts rating, colour, year, camera, keyword tree (one level at a time,
    with a breadcrumb), albums and country; click a value to filter by it;
  - a grid of thumbnails that loads 300 at a time as you scroll;
  - a viewer that shows the thumbnail at once and swaps in the preview, with ← → Esc and
    EXIF, rating, place and keywords.

Checked on the real catalog by driving the WebView over its DevTools port:

| Query | Photos | Time |
|---|---|---|
| Everything | 99 716 | 9.9 ms |
| Year 2025 + keyword street | 8 743 | 16.7 ms |
| + text "tokyo" | 689 | 5.1 ms |

The viewer showed the black-and-white preview, and no errors appeared. A cold debug
build of the app takes 4.5 min.

Known gaps, for stage 1 (the first two fixed below):

- **Selecting a value hides the others in its group.** Counts follow the current filter,
  so with 2025 picked, Year lists only 2025. The fix is counting each group without its
  own constraint.
- **The grid is not virtualised.** It only appends pages of 300.
- **No thumbnail cache headers beyond `max-age`.**

## Stage 1: the browser

### Sidebar counts per group, and a virtual grid

- **Each group counts without its own filter** (`QueryOptions::facets_ignore_own`,
  `Filter::without`). With 2025 picked, Year still lists every year, counted over the
  other filters; the other groups count over everything picked. It costs one more
  counting pass per group the filter picks from.
  - Benchmark at 250k photos, counting this way: 3–5 filters p95 4.9 ms (was 1.7 ms
    counting over the matches only), text p95 2.6 ms, everything p95 8.4 ms. Gates pass.
- **The grid keeps only what is on screen** (`app/src/Grid.svelte`): about 60–85
  cells at any scroll position, absolutely placed inside a box as tall as the whole
  result. Photo rows load in pages of 500 as the grid reaches them, and the viewer
  loads the next page when stepping past one.
  - On the real catalog (99 716 photos, a 2.2 million px tall grid), jumping to the
    middle or the end fills the screen with thumbnails in about 70 ms.
  - Closing the viewer scrolls the grid to the photo last shown.
  - The scrollbar is always present: WebView2 did not report the width change when it
    appeared, which left the last column under it.

### The library tree: albums, smart albums, folders, keywords

The sidebar now starts with three trees, each node counted with the other filters
applied (its own pick aside). Picking a node filters by it, and picking it again lets
go. Open branches and folded sections are remembered between runs.

- **Albums:** groups, projects, albums and smart albums, nested as in Capture One.
  Groups only expand. The catalog's hidden root ("[Collection]", pk 1) is no longer
  reported as a collection.
- **Smart albums.** Capture One stores their rules (`ZSMARTCRITERIA`), not their
  members. `source-c1/src/smart.rs` reads the rules into `c1model::SmartRule`, and the
  library expands a picked smart album into a filter.
  - The catalog's 27 smart albums use three kinds of condition, and all three are
    supported: keyword contains (substring of a keyword's name at any level, ignoring
    case), rating (is / at least / greater than) and capture year. Anything else is
    kept as `Unsupported`, and the album shows disabled with the reason.
  - "street 4+" selects 5 084 photos, the same as the equivalent filter written by hand.
  - Counting all 27 under a filter takes 14 ms, in a separate call so it never delays
    the grid.
- **Folders:** a new `folder` dimension (schema 2) holds every folder above each photo
  in the path's own case (`H:`, `H:/fotos`, `H:/fotos/2026_06`). Names sort naturally,
  ignoring the leading spaces some folder names on H: have.
- **Keywords:** the whole tree from the counts, replacing the one-level list and
  breadcrumb.
- **Filters gained** `keywords_containing`, and `all_of` / `any_of` groups of nested
  filters (what smart albums expand into).
- **The store now records its model version** (`MODEL_VERSION` 2, as `Collection`
  gained `rule`). A store written by another version is dropped and refilled by the
  next sync, which takes a few seconds for the real catalog.

### Selection and the details panel

- **Click selects, double-click or Enter opens** the viewer, as in Capture One. Arrow
  keys, Page Up/Down, Home and End move the selection and keep it on screen.
- **The details panel** (right, toggled with `i` or the Details button, remembered)
  shows the selected photo:
  - rating, colour label, edited and in-trash marks;
  - capture time, camera, lens, exposure, pixel size;
  - keywords, albums, place and GPS (with where the position came from), IPTC caption
    fields, folder, file size, import time, styles, layers and the catalog id.
- **Links filter the grid:** each level of a keyword, an album, the folder and the
  camera. The tree opens the branches above what was picked so it shows.
- **The viewer shows the same panel** at its side while it is on, and stepping with
  ← → updates both.

### Next

1. Push to GitHub so CI runs. `.github/workflows/ci.yml` runs on Windows: UI check and
   build, then `cargo fmt --check`, `clippy -D warnings` and `cargo test`. `rustfmt.toml`
   keeps the 120-column style, and the code is formatted with it.
2. Check the original tier on real RAF/DNG files once H: is mounted.
3. Stage 1, continued: originals at 100% from H: (needs the drive mounted).
