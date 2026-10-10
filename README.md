# c1-studio

A fast browser and indexer for Capture One catalogs, written in Rust. The plan is in
[docs/PLAN.md](docs/PLAN.md) and the progress log in [docs/PROGRESS.md](docs/PROGRESS.md).

## Layout

```
crates/c1model/    canonical photo model + the PhotoSource contract + conformance checks
crates/source-c1/  Capture One catalog as a PhotoSource (read-only for now)
crates/c1index/    the library: redb store + tantivy index, filters, facet counts, sync
crates/c1image/    display-ready pixels: thumbnails, white-balanced JPEG-XL previews, raw-embedded JPEGs
crates/c1cat/      CLI over the library
app/               the desktop app: Tauri 2 (app/src-tauri) + Svelte 5 (app/src)
legacy/c1cat-py/   the original Python c1cat, kept as the oracle for differential tests
```

## Use

```powershell
cargo build --release -p c1cat
$c1 = '.\target\release\c1cat.exe'
& $c1 sync                                     # first run indexes the whole catalog
& $c1 find --rating '>=4' --keyword street --limit 20
& $c1 count --year 2026 --unkeyworded
& $c1 facets --year 2026 --dims rating,camera,keyword_tree
& $c1 image --tier preview -o out --rating 5 --limit 5   # render previews to JPEG
& $c1 bench                                    # speed gate on 250k synthetic photos
```

The catalog is located from `--catalog`, then `C1_CATALOG`, then
`%APPDATA%\c1studio\config.json` (`{"catalog": "C:\\c1_catalogs\\fuji\\fiuji.cocatalogdb"}`).
Library data lives in `%LOCALAPPDATA%\c1studio\<catalog id>\`.

## App

```powershell
cd app
pnpm install
pnpm tauri dev                          # live-reloading window
pnpm tauri build --debug --no-bundle    # target\debug\c1studio.exe
```

## Test

```powershell
cargo test --workspace
cargo test -p source-c1 --test live -- --ignored   # against the real catalog, read-only
python tools/difftest/difftest.py                   # Rust vs the Python oracle in WSL, same catalog
```
