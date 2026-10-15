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

### Keyword tree: rename, join (replace), group, ungroup, and a search bar

- **One primitive** (`c1model::KeywordRemap`): a list of `from → to` paths. Each
  keyword, with everything under it, moves to its new path; one landing on a keyword
  already there folds into it. The four operations are built on it:
  - rename;
  - join several into one, existing or new (replace is a join of one);
  - group under a parent, existing or new, or move to the top level;
  - ungroup: the children move up a level, and the group goes if no photo carries it
    itself.
- **Writing** (`source-c1/src/keywords.rs`):
  1. Every photo whose keyword string has a moved path is rewritten, keeping index
     order and dropping duplicates, as Python's `_rewrite_paths` does.
  2. The tree is reshaped. Nodes keep their `Z_PK` and columns; folded ones are
     deleted once nothing points at them (the bundled SQLite enforces `ZPARENT`'s
     foreign key). Missing parents are created.
  3. Every moving node first steps aside under a temporary name, so
     `UNIQUE(ZNAME, ZPARENT)` never trips; a name swap works.
  4. The intervals are renumbered (Python's `rebuild_tree` order) only when the
     structure changed; a plain rename leaves them, as Python does.
  5. The whole tree is checked before the commit.
- **CLI:** `c1cat keyword rename | move | merge | group | ungroup`.
- **Differential test:** rename, move to the top level, move under an existing and
  under a new parent, and merge, on the real catalog. Identical to Python, `ZKEYWORD`
  included (565 rows).
- **Queued like everything else** (`remaps` in `edits.redb`, applied in order):
  - the library rewrites its photos at once;
  - syncs lay the remaps over what Capture One sends;
  - queued keyword edits follow their keyword to its new path;
  - the keyword list (completion) shows the tree as it will be.
- **Undo:** drops the remap and re-reads the photos it touched from the source: their
  fingerprints are cleared, and the next sync is a full one.
- **Planning:** photo edits are planned against the catalog as the remaps will leave
  it (`PhotoSource::plan_after`), so removing a keyword just renamed is not lost.
  Each remap is checked after the earlier ones; one whose keywords are gone is listed
  as blocked.
- **Writing order:** remaps first, then the photo edits, in one transaction under one
  backup.
- **In the app:**
  - right-click a keyword in the sidebar tree for Rename…, Join or replace…, Group
    under…, Move to the top level and Ungroup;
  - the dialog takes more keywords with completion, counts the photos carrying them,
    and says when the target is new;
  - Ctrl+Z undoes a tree change like any edit;
  - the Write dialog lists keyword changes first;
  - a search bar above the Keywords tree shows only matching keywords, with the
    branches above them opened.
- **Tests:**
  - five write tests: rename keeps the intervals, join, group then ungroup, an
    ungrouped group still in use stays, name swap;
  - a blocked-remap test;
  - three library tests: rename through sync and write, undoing a join, the
    grouped/ungrouped keyword list;
  - in the live app: search, rename, group, the Write dialog, two undos;
  - end to end on a catalog copy: rename, then a removal of the renamed keyword, a
    group under a new parent, and an add by the new path. Written in one go, the
    removal followed the keyword through both changes, the tree has
    drinks › cola drink, its intervals are contiguous, and `integrity_check` is ok.
- **Seen in passing:** 159 photos carry keywords only from their file (the default and
  combined layers, not the adjustment layer). The app reads the adjustment layer, as
  the Python tool does, so it does not show them, and keyword changes leave them.

### Removing a keyword takes the keywords under it

- **What was wrong:** reported on photo 64249 (`animal`, `animal|cat`, `animal|dog`,
  `street`). Removing `animal` and `animal|cat` left `animal|dog`, because a removal
  took only that exact keyword.
- **Now:** removing a keyword from photos removes everything under it as well. The
  library splits the removal into one queued removal per keyword the photo actually
  has, so undo restores exactly those, and the write to Capture One is still the exact
  per-keyword removal the differential test covers.
- The panel's × says when it will take keywords under it too.

### Tile size

- The header sets the grid's tiles: a slider (about that wide; the count per row follows
  the window), a per-row box (that many whatever the window), and 3:2 or square.
  Ctrl+wheel and Ctrl+plus/minus step one per row; Ctrl+0 resets. Remembered.
- Rescaling keeps the focused photo where it was on screen, or else the top row at the
  top, over any number of steps.
- Tiles wider than the 450 px thumbnail load the preview for what is on screen once
  scrolling stops.

### Location, GPS and caption fields; copy and paste between photos

- **Writing** (`source-c1/src/write.rs`), as the Python `cmd_meta.py` does:
  - location (`ZIMAGE_CITY`, `_STATE`, `_COUNTRY`, `_ISOCOUNTRYCODE`, `_LOCATION`) and
    caption (`ZCONTENT_DESCRIPTION`, `ZCONTENT_HEADLINE`, `ZSTATUS_TITLE`,
    `ZCONTACT_CREATOR`, `ZSTATUS_COPYRIGHTNOTICE`) go to the adjustment and combined
    metadata rows; only the columns that change are written, and a cleared field is NULL;
  - GPS goes to `ZIMAGE.ZGPSLATITUDE/LONGITUDE` as `50.0614N`/`19.9366E`, the shortest
    decimal that reads back as the same value. It is per image: every variant of it
    moves. Altitude is not stored.
- **Conflicts:** GPS compares by position only, so a position typed here reads back from
  Capture One as done, not as a conflict.
- **CLI:** `c1cat describe --description/--headline/--title`, `c1cat iptc
  --set-city/--set-state/--set-country/--set-country-code/--set-location/--set-creator/
  --set-copyright` (empty clears), `c1cat geo --lat --lon | --clear`.
- **Differential test:** ten more cases (set, clear, accented names, GPS over decimal and
  camera-written `50,2.25N` values, whole degrees, clearing): SAME as Python.
- **In the app:**
  - the details panel's Location fields (country, code, state, city, location, GPS) are
    typed in place; Enter or leaving the field saves, Escape puts it back. GPS takes
    `50.0614, 19.9366`, N/S/E/W letters or degrees-minutes-seconds. With several photos
    selected, a field typed applies to all of them;
  - **Copy…** (Ctrl+Shift+C) opens a list of the photo's fields with their values; tick
    what to copy (keywords: add to theirs, or replace theirs). **Paste** (Ctrl+Shift+V)
    puts them on the selected photos as one undoable edit, queued like any other. The
    clipboard holds the photo as it was when copied, survives restarts, and copying an
    empty field clears it on the targets (the dialog says so).
- **Tests:** four write tests, a GPS format round trip, three paste and typing tests in
  the app crate. In a second app instance on a catalog copy: typed fields, bad GPS, copy,
  paste to two photos, undo, redo, then a write checked in the copy's database.

