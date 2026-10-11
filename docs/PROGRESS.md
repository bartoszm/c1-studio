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
- Two catalogs exist under `C:\c1_catalogs` (`fuji`, `masha_piotr`).
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

Stage 1 is done. The app does not load originals: they may be offline, so the cached
previews are the largest image it shows. The `original` tier stays in `c1cat image`, untested
on real files.

- **Fixed:** after a filter change, cells already on screen kept the previous result's
  thumbnails while the details panel and viewer showed the photo now at that position.
  The page map was swapped for a new one, but the variable holding it was not reactive.

## Stage 2: editing and sync to Capture One

### Writing ratings and colour tags (`source-c1/src/write.rs`)

Ported from the Python `safety.py` and `cmd_meta.py`. Capture One now reports ratings
and colour tags as writable.

- **Write status, cheapest check first:** `Locked` while Capture One runs, while a
  rollback journal is present, or while any program holds the file (an open with no
  sharing fails). `ReadOnly` when the journal mode is not `delete` or the schema is not
  160800. `Ready` otherwise.
- **`plan` reads the live catalog,** so it works with Capture One open. Each change
  compares its `base` with the catalog: apply, no-op (already so) or conflict (someone
  changed it, or the photo is gone).
- **`apply`:**
  - refuses unless the status is `Ready`;
  - backs the catalog up through SQLite's backup API into `<data dir>/backups`, with a
    JSON manifest beside each copy;
  - an unchanged catalog reuses its newest backup, and the newest 5 are kept;
  - then, in one `BEGIN IMMEDIATE` transaction, it re-plans and writes nothing if the
    catalog no longer agrees (`StalePlan`).
- **Where the values go:** both metadata rows (adjustment and combined layer), never
  the default layer. Unrated and no colour are NULL. `ZDOCUMENTCONTENT.ZDATEMODIFIED`
  is stamped.
- **CLI:** `c1cat rate --set 0-5 | --clear` and `c1cat color --set NAME`. They take the
  usual selectors, are dry runs unless `--apply` is given, and refuse an empty
  selection.
- **Tests:**
  - A test catalog built from the real schema (`tests/fixtures/schema.sql`) runs the
    unit tests and the conformance checks (`check_read`, `check_write`).
  - It also runs an end-to-end apply: backup first, then the next snapshot sees the
    change.
- **Differential test against Python:** `tools/difftest/writetest.py`.
  - It copies the catalog twice and runs the same six edits through both
    implementations: rate, colour, clear, none and 0 on 20 variants in four starting
    states.
  - It then compares every table row by row. On the real catalog the copies are
    identical: 33 changes, 32 metadata rows differ from the original, and nothing else
    changed.

### Editing in the app, the queue, and writing to Capture One

- **Editing:**
  - Click picks a photo; Ctrl+click adds or removes one; Shift+click adds a range;
    Ctrl+A picks the whole result; Esc goes back to one.
  - Capture One's keys: 0–5 set the stars (0 clears); `-`, `+` and `*` tag red, green
    and yellow.
  - The details panel's stars and colour dots edit too, and say how many photos an
    edit applies to.
  - Ctrl+Z undoes an edit and Ctrl+Y (or Ctrl+Shift+Z) redoes it, one key press or
    click at a time, however many photos it touched.
- **The library shows an edit at once** (filters, counts, panel), whether Capture One
  is open or not. A blue dot marks photos with edits not yet written, and the footer
  says how many are waiting.
- **The queue** (`c1index/src/edits.rs`) lives in `edits.redb`, beside the store and not
  in it. The store is a copy of the sources and is dropped on a format change; this is
  the user's own work.
  - One entry per photo and field. `base` is what Capture One held when the field was
    first edited.
  - Editing a field back to that value removes the entry.
  - Values are JSON, so an old queue still reads after `Op` grows.
- **Syncs keep queued edits on top** of what Capture One sends:
  - Capture One already holding the wanted value settles the entry.
  - Capture One changing the field to something else marks a conflict (`theirs`), and
    the user's value keeps showing until they choose.
- **Event log:** every edit made here, and every rating, colour, keyword and album
  change a sync brings in, goes to an event log (for the history views of stage 4).
- **"Write to Capture One…"** pulls Capture One's state, then shows:
  - what would be written;
  - conflicts, each with *Keep mine* or *Take Capture One's*, one by one or all;
  - edits that can no longer be written;
  - edits already matching.

  It writes only while Capture One is closed and every conflict is settled. On
  success the queue empties and the backup's path is shown. *Undo all…* drops the
  queue.
