# Changelog

## Unreleased

- **Library open in another window:** a window that could not open its library because another C1 Studio window had it said so and stayed stuck, even after the other window was closed. It now explains it ("This library is open in another C1 Studio window") and has a **Try again** button (in the window and in the source menu).

- **Analysis tabs:** the Analysis view has two tabs. *Look* holds the shape, colour, light and focus charts, with a **Stack bursts & duplicates** toggle (also in the header) that shows one photo per group, its sharpest. *Alike* holds duplicates and series in one list, sortable (most matching, biggest, newest, oldest, sharpest), with sliders for what counts as a duplicate or a series; the duplicate and series settings moved there from Settings. Stacking hides the other frames until you open the group; with a filter on, a frame is hidden only when the sharpest frame of its group matches the filter too, so no burst disappears.
- **Sorting:** the header has a *Sort by* box (Date taken, Rating, Colour label, Imported, File name, Sharpness, Brightness, Best match) and a direction button that says what it does ("Newest first", "Lowest first", "Z to A"; click to reverse). It orders the grid and the strip under the map and the analysis, and every field has both directions. **Best match** ranks photos by how well they fit the colours, light and focus you picked (red + green: the share of the photo in either; dark: how dark; sharp: how sharp), averaged when you pick several; it is offered only while such a filter is picked. Photos not measured yet come last. Groups in Alike sort the same way. The index is rebuilt once on first start.
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