### Filter summary and date filters (issues #1, #2)

- **Filter summary:** a bar of chips above the grid, one per active filter value, each
  with its ×, and Clear filters at its end. Only there while something is filtered.
- **Date** (second in the sidebar, under Albums): a tree of years › months › weeks with
  counts. A week is the days of one ISO week inside one month (`Dim::Week`,
  `2025-01-W05`), so a week across two months shows under each with its own days and
  the weeks add up to the month. Picking a node opens it; picking it again goes up a
  level. The tree counts as if no date were picked, so all of it stays reachable.
- **Custom range…** opens From and To (each optional) with "Last 7/30/90/365 days";
  Apply sets `captured`. A range and a tree pick replace each other.
- Index schema 4 (the week facet): rebuilt from the store on the first start, no resync.

### Opening a chosen source; providers

- **Providers** (`c1model::provider`): a `Provider` is a kind of source. It has an id
  (`capture-one`), says how a location is picked (a file of some types, or a folder),
  recognises a location, gives a stable key for the app's data about it, and opens it
  as a `PhotoSource`. The app holds a `SourceSpec { provider, location }` and the list
  of providers it was built with (`app/src-tauri/src/sources.rs`); another kind of
  source is one more provider there, nothing else in the app changes.
- **Capture One** (`source_c1::CaptureOneProvider`): a catalog is its `.cocatalogdb`
  file or the folder holding exactly one. Its data key is the one used before, so
  existing libraries, queues and backups carry over (a test pins `fiuji-ba047789`).
- **Opening:** `c1studio <path> [--provider <id>]`; else `$C1_CATALOG` (scripts, test
  instances: not remembered); else the source opened last; else a provider's usual
  place. In the app, the header's source menu shows what is open, recent sources (each
  can be forgotten), "Capture One catalog…" (system file dialog, `tauri-plugin-dialog`)
  and a typed path. Switching opens the new source's own library and settings, then the
  window reloads. Recent sources: `%APPDATA%\c1studio\sources.json`.
- **Tested** in a separate build with its own data and settings: a path on the command
  line, a typed folder, a bad path, back through Recent, and a restart reopening the
  last source.

### Older Capture One catalogs: read, and say which version

- **Version:** `ZVERSIONINFO` keeps a row per Capture One that opened the catalog
  (`ZAUTHOR` "16.5.5.2701 Pro Win", `ZVERSION` 1650). The newest row is the catalog now;
  the oldest says what made it. `source-c1/src/compat.rs` turns that into "Capture One
  16.5.5 Pro (Windows), catalog schema 1650 (created with 16.4.5)", re-read on each
  full read since a newer Capture One upgrades a catalog in place.
- **What is checked:** schema 160800 (16.8) read and written; 1650 (16.5, the
  `other` catalog) read, writing off. Any other schema is tried for reading and
  marked untested, writing off. 1650 lacks `ZRETOUCHINGLAYER`: a missing table now
  means no retouching layers.
- **`Compatibility`** (c1model, in `SourceInfo`): format, read and write support
  (`Tested`/`Untested`/`Off`) and a note, so any provider can say the same.
- **In the app:** a footer badge ("read-only · 16.5", "untested version · 15.3") with the
  details on hover and in the source menu; the write dialog and autosave say why a
  catalog is not written. A read that fails names the version and the versions the app
  reads, shown in full in the empty grid ("sync failed" in the footer). Why the last
  sync failed is kept in the session, so the window shows it however late it asks.

### Editable IPTC fields

- **Fields** (the set the Python oracle writes, so the differential test checks each):

  | In the panel | Capture One section | Column |
  |---|---|---|
  | Headline | IPTC – Content | `ZCONTENT_HEADLINE` |
  | Description (several lines) | IPTC – Content | `ZCONTENT_DESCRIPTION` |
  | Title (IPTC Object Name) | IPTC – Image | `ZSTATUS_TITLE` |
  | Job ID | IPTC – Status | `ZSTATUS_JOBIDENTIFIER` |
  | Instructions | IPTC – Status | `ZSTATUS_INSTRUCTIONS` |
  | Source | IPTC – Status | `ZSTATUS_SOURCE` |
  | Copyright | IPTC – Status | `ZSTATUS_COPYRIGHTNOTICE` |
  | Creator | IPTC – Contact | `ZCONTACT_CREATOR` |

  Title is IPTC's Object Name, a short formal identifier, not a caption; Capture One
  shows it under IPTC – Image, and so does the panel, with that said on hover.
- **Model:** `Iptc` gains `job_id`, `instructions`, `source` (`#[serde(default)]`, so
  queued edits still load); `MODEL_VERSION` 3, so the library re-reads the catalog
  once.
- **Writing** as before: only changed columns, adjustment and combined rows, NULL when
  cleared. CLI `c1cat iptc --set-job-id/--set-instructions/--set-source`. The
  differential test adds Title, Headline, a two-line Description and the status
  fields, set and cleared: SAME.
- **Panel:** Location stays its own section (Country, Code, State, City, Location,
  GPS); below it IPTC – Content, Image, Status and Contact, typed in place like
  Location. Description is a text area: Enter adds a line, Ctrl+Enter or leaving
  saves. Several photos selected: a field typed applies to all. Copy/paste offers the
  same groups.

### Location tree and ISO country codes

- **Tree:** `Dim::PlaceTree` (index schema 5) holds `Country|State|City|Location` and
  every level above it, a missing level as `—` so the levels stay strict (`Poland|—|
  Kraków`: no state). Filter `place_under`; the sidebar's Location section replaces
  the flat Country list, `(no state)` and the like for missing levels.
- **Country ↔ ISO code** (`app/src/lib/countries.ts`): names from the browser's ICU
  region names in English, Polish and the user's language (retired codes such as DD
  left out), plus aliases (USA, UK, England, Hong Kong…). Typing a country fills its
  code, clearing it clears the code, an unknown name clears a code that belonged to the
  old country; a code typed with no country fills the country. Codes are never learned
  from the catalog, whose own pairs include China/HK and UK.
- **Fill in:** flag `no_country_code` counts photos with a country but no code; the
  dialog lists their countries with the code each name gives and sets them as one edit
  (`set_country_codes`). On a copy of fuji: 7,857 photos filled and written (23 s,
  debug build), none left without a code, both metadata rows, integrity ok.

### Renaming and joining places

- Right-click in the Location tree: **Rename…** / **Join into…** (**Set a name…** on a
  missing level). One primitive, `move_place(from, to, code)`: every photo at or under
  `from` takes `to`'s levels (same depth), keeping the levels below; queued as
  `SetPlace` edits, one undo. A country renamed to a known name takes its ISO code.
- Join lists the places at the same level from the whole catalog, names equal but for
  case and accents first (Krakow / Kraków / krakow), then siblings, then the most used.
