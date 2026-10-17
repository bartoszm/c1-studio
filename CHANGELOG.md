# Changelog

## Unreleased

- **Analysis tabs:** the Analysis view has two tabs. *Look* holds the shape, colour, light and focus charts, with a **Stack bursts & duplicates** toggle (also in the header) that shows one photo per group, its sharpest. *Alike* holds duplicates and series in one list, sortable (most matching, biggest, newest, oldest, sharpest), with sliders for what counts as a duplicate or a series; the duplicate and series settings moved there from Settings. Stacking hides the other frames, including ones that match a filter, until you open the group.
- **Sorting:** a sort box in the header orders the grid and the strip under the map and the analysis: newest, oldest, best rated, last imported, file name, sharpest, softest, brightest, darkest. Photos not measured yet come last. The index is rebuilt once on first start.
- **Splitter:** drag the line between the map (or the charts) and the photo strip to resize it; arrow keys move it, double-click resets. Remembered.
- **Settings:** a wider dialog with categories (Sidebar, Map & places, Analysis).

- **Analysis settings:** Settings now has an Analysis section where you set when a photo counts as a colour (share of the photo, floor for the biggest colour, black & white chroma), as dark, bright, clipped or low contrast, as soft or blurry, as square or panorama, and what makes a duplicate or a series. The numbers beside each group show how many photos each class would get; Apply sorts the measured photos again without measuring them. Kept per library (#29). Not yet: a "has a distinct patch of red" colour mode, which needs the photos measured again.

- **Start-up dialog:** while a catalog opens, a dialog shows the step (opening the library, reading the catalog, indexing N of M photos, saving the index) and the time so far, instead of an empty window. The window is no longer blocked while the library opens.

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