- **Tests:**
  - Seven library tests on a memory source:
    - an edit shows and is queued;
    - queued edits stay on top of a sync;
    - a conflict is marked, then kept or taken;
    - the same change made in the source settles the queue;
    - the queue is written to the source and settled;
    - discarding shows the source's value again;
    - the queue outlives a dropped store.
  - In the app, against the live library: rate, Ctrl+click, tag, undo, redo, and the
    dialog's plan. Nothing was written, and the queue was left as found.
  - End to end on a copy of the catalog, in a second app instance:
    - two edits, then a rating changed in the copy behind the app's back;
    - the dialog shows the conflict, *Keep mine*, write;
    - the copy holds both values on the adjustment and combined rows only;
    - the backup holds the state before the write;
    - both pass `PRAGMA integrity_check`.

### Writing on demand, autosave as an option

At the user's request, nothing goes to Capture One unless they ask: edits queue in the
app until *Write to Capture One…*. **Autosave** (a footer checkbox, off by default, kept
in `<data dir>/settings.json`) writes the queue by itself.

- **How autosave runs:** a backend loop (`autosave_loop` in the app) checks every 4 s.
  It writes once there has been no edit for 8 s, the queue is non-empty, and the
  catalog is free, through the same plan, conflict and backup rules as the dialog.
- **What it says in the footer:**
  - "saving to Capture One when you pause…";
  - waits for Capture One to close (or for another program holding the file);
  - "autosave paused: N conflicts" (opens the list);
  - "saved N edits at 09:21";
  - failures.

  A pause for conflicts or a failure is not retried until the queue changes.
- **Backups shared by the app's own writes:** while only the app has written since the
  newest backup (the catalog still matches the size and time recorded after the app's
  last write) and the backup is under an hour old, the next write reuses it, instead of
  copying the 730 MB catalog every few seconds.
  - Backup times are whole nanoseconds and milliseconds: decimals did not survive the
    trip through JSON exactly.
- **Fixed:** photos that tie on the sort key (same capture second, same import, same
  rating) swapped places when one was edited, because an edit re-adds its document
  and the index broke ties by document order. The sort keys now carry the row
  (schema 3): `captured_sort`, `imported_sort` and `rating_sort`.
- **Tested end to end** on a copy of the catalog, in a second app instance:
  - autosave off: an edit stays queued and the catalog is unchanged;
  - turned on: written after the pause; a second edit shares the backup;
  - the file held open with no sharing (as Capture One does): it waits, and saves a
    second after the file is released;
  - a rating changed in the copy behind the app: "paused: 1 conflict", nothing
    written; *Keep mine*, then saved with the user's value;
  - a new backup is made after the outside change.

### Keywords

- **Writing** (`source-c1/src/keywords.rs`), ported from the Python `keywords.py` and
  `cmd_keywords.py`:
  - A photo's keyword string is parsed with its indices. Added paths are appended and
    numbered after the last; removed ones are dropped and the indices made dense again.
  - The string is written to the adjustment row sorted by path (bytes) and to the
    combined row sorted by index. No keywords is NULL.
  - A missing keyword node (and any missing level above it) is inserted into the
    `ZKEYWORD` nested set the same way Python does: everything at or after the gap is
    lifted into a disjoint range and brought back two higher, so the UNIQUE and CHECK
    constraints hold at every step.
  - The tree is checked before the commit; a broken tree rolls the write back. Names
    with `,` or an empty level are refused.
  - Keyword sets do not conflict: adding one that is there, or removing one that is
    not, is a no-op.
- **CLI:** `c1cat keyword add|remove PATH... <selector> [--apply]`.
- **Differential test:** `writetest.py` now also runs six keyword edits:
  - an existing keyword;
  - new nested ones;
  - a child under the tree's first root, which shifts 564 nodes;
  - three at once;
  - two removals.

  On the real catalog the Rust and Python copies are identical, `ZKEYWORD`
  included. (The Python side rebuilds its mirror before each keyword edit, since it
  starts from the mirror's keywords.)
- **The queue** keeps one entry per photo and keyword (`keyword:<path>`). Adding then
  removing the same keyword cancels out. Multi-keyword ops are split.
- **In the app:**
  - each keyword in the details panel has a × to remove it;
  - an "Add a keyword" field (Ctrl+K) completes from the catalog's 560 keywords, most
    used first, and takes nested paths (`trips | japan`);
  - both apply to every picked photo;
  - queued keywords show in blue;
  - the panel sorts keywords so the list keeps still;
  - the Write dialog lists `keyword: + uitest|nested` / `keyword: − cola`.
- **Tests:** four write tests and one library test. In the live app: add, remove, the
  dialog, undo twice; the queue was left empty.

### Next

1. Albums: add and remove photos, create albums, with a differential test.
2. A list of backups, with restore.
3. Keyword library edits (rename, move, merge, delete), if wanted: Python has them;
   they rewrite every photo using the keyword.