- No new catalog writes: `SetPlace` is the path the differential test covers. On a copy
  of fuji: Krakow joined Kraków, USA renamed into United States (US, 3,095 photos),
  hong-kong → Hong Kong and undone; written, integrity ok.

### Several photos: shared keywords, fill and sync

- **Keywords of a selection:** with several photos picked, the panel lists every
  keyword any of them has (`keyword_counts`). One on only some is dimmed with a count
  (3/5) and a + that adds it to all; *Add to all* does so for each such keyword; ×
  removes from all. Read 80 ms after the selection settles, so Shift+arrows stay quick.
- **Fill** (Ctrl+Alt+V, or *Fill* next to *Paste*): `paste_ops(.., fill)` only sets
  fields the target leaves empty and adds the keywords it lacks; an empty source never
  clears, and *replace theirs* turns into an add. A unit test pins it.
- **Sync to N…** (Ctrl+Shift+S): the Copy dialog, opened on the focused photo, puts the
  ticked fields straight onto the rest of the selection, *Overwrite* or *Fill*
  (remembered). The choice also lands on the clipboard.
- **Range by keyboard:** Shift with arrows, Page Up/Down, Home and End extends the
  selection like Shift+click.

### Albums and smart albums

- **Two kinds, as in Capture One:** an *album* holds the photos put into it
  (`ZVARIANTINCOLLECTION` and `ZIMAGEINCOLLECTION` links); a *smart album* holds none,
  only its rule (`ZSMARTCRITERIA`). Only albums take photos: putting one into a smart
  album, a group or a project is refused (`membership_problem`), in the library, the
  plan and the write.
- **Model:** `CollectionChange` (c1model): `Create` (album, smart album with its rule,
  or group; in a group or project or at the top level), `Rename`, `SetRule` (smart
  albums), `Delete` (with everything inside). A new collection's id is
  `new:<uuid>`; the uuid is written to `ZUUID`, so later changes and photo edits in the
  same write find it, and the next sync brings Capture One's own id. `ApplyPlan.collections`
  are written after the keyword tree and before photo edits. `PhotoSource::plan_after`
  takes them; `check_collection` says why one can no longer be written.
  `CollectionChange::apply_to` is the reference behaviour (the in-memory source
  follows it, giving made albums ids of its own as a real source would).
- **Capture One's own collections:** the root names Recent Captures, Recent Imports,
  All Images and the Trash. `Collection.system` marks them (`MODEL_VERSION` 4, so the
  library re-reads once): never renamed, deleted or given albums from the app. The
  albums Capture One makes in Recent Imports are the user's. Projects are not deleted
  here.
