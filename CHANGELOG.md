# Changelog

## 0.2.0 — 2026-10-15

### New

- **Trips:** trips are found from capture times and places and listed in a sidebar section, with a timeline strip and route replay on the map. Your edits to a trip (renames, dismissed spans) are kept (#16).
- **Places by name, offline:** photos without GPS are placed on the map from their location names, using a bundled GeoNames gazetteer. A tray lists photos that are not on the map, and places can be set from neighbours' positions (#17, #18).
- **Settings:** reorder and hide sidebar sections, a Main colour filter, an offline map option (#15).
- **Analysis:** photos in a strip under each chart, and every group shows an exact total (#43).
- **Feature page:** [docs/FEATURES.md](docs/FEATURES.md) has a recorded clip per feature; the clips are recorded by scripts in `tools/record` and are refreshed for this release (#20).

### Improved

- **Batch editing:** a fixed "Edit N photos" block in the details panel, one selection read, feedback after each edit (#41).
- **Edits and undo:** the grid and the details panel stay where they are through edits and undo (#38).
- **Map:** found places are centred, the view wraps across 180°, the trip view is kept, and following the grid is immediate (#39); room for the find box, the tray and the pins (#42); answers that arrive after the map pane closed are dropped (#19).
- **Analysis counts:** whole groups are counted, things that are no group are dropped, empty charts say so (#40).
- **Show in grid** works for photos placed by name, and with the grid following the map (#18).
- Cheaper per-photo place lookups; queued Nominatim requests keep their pace after a wait.

### Install

Run `C1.Studio_0.2.0_x64-setup.exe`. It installs over 0.1.x for the current user and keeps your settings and index.

## 0.1.1 — 2026-10-14

- First start opens nothing: choose a catalog from the source menu (#11).
- Release builds no longer contain file paths from the build machine (#11).
- Installer: publisher, description and Start menu entry; per-user install; fetches WebView2 if missing (#12).

## 0.1.0 — 2026-10-13

First release: search, details, viewer, analysis, map, history and the `c1cat` CLI.