- **Writing** (`source-c1/src/albums.rs`, ported from Python's `collections.py`): rows
  as Python makes them (`ZPROJECT` from the parent, `ZCOLLECTIONINDEX` after the
  siblings, the default search and filter blobs byte for byte); smart rules in Capture
  One's layout (`smart::criteria`, CRLF and the disabled `ANY` block, read back equal),
  `Rating {min, max}` as *is* or *at least*; membership album by album, refused if it
  would leave more links that do not match. Deleting also removes what pointed at the
  collection: `ZSELECTEDVARIANTS`, `ZCOMPAREVARIANTS`, and the document's selected
  collection (moved to All Images). The Python `delete` did not, and cleared
  `ZPRIMARYVARIANT` where it held the *collection's* pk (it holds a variant): both
  fixed in the oracle too.
- **Library:** album changes queue in `edits.redb` (table `collections`) and show at
  once (`Library::collections` lays them over the store; smart rules follow). Deleting
  an album drops the queued edits that put photos into it, kept with the change so
  undoing it brings them back. Undoing a new album takes what was queued for it since:
  changes to it, albums made in it, photos put into it. Photo membership edits queue
  per album (`collection:<id>`) like keywords.
- **CLI:** `album create|rename|delete|add|remove`, `group create`, `smart create|rule`,
  named and flagged as the Python ones.
- **In the app:**
  - the + beside Albums and a right-click on an album: New album / smart album /
    group, Put the N selected photos here, Edit rule… (match all or any of *Keyword
    contains*, *Rating is*, *Rating is at least*, *Captured in year*: the conditions
    every smart album in the fuji catalog uses), Rename…, Delete… (lists what goes);
  - the details panel's Albums: × takes photos out, *Put in an album* completes from
    the albums, *New album…* can take the selected photos; with several selected, an
    album holding only some shows 3/5 and + puts the rest in;
  - queued albums show in blue; the Write dialog lists album changes before photo
    edits, with *Drop these* for ones that can no longer be written; Ctrl+Z / Ctrl+Y.
- **Tests:**
  - model: changes and paths, what is refused, membership queue keys;
  - write level (run with Capture One open): albums made, filled and emptied, a group
    deleted with its album, links and selected collection; Capture One's own left alone;
  - source level (Capture One closed): the same through `PhotoSource`;
  - library: made here, filled, a smart album by its rule, written and back under
    the source's ids; delete and undo; undoing a new album takes its queue.
  - differential: `writetest.py` adds groups, albums, filling, emptying, `--create`,
    rename, two smart albums (all and any, `&` escaped), adding to and taking from an
    album Capture One made, and deleting a group with an album in it. ZCOLLECTION:
    SAME. The link tables were compared by rowid and differed only in insertion order
    (Python inserts in its selector's order, Rust album by album); they are now
    compared as sets, since nothing points at a link row's `Z_PK`. On a copy of fuji:
    SAME (111 changes, every table).
  - end to end on a copy of fuji in a second instance: a new album filled with three
    photos from the panel, a smart album *street* ∧ ★★★★+ (5,086 photos, the same as
    Capture One's own "street 4+"), a smart album refusing a photo, rename with undo
    and redo, written (own ids #907 and #908 after the sync), then a photo taken out
    and the smart album deleted, written; the album row matches Capture One's fields
    and `integrity_check` is ok.

### Backups and restore

- **Contract:** `PhotoSource::backups()` (newest first) and `restore(id)`, which
  returns the backup of what it replaced; `BackupInfo` (id, path, time, size, note).
  Defaults: none, and restore unsupported.
- **Capture One** (`write::restore`): the backup is checked first (`quick_check`, and the
  schema this build writes; anything else is refused before the catalog changes). Then
  the catalog as it is now is backed up *fresh* (a backup only covering an earlier
  state, which ordinary writes may share, does not do), noted *before restoring the
  backup of …*, and never prunes the backup being restored. The backup is copied over
  the catalog with SQLite's backup API (`Connection::restore`), and the result checked.
  Refused like a write while Capture One or anything else holds the catalog.
- **App:** *Backups…* in the source menu: the list (when, how long ago, size, note) and
  *Restore…* with a confirmation that says what is undone and that it can be undone.
  After a restore the catalog is read in; queued edits stay queued.
- **CLI:** `c1cat backups [--json]`, `c1cat restore BACKUP [--apply]`.
- **Tests:** restore and undo of it (keeping one backup, the one restored survives the
  backup made first); a backup of another schema is refused with the catalog unchanged.
  In the app, on a copy of fuji: restoring the backup from before the albums were
  written took them away, the replaced catalog showed at the top, and restoring it
  brought them back.

### Albums in the left panel, drag and drop, moving

- Asked for (2026-10-11): album management in the left panel, photos added by drag and
  drop, taken out from the photo's album list or the album itself.
- **Left panel:** *+ Album*, *+ Smart album*, *+ Group* under the title (the + menu went),
  and a hint line. The details panel keeps the photo's albums with × (and 3/5 with +
  for several); its *Put in an album* field and *New album…* went.
- **Drag and drop** (`dragDropEnabled: false` in the Tauri window, which otherwise takes
  drops for files on Windows): grid tiles onto an album put them in (the picked photos
  if the tile is picked); an album or group onto a group or project moves it, onto
  the Albums title to the top level. Drop targets light up.
- **Delete** while the grid shows an album takes the picked photos out of it.
- **Moving:** `CollectionChange::Move` (model: not into itself or what is under it, not
  where it is, not a project or a group holding one, not Capture One's own). Capture One:
  `ZPARENT`, `ZCOLLECTIONINDEX` after the new siblings, and `ZPROJECT` set again for it
  and everything under it (into a project and out tested). Menu: *Move to…* (a dialog
  with the places it can go) and *Move to the top level*.
- **Fixed:** the album menu's *Put the N selected photos here* (and the new *Move to the
  top level*) did nothing: the menu was closed before the album was read, and Svelte 5's
  `{@const}` then read an empty menu.

### Copy and paste like Capture One

- Reported: "copying properties between photos does not work". The paste itself worked
  (checked in the sandbox with real keys and clicks); what failed was silence. A paste
  onto photos already holding the values changed nothing and said nothing, and Ctrl+C /
  Ctrl+V did nothing (only Ctrl+Shift+C / V did).
- **Tiles** show ↑ (copy at once) and ↓ (Paste from …) on hover, as Capture One's do.
- **Paste…** (↓, the panel, Ctrl+Shift+V) chooses the fields and **Overwrite** or
  **Merge** (was "Fill": empty fields filled, keywords added); the choice is
  remembered for Ctrl+V. Ctrl+C copies, Ctrl+V pastes, Ctrl+Alt+V merges.
- The footer says what a paste did ("Pasted onto 3 photos") or that nothing changed.
- **Tested** in the sandbox on the fuji copy: ↑ and ↓, the paste dialog with Merge,
  two photos dragged onto an album, an album dragged into a group and moved back from
  the menu, Delete in an album; all written, `integrity_check` ok.

- **Changed after:** the user wanted one big button that copies from a photo into the
  selected ones, not tile buttons. The tile ↑/↓ went; **⇉ Copy to N selected** in the
  header (shown with several picked) opens the field choice with Overwrite or Merge
  and puts the focused photo's fields on the rest (what *Sync to N…* did, renamed
  *Copy to N…* in the panel too).

## Stage 3: local analysis

### Measuring photos

- **`crates/c1analysis`** (pure functions over pixels, `image` only): from a render at
  most 512 px long, which Capture One's thumbnails about are:
  - shape (width, height of the render: crop and rotation included);
  - main colours: k-means (k 5) in CIE Lab over a 64 px copy, started from pixels
    spread through the lightness range so the same picture gives the same palette;
    mean Lab chroma for black and white;
  - tones: mean and spread of luminance, shares clipped at either end;
  - sharpness: the variance of the Laplacian in the best of a 4×4 grid of tiles, so a
    sharp subject on a soft background counts;
  - a difference hash (64 bits).
  Classes (`shape`, `hues`, `light`, `focus`) are made from these at index time;
  `VERSION` re-measures, `CLASSES` only rebuilds the index.
- **Groups** (`groups.rs`): duplicates are different images (not variants of one) taken
  at the same instant whose hashes are at most 3 bits apart (found through four 16-bit
  bands; featureless hashes, near all 0 or all 1, are left out). Series: one camera,
  each photo within 4 s of the one before and at most 22 bits from it, three or more.
- **Library:** results in `analysis.redb` (by photo key, with the render's size and
  modification time, so a photo is measured again only when Capture One renders it
  again; kept apart from the store, which is dropped when its format changes). In
  memory with each photo's groups (`dup:<row>`, `series:<row>`, named after the
  group's first row); every document is built with them. New dimensions `shape`,
  `hue`, `light`, `focus`, `group`; flags `analysed`, `duplicate`, `series`; fast
  fields `sharpness`, `brightness`. Filter fields to match, and a sharpness range.
- **Runner** (`c1index::analyse::run`): stats each photo's thumbnail, measures the ones
  that changed in batches of 1000 across threads, commits each batch, then regroups.
  The app runs it after every sync on half the cores (footer: *measuring photos N /
  M*), stops it when another catalog opens; `c1cat analyse [--threads] [--stats]`.

### Tuned on the fuji catalog

- **Speed:** 95,704 thumbnails (all there are) in 4 min 9 s on 11 threads, 2.6 ms a
  photo, none unreadable. Release build; dev builds optimise `c1analysis` too.
- **First thresholds were wrong for this catalog** (`--stats` and contact sheets drawn
  in the app):
  - focus: grain and JPEG noise lift the Laplacian everywhere (median 4,141); 99.6%
    were "sharp". Looked at by eye, below ~300 is motion-blurred or missed, up to
    ~700 soft or low in detail (fog): blurry < 300 (1.6%), soft < 700 (3.8%);
  - light: a low-key catalog (median brightness 0.31); dark < 0.2, bright > 0.55;
  - colour: black and grey dominated the colour photos; colour names now leave out
    black, white and grey (*muted* when no colour shows);
  - duplicates: "alike within 3 bits" chained unrelated dark frames into one group of
    100+, and caught burst frames; requiring the same instant leaves real ones
    (1,629 photos, checked on a contact sheet: the same frame twice).
- **Result:** black & white 40,290; landscape 92,041, portrait 3,660; series 14,718.
- **Tests:** shapes, colours and their names, black and white, light, sharp and soft
  (and a sharp patch on a plain background), hashes of alike pictures, duplicates
  (not variants, not burst frames), series; library: filters, groups by regroup,
  analyses outliving the store.

## Stage 4: the map

- **`c1index::geo`:** `Library::map(filter, view)` takes the photos the filter selects
  that have a position (lat, lon, rating, sharpness from fast fields), and clusters
  those in the view in Web Mercator pixels: 72 px cells laid on the whole world, so a
  marker keeps its photos while the map pans; from zoom 17 only photos at the same spot
  (about 1 m) share a marker. A cluster carries its centre, count, best photo (rating,
  then sharpness) and bounds. A filter's own `within` is set aside, so the map keeps
  showing the places around the area it picked. Tauri command `map`.
- **App:** `MapPane.svelte` (MapLibre GL 6, OpenFreeMap positron turned dark by a CSS
  filter on the canvas only, HTML markers with thumbnails: discs below zoom 5). Header
  *Map* toggle; *grid follows map* sets `filter.within` to the view (debounced), chip
  *Map: area in view*. Single markers are draggable and photos can be dropped from the
  grid onto the map: both are `Edit::Geo` through the normal queue. The view is
  remembered. CSP allows tiles.openfreemap.org and workers; the MapLibre worker is
  bundled with Vite (`?worker&url`).
- **Tested** on a fuji backup copy: 809 photos with a position, clusters at 3 zooms,
  click-to-zoom, grid following, marker drag and drop from the grid queued `SetGeo`.
  Unit tests for clustering and `Library::map`.

- **Tabs** (the user's request): *Photos | Map | Analysis* in the header, one filter for
  all. Map = full map with a 190 px strip of the grid under it (the strip is where photos
  are dragged from); leaving the map tab keeps the area, so it can be looked at in
  Photos. Analysis (`AnalysisPane.svelte`) replaces the sidebar's Look section: measuring
  progress, bar charts of shape / colour / light / focus (click to filter), and the
  duplicate groups and series as rows of thumbnails (`Library::groups`, command
  `groups`; click shows the group). The tab is remembered. (The viewer's `.stage` CSS
  class once collided with a layout wrapper of the same name: the wrapper is `.pane`.)

- **Box select** (Shift+drag on the map → `filter.within`, grid stops following), **place
  search** (Nominatim from the page; CSP allows it; *Put N picked photos here*), **GPX
  import** (`c1index::gpx`: hand-scanned trkpt/rtept with times, interpolation within a
  gap; commands `pick_gpx`, `gpx_plan`, `gpx_apply`; `GpxDialog.svelte`). The camera
  clock's UTC offset is asked for because Capture One keeps camera time. Tested with a
  generated track on the fuji copy: offset +2 placed 78 photos, offset 0 none, origin
  Gpx, undo restored the queue.

- **Fixed:** leaving the Map tab while its clusters were still being counted left
  `TypeError: Cannot read properties of undefined (reading 'getCanvasContainer')` in the
  footer. The answer came after the map was removed and its markers were added to no map.
  Closing the pane now drops the answers still on the way (clusters and the route).

### Previews take Capture One's colours, fitted per photo

The tone curve match above left saturation, film simulations and colour edits behind.
The `.cop` is already cropped and straightened, so once rotated back it frames the same
scene as the `.cot`; shrunk to 450 px, every pixel pairs an unedited colour with
Capture One's finished one. `c1image::Lut` fits a 17³ table over 8-bit sRGB to those
pairs (`lut.rs`):

- least squares on the trilinear weights, plus smoothness that asks neighbouring nodes
  to differ as the best single colour matrix would, so colours the photo lacks follow
  that matrix instead of staying flat; solved by conjugate gradients, one thread per
  channel;
- then fitted again with each pixel weighted by 1 / (1 + (residual / 12)²), so edges and
  stale-thumbnail areas count less; thumbnail black where the preview has a picture is
  left out altogether.

The table learns the black-and-white channel mix too, so it starts from the colour
render and greys the result. When the two shapes differ by more than 2% (a thumbnail
from before a crop) or too few pixels pair, the tone curve match is used as before.
`render_preview` takes `Matching::{None, Tones, Colours}`; `load` uses `Colours`.

`lookcheck` now fits on half of each thumbnail (a checkerboard of 16 px blocks) and
scores every method on the other half (120 random photos, median, out of 255):

| | plain | + b&w | + b&w + tones | colour table |
|---|---|---|---|---|
| black-and-white (53) | 34.3 | 32.6 | 9.2 | 5.6 |
| colour (67) | 29.0 | 29.0 | 7.9 | 5.1 |

Smoothness 0.005–0.1 moves these by 0.1 at most. Render time is unchanged (~320 ms for
both): the fit takes 30–50 ms, the 450 px box shrink ~5 ms and the table ~10 ms over
2560 px.

**Against Capture One's own exports** (`exportcheck <folder>`: 22 edited photos the user
exported as 2560 px sRGB JPEGs; median absolute difference per channel at 2560 px):

| | thumbnail, upscaled | tones | colour table |
|---|---|---|---|
| all 22 | 11.2 | 12.6 | 10.1 |
| 15, without the 7 whose geometry differs (every render 20+ off) | 5.5 | 8.2 | 4.4 |

`exportcheck` finds each export's photo among those sharing its file name (up to 14) by
the closest thumbnail, and finds the export inside each render by scale and offset.

- **Framing.** 12 of the 22 exports cover only 82–98% of the cached renders (both
  `.cot` and `.cop` are framed alike there), so a crop made in Capture One does not
  always reach its cache. Some photos also have a perspective (keystone) fix that the
  thumbnail shows, with black wedges, and the `.cop` lacks. The viewer shows these
  photos uncropped and uncorrected; no colour work changes that.
- **Fits on frames that do not line up** taught the table wrong colours (one export got
  worse, 29 → 36). `Lut::residual` is the median distance of the fitted pairs; over 120
  random photos it is 5.7 at the median and 13 at the 98th percentile, and the two bad
  fits sat at 45 and 51. Above 25 the tone curve match is used instead (~1% of photos).
- Local adjustments (masks, gradients, vignetting) and detail finer than the thumbnail
  are still not carried over.

### Framing: straightening and crop

The exports showed the viewer framing photos unlike Capture One. Over 600 sampled
variants (400 cropped):

- **`.cop` previews are the whole sensor image**, with no crop, straightening or
  perspective fix: every one has the sensor's shape. (The note above that they come
  cropped and straightened held for one sample only.)
- **`.cot` thumbnails are straightened and perspective-fixed, on a canvas grown to hold
  the turn** (a 3:2 frame turned 1.67° gives 450×307), **but not cropped**.
- **`ZCROP`** is `centreX;centreY;width;height` in sensor pixels: the centre in the
  sensor's orientation, the size in the displayed one (a portrait crop of a turned
  photo only fits that way round). It is applied after the turn and straightening.
- **Straightening without a crop** shows the largest frame of the picture's shape that
  the turned picture fills: 1 / (long/short · sin θ + cos θ) of it, 96% at 1.43°.

So `render_preview` now turns the preview by the full `ZROTATION` onto a grown canvas
(`geometry::straighten`, bilinear), which frames it like the thumbnail, fits colours
there, and `c1image::frame` cuts the crop (`ImageRef::{straighten, crop}`, `c1model::Crop`
as offsets and sizes in fractions of the picture). Thumbnails get the same crop measured
on their grown canvas (`config::on_canvas`); `source-c1` reads all of them in one query
and again when the catalog file changes, and the app cuts and re-encodes only the
thumbnails that have one.

`exportcheck` gained an *as shown* column: the framed preview against the export with
no search. On the 22 exports its median is 9.6, as good as the searched-for match
(9.6, from 10.1 before straightening). Straightening fixed most of the worst: DSCF1893
34 → 15, DSCF5614 23 → 15, untitled-1506 40 → 20.

Still not done:

- **Perspective (`ZGEOMETRYKEYSTONE*`).** DSCF5376_1 (tilt 21) and untitled-0245-2 (tilt
  11) stay 28–32 off; the thumbnail has the fix, so the colour fit falls back for them.
- **Lens distortion correction** (`ZLENSDISTORTION`: `radius,shift;…|amount|…`, on when
  the amount is 100) zooms the picture a few percent: untitled-0395-3 is 40 off.
- **Portrait crops** follow the rule above but no export checked them yet.

### Catalog history: the change log

Time travel needs to know what changed and when. History builds up only from the day it
is recorded, so it is recorded before its views. The log is the `events` table in
`edits.redb`, which a `MODEL_VERSION` bump never wipes. Values are JSON, so entries
written before the kinds below existed still read.

- **`c1index::Event { at, origin, what }`.** `Origin` is `App`, `Source`, `Gpx` or
  `Restore` (a backup put back from the app). `What` is one of:
  - `Change(Change)`: one field of one photo;
  - `Added` / `Removed`: the photo id and its file name, so the entry still reads after the
    photo is gone;
  - `Collection { change, label }`: an album made, renamed, moved, given a new rule or
    deleted;
  - `Remap(KeywordRemap)`: a keyword tree change;
  - `Written { photos, changes, albums, keywords }`: a write reached the source. Edits the
    source already held are not counted; nothing written, nothing logged.
  - `Undone { label }`: a queued album or keyword tree change taken back.
- **Per photo:** the multimap table `events_by_photo` (photo key → sequence numbers).
  `Library::photo_events` reads it. A `meta` entry says how far the log has been looked
  up, so entries added by a build from before the lookup are picked up on the next open.
  An entry this build cannot read (a newer kind) is skipped with a line on stderr; it does
  not stop the library opening.
- **Album names:** the table `album_names` keeps each album's path as last seen. The
  History section uses it for albums deleted since, and for albums made here whose app id
  the source has replaced.
- **Sync logs Capture One's side:**
  - `c1model::diff` now covers IPTC, place and GPS. A position counts as changed only when
    it moves.
  - Photos added and removed are logged.
  - `collection_diff` compares the albums with the stored list.
  - The first fill of a source logs none of this; `imported_at` dates those photos.
  - What the app did is not credited to Capture One:
    - album changes the app wrote are kept in the table `written_collections` until a sync
      that saw the source's albums commits, so a failed sync or a restart in between does
      not lose them. Each matches once. A new album is matched by name, kind and parent,
      with app ids read as the source's; albums made inside albums made in the same write
      are matched once their parent is;
    - photos are not logged as moving from the app's album id to the source's, nor as
      leaving an album the app deleted (or anything under it);
    - photos re-read after a keyword tree change was undone (fingerprint 0) are logged as
      `App`;
    - `Library::sync_as(source, origin)`: restoring a backup syncs first as `Source`, then
      reads the restored catalog back as `Restore`, holding the sync lock throughout.
  - The source is read before the edit lock is taken (a separate lock keeps syncs one at a
    time), so a refresh reading a changed catalog does not hold up edits for seconds.
- **App side:**
  - `Library::edit_as(changes, origin)`; GPX placements pass `Gpx`.
  - Album and keyword tree changes are logged when queued, and again when undone.
  - `Library::log_written` after a write; the write holds the sync lock until it is read
    back.
- **Noticed sooner:** the app reads Capture One again when its window comes back into
  focus (at most every 10 s) and every 5 minutes. An unchanged catalog costs a file stat.
  A refresh is skipped while another sync or a write holds the lock (a poisoned lock is
  taken over). `SyncReport::changed` is false for a full read in which nothing differed:
  Capture One touches its file often. Thumbnails and analysis are looked at again only
  when something changed.
- **First view:** a collapsible **History** section at the bottom of the details panel. It
  shows the date, who made the change (here / Capture One / GPX / backup) and a one-line
  description (`eventText` in `lib/format.ts`). Commands `photo_history(row)` (entries and
  album names) and `change_log(limit)`. After a sync the open section keeps its list until
  the new one is in; only the newest read is shown, and a failed read says so. The details
  panel also reads the photo again after a sync that changed something; before, it kept
  showing the old values.
- **Review fixes** (PR #8): each is covered by a test in `crates/c1index/tests/library.rs`:
  an album deleted here, albums written then a restart, two new albums of one name, a
  restore, a touched source with nothing new, captions and places written and read back.
  `edits.rs` tests old-build and unreadable entries.
- **Tested** with a second instance on a copy of the newest fuji backup:
  - the first fill of 99k photos logged nothing;
  - a rating set here showed as *here*;
  - a caption and colour set with `c1cat` on the copy showed as *Capture One* after the
    5-minute refresh;
  - writing the queue logged one `Written` entry and nothing else.

  The focus refresh was not driven: it would have taken the user's focus.
- **Known limit:** what Capture One changed between the last sync and a store wipe shows
  up as a fresh state, not as changes.

### Settings: sidebar sections, main colour, offline map

- **Settings dialog** (⚙ in the header, or right-click a sidebar section's title):
  - the sidebar's sections can be hidden and put in any order (drag, or ↑↓); *Reset
    sidebar* brings back the default. Right-click a title for *Hide this section*;
  - *Use online map services* (on by default).
  - Kept in the webview's storage under `settings` (`lib/settings.svelte.ts`), the same for
    every catalog. `lib/sections.ts` merges a stored list with sections added since: a new
    one goes after the section it follows by default.
- **Hidden sections cost nothing:** a query counts only the groups something shows
  (`SECTION_DIMS`, plus those the analysis tab and the Location fill-in need). Smart albums
  are not counted while Albums is hidden. Showing a section again re-runs the query. A
  filter set from a section that is then hidden keeps its chip.
- **Main colour** section: the analysis' colour names (`hue`), with swatches. Picks add up
  (any of them), as in the analysis tab. It shows once photos are measured.
- **Offline map:** with online services off, the map draws a plain ground with a 10° grid
  (made locally) instead of OpenFreeMap's tiles; the place search is hidden; place hints
  in the details panel don't ask Nominatim (`allowPlaceLookups` in `lib/placeHints.ts`).
  Pins, tiles, Shift+drag, dragging photos onto the map and GPX still work. Switching
  changes the map's style in place.

### Trips, the timeline and route replay

- **Detection** (`c1index::trips`): each day with photos is home, away or unknown by its
  photos' places, or by positions near or far from home (the middle of the photos placed
  at home that have one) when they have no place. Days join while the gap between photos
  stays under `gap_hours` (36), or `bridge_days` (4) when the country on both sides is the
  same; a day at home ends a stretch. A stretch with a day away (and at least `min_photos`)
  is `Found`; one with no place at all, `suggest_days`/`suggest_photos` (2, 100) or more,
  is `Suggested`. Time decides membership: a trip is a span from its first photo to its
  last, and `Filter::trip` picks that span (kept apart from `captured`, so days picked in
  a trip narrow it).
- **Home** is set once (`TripSettings::home`, country and optional city). Until then
  nothing is found and `guess_home` offers the city seen on the most days (Kraków on
  fuji).
- **The user's edits stay:** `trips.redb` (beside the store, which a model change wipes)
  holds every trip. `reconcile` keeps `Kept` and `Dismissed` ones, drops findings over
  them, stretches a kept trip over a finding that runs past it alone (photos imported
  later), and replaces untouched trips with the new findings, keeping the id of the one
  each overlaps. `TripEdit`: rename, keep, dismiss, forget, merge, split, create.
- **Library:** `trips(filter)` (worked out again only when the index generation changed;
  counts and covers per trip under the filter), `timeline(filter)` (photos per local day),
  `route(filter)`. Detection reads the index's columns, not the store; `schema::LocalDays`
  asks the time zone once or twice per day, not per photo (3 µs each on Windows, a third
  of a second over the library). On fuji: about 100 ms when the library changed, 40 ms
  for a filter change.
- **Tuned on fuji** with a prototype over all 100k photos, folder names used only to check
  the result: 43 trips and 62 suggestions; every trip folder with places came out as one
  trip (Tokyo + Hong Kong 2025, Sicily, Istanbul, both Berlins), and untagged ones as
  suggestions (Iceland 2022, Lofoten 2025 in two halves). Weekends at home without
  places are suggested too; ✓ and × deal with them. Trips under 10 photos are dropped (a
  few photos with a wrong clock made one-day "trips" to Tokyo in March 2018).
- **App:** a Trips section (by year, covers, counts, quick ✓/× on suggestions, a menu for
  the edits, home and gap settings), a timeline strip above the grid and map (day, week
  or month bars by room; drag or click picks days; trip bands pick trips; a picked trip
  zooms it), and route replay on the map (a line layer drawn in colours chosen for the
  inverted canvas, a moving head with the photo). *Make an album of it* queues a new
  album with the trip's photos. `c1cat trips` lists them.
- **Tested** in a second instance on a copy of the newest fuji backup: home from the
  guess, 101 trips and suggestions, picking Tokyo/Hong Kong (7,423 photos, timeline
  zoomed to it), a merge, an album from a trip (252 photos queued), and the route of
  Belgrade 2017 (6 positions). Only 809 photos on fuji have a position, so most trips
  have no route yet.

### Places without GPS on the map, and placing photos

Only 809 of fuji's 100k photos have a position; about 31k have a place typed (city, region,
country) and 68k have neither.

- **Gazetteer** (`crates/c1geo`): GeoNames (CC BY 4.0) countries, first-level divisions
  and every place of 1000+ people, 165k rows, 3.8 MB gzipped in the binary
  (`data/gazetteer.tsv.gz`, built by `tools/gazetteer/build.py` from the GeoNames dump;
  rerun to refresh). Read on first use (~0.3 s). Names are folded (case, diacritics,
  spacing). `locate(place)`: country by ISO code or any of its names; state by name or a
  shared start (`Malopolska` ↔ `Małopolskie`, `Lower Silesian` ↔ `Lower Silesia`); city in
  that country by any name (`Cracow`, `Tokio`), one typo away (`Istambul`), or typed as
  the location (`Germany|—|—|Berlin`); a whole city before a district (`D` rows: places
  well inside a city ten times bigger). A country's and a state's point is the
  population-weighted median of its cities. `reverse(lat, lon)`: the city whose reach
  (1 + √pop/40 km) covers the point best, else state and country of the nearest within
  60 km. `parse_point`: coordinates as written (`44°58′N 14°45′E`, `N44°58' E14°45'`,
  `44 58 N 14 45 E`, decimal, longitude first); `app/src/lib/coords.ts` is the same parser.
- **Library** (`library/placing.rs`, `places.redb`):
  - `map` also returns photos with only a place, at its position, clustered apart
    (`PlaceCluster`), shown only up to the zoom their level reaches (country 5, state 8,
    city 11, location 14; `geo::max_zoom`), with counts in view, hidden, place-only,
    unresolved and unplaced. Places at one point (a city and spots in it known no better)
    carry their paths. 80 ms on fuji at world zoom.
  - `Filter.placed` (an exists query on `place_tree`, no index change).
  - Online fallback: `unresolved_places` lists places named more finely than the
    gazetteer knows them; the app asks Nominatim (`pointFor`, one request a second, all
    names then fewer) and `set_place_points` keeps the answers, misses included, in
    `places.redb` (it outlives the store).
  - `promote` / `promote_rows`: photos get their place's position (`GeoOrigin::Geocoded`),
    recorded in `places.redb` as coarse, so the map keeps them ≈ and neighbours never take
    them as fixes. Moved by hand, they count as placed (the stored point no longer holds).
  - `fill_plan` / `fill_places`: empty place fields from the position, written in the
    catalog's own spelling (the name the catalog uses most for that gazetteer place:
    `Malopolska`, not `Lesser Poland`); a photo whose place disagrees is left alone.
  - `neighbour_plan` / `neighbour_apply`: positions from GPS photos taken before and after
    (`gpx::locate`, or the nearest within half the gap), places from the place both
    neighbours share. On fuji with 30 min: 397 positions, 10,923 places, 150 ms.
- **App:** dashed ≈ markers (cards hang below their point so GPS pins stay visible); a click
  shows the place's photos in the grid, zooms in, or gives them its position (asked
  first). A tray under the map lists photos with no position (tabs: no location, only a
  place) to drag on or type coordinates for. `Neighbours…` dialog. A footer offer to fill
  places after a placement (Settings: always). Details panel: *Fill place from GPS*,
  *GPS from place ≈*. The place search takes coordinates, online or not. Chips for
  `gps` and `placed`. `c1cat geo --at`.
- **Spelling and speed:** name resolution is remembered per place path; the catalog's
  spellings are built once per call, not per index generation (the background analysis
  bumps it all the time: the first version took 120 s to plan 300 photos).
- **Tested** in a second instance on a copy of the newest fuji backup: world view shows
  26.5k photos by place in 11 ≈ clusters; Kraków at zoom 13 hides them all (2,910), at
  zoom 10 shows them below the GPS pins; 30 places looked up online (Hotel Forum,
  Wasilewskiego found as spots in Kraków); two tray photos placed by typing
  `44°58′N 14°45′E`, offered *Baska, Primorje-Gorski Kotar, Croatia*, filled; Hotel Forum's
  113 photos given its position. Written to the copy (117 edits) and read back: GPS and
  place in `ZIMAGE`/`ZVARIANTMETADATA`, and the 113 still drawn as ≈.

### Feature page and recorded clips

- **`docs/FEATURES.md`:** one section per feature (search, browsing, editing, keyword
  tree, albums, analysis, map, trips and the timeline, writing with history and backups),
  each with a GIF and a link to the guide. The README links to it.
- **`tools/record`:** the clips are recorded by scripts. `start.ps1` runs a second instance
  on a copy of the newest fuji backup (CDP on 9223, a junction to the live `Cache`);
  `drive.mjs` drives it over CDP and records screencast frames with their times, with a
  drawn pointer and captions (`overlay.js`); one scenario per clip in `scenarios/`;
  `assemble.py` makes the GIF with Pillow, `to_mp4.py` an MP4 with ffmpeg from the same
  frames. `record.ps1` runs them all. Scenarios undo their edits; `history` writes to the
  copy.
- Recording found the map's `getCanvasContainer` error after leaving the Map tab quickly
  (fixed separately).

### Map: finding places, the 180° meridian, trips, following (#21, #23, #24, #25)

- **Find a place** (`lib/mapview.ts` `hitView`): the map centres on the place's own point.
  Nominatim's box covers every part of a place (Tokyo's runs to Minamitorishima, France's
  to the Pacific, Fiji's round the world), so it sets the zoom only when the point lies
  in its middle 60 % and it spans less than 180°; else the zoom comes from the place's
  `addresstype` (country 5, region 7, county 9, city 10, village 13, street 16), or its
  `place_rank`.
- **Across 180°:** maplibre keeps longitudes unwrapped after a pan (a centre of 499.77),
  and `view()` clamped west 422 / east 578 to nothing. On `moveend` the centre is
  wrapped back (a jump, not seen) and stored wrapped; a remembered unwrapped one is
  wrapped on start. A view still across 180° sends every longitude (`viewBox`): the
  filter takes one box. Two boxes, or a box with west > east in `filter.rs` and
  `geo.rs`, would keep it to the view.
- **Trips:** an edit no longer fits the route again, moves the playhead to the end and
  stops the playback; only another trip does (`routeStep`).
- **Following:** turning *grid follows map* on, *Clear filters*, or dropping the filter
  that kept the grid off the map applies the map's current view at once (`shouldFollow`,
  in `run()` and an effect), not at the next move. Dropping the *Map: area* chip turns
  following off, or it would come straight back.
- `tools/record/checks/21.mjs`, `23.mjs`, `24.mjs`, `25.mjs` measure each on the recording
  instance.

### Edits no longer blank the grid and the panel (#33, #34)

- **Pages kept through a refresh:** after an edit, an undo or a sync, `run(true)` read
  only the first page again and dropped the rest, so past position 500 the grid's tiles
  and the details panel went blank for 25-100 ms (65 of 70 thumbnails made again, focus
  and the panel's scroll lost). Now the pages on screen and the focused photo's are read
  again into a new map, swapped in with the result in one go (`lib/pages.ts`); the old
  ones show until then. The focused photo's row is kept in its own state (`focusRow`),
  so the panel and the viewer never see a gap.
- **Look stays:** the panel's Look was cleared and read again on every edit (the fields
  below jumped 139 px and back). It is read again only for another photo, or when the
  analysis has measured photos again (`analysis-done`).
- `tools/record/checks/`: scenarios that measure it on the recording instance (33: nodes
  replaced, focus and scroll through edits past position 500; 34: field positions each
  frame). CI runs the app's TypeScript tests (`pnpm test`).
### Analysis: whole groups under a filter, honest empty charts (#27, #32)

- **Groups:** `Library::groups` counted only a group's matching photos and kept groups
  of 1, so with *red* picked one frame of a 7-photo burst showed as a "series" of 1.
  `Group` now has `size` (the whole group, counted as the details panel counts it: one
  facet count over `Dim::Group`) and `matched`; a group with fewer matching than it
  needs (2, `SERIES_MIN` for a series) is left out. The badge says *2 of 7*. The card
  and the panel's link both show the group within the other filters (`showGroup`).
- **Empty charts:** *Nothing matches the filter* when the filter picks nothing, *Not
  measured yet (N photos)* when nothing it picks is measured; the *0 of 0 photos
  measured* line is gone.
- Tested with a burst of four and a duplicate pair split by colour and shape
  (`groups_split_by_a_filter_count_whole_and_drop_what_is_no_group`).
  `tools/record/checks/27.mjs` measures it on the recording instance.

### Analysis: photos in a strip, every group (#26, #28)

- **Photos on the tab:** the Analysis tab unmounted the grid, so a bar's photos showed
  only on *Photos*. The grid now stays as a 190 px strip under the charts and groups, as
  under the map (`.pane.strip`): the same result as the Photos grid, so the counts
  agree; a click shows the photo in the details panel and the arrow keys move through
  it. A group's card and the panel's group link stay on the Analysis tab (the card shown
  is outlined); from another tab the link opens *Photos* as before.
- **Every group:** the lists stopped at 300 groups (*300+ groups*) and every filter
  change drew up to 600 cards of six thumbnails. `Library::groups` takes an offset and
  a limit and returns a `GroupPage` with the exact total (the order ends on the id, so
  pages follow on). The tab reads 60 of each, the header gives the total, and *Show
  more (N left)* adds the next 60; another filter starts again from the first page,
  while an edit or new measurements read as many as are shown, so an opened list stays
  open. *Show more* waits while the lists are read again. Thumbnails load lazily. On the
  Analysis tab the page keys scroll the charts; the arrows move in the strip.
- Tested with three pairs and a three paged two at a time
  (`groups_page_with_an_exact_total`). `tools/record/checks/26-28.mjs` measures it on
  the recording instance.

### Next

1. Time travel views on the change log: state on a date, growth of the collection, compare
   two dates, restore a past state as a set of `Change`s.
2. Trips found by positions alone could be named with `c1geo::reverse`.
3. Stage 4, rest: photo direction, lasso.
