# Changelog

## 0.6.0 — 2026-10-23

### New

- **Describe photos with AI:** the **AI** tab sends the picked photos to a vision model and shows what it suggests: a caption (English, Polish or both), a headline, the subject, keywords chosen from the catalog's own (and at most three new ones), a rating, quality and composition, and the place level by level (country, region, city, location, a position), each with how likely it is. What a photo already has goes along as context. Each photo gets a card to review: tick what to take, then **Accept** or **Reject**, or accept every card above a confidence at once. Accepted suggestions are edits in the queue like any other, marked *AI* in the history and undone with Ctrl+Z; a caption, rating or place the photo already has is replaced only when ticked. The estimate before a run says how many photos and calls it takes, and what it costs on the API. Photos are sent only when you start a run (#68, #73).
- **Three ways to reach a model** (*Settings › AI › Access*): Claude with an Anthropic API key, billed per token (the key is kept in the Windows Credential Manager); Claude on your Claude plan, through Claude Code; or **Gemini through agy**, the Antigravity CLI, on your Google account's quota. Gemini can take **contact sheets**: up to 36 numbered photos in one call, which takes far fewer calls for a first pass over many photos (#68, #74).
- **The photo sent** (*Settings › AI › Photo sent*): Capture One's own thumbnail, its render of your edits (the default), or the larger preview, whose colours the app matches and which can be off (#74).
- **See what a run is doing:** the run line names the sheet (or photo) at work, its step (making its image, waiting for the answer, trying again) and how long it has taken; the bar counts sheets. **Details** shows each call at work with its photos and, for a sheet, **Show sheet**, the image as sent; and a log of the run, which stays after it ends, so a failure can be read later. The footer shows the sheet too (#77).
- **Keywords offered** (*Settings › AI*): the keyword tree with a box each, to leave keywords out of what the model may choose, with all below them. Keywords named only with numbers (01, 02, 2023) are left out by default; one box brings them back. A keyword left out is not suggested as new either (#78).
- **History:** a **History** tab with the catalog over time: photos imported per day and the change log's days. Drag across it to compare two dates: photos changed, added and removed, per field, keywords gained and lost, albums. Every number shows its photos in the grid. The change log now also keeps what a change replaced. *Settings › History* turns the tab off for a catalog (#67).
- **No location:** *Places › No location* shows the photos with nothing in the Location field (and click again, those with one), also per city. With a city known, the Location field offers only that city's locations (#69).
- **Map and grid in sync:** a single photo's pin picks the photo in the grid and scrolls to it; with *sync grid and map* on, the map centres on the photo picked in the grid. **Google plus codes** are taken wherever GPS is typed (full codes, short ones near the photo or the map's middle, or short code + town), and the details panel shows a photo's plus code (#69).

### Changed

- **Neighbours… is one dialog:** the *Fill missing* and *Refine* tabs are gone. **Position** gives photos with no position, or only their place's (≈), one from the GPS photos taken around them (30 minutes by default): between two, or at the nearest within half the window, but not between two taken from a plane, and only if it lies in the photo's own place, which *Fill missing* did not check. A ≈ position now moves like a missing one, no longer only to GPS taken within 2 minutes. **Place** gives photos with no place, or no location, the place of a neighbour that says the same and more (15 minutes). It now looks past neighbours that only repeat the photo's place, so a city's photos get the location of a photo taken at a named spot nearby, not just the one next to it. Both sides must agree: a neighbour somewhere else, on either side, gives nothing.
- **Each kind of source names itself:** dialogs, the footer and messages say *catalog* and *Capture One* for a catalog, *library* and *the photo folders* for folders, and no longer mention Capture One where it does not apply. While a photo folders source starts, the start-up dialog shows its scan ("Scanning folders 12,345") (#70, #71).
- **Photo folders do less work:** a rating or caption change no longer rebuilds the folder and keyword trees, a scan reads each file's size and time once, and thumbnails are kept across restarts (#70).

### Fixed

- **"Capture One: open" in the footer** after the first read of a catalog, while the app made thumbnails or measured photos: its own reads of the catalog counted as another program holding it, and a write in that time was refused. Only other programs count now (#76, closes #72).
- **Offline photos** were not dimmed after a restart in which the scan found nothing new. They are dimmed straight away now (#71).
- **Errors in the footer** clear after 10 seconds or a click, instead of staying until the next edit (#69).

### Install

Run `C1.Studio_0.6.0_x64-setup.exe`. It installs over 0.5.x for the current user and keeps your settings and your library: the search index is not rebuilt. Describing photos needs one of: an Anthropic API key, Claude Code signed in with a Claude plan, or agy signed in with a Google account (*Settings › AI* says which is ready).

## 0.5.0 — 2026-10-21

### New

- **Trash:** **Delete** moves the picked photos (or the one in the viewer) to C1 Studio's trash, out of every view and count; Ctrl+Z brings them back. In an album, Delete still takes photos out of the album and **Ctrl+Delete** trashes them. **Trash** at the bottom of the sidebar shows them, with **Restore**, **Delete for good…** and **Empty trash…**. Emptying sends a photo folder's files (and their own `.xmp` sidecars) to the Windows Recycle Bin; in a Capture One catalog it marks the images deleted, so they show in Capture One's own Trash, after a backup and with Capture One closed. Nothing reaches the source before the trash is emptied (#64).

### Fixed

- **Editing in the full-screen viewer:** after ← or →, the first rating, colour or keyword went to the photo just left, so stars took two presses and keywords seemed not to be added. Edits now land on the photo shown (several picked photos stay picked). ← → and Escape typed in a field of the details panel stay in the field instead of changing photo or closing the viewer, keys no longer reach the viewer behind an open dialog, quick arrow presses are no longer lost, and the viewer's bar shows a rating or keyword as soon as it is set (#65).

### Install

Run `C1.Studio_0.5.0_x64-setup.exe`. It installs over 0.4.x for the current user and keeps your settings and your library: the search index is not rebuilt.

## 0.4.0 — 2026-10-19

### New

- **Photo folders:** open plain folders of JPEG, PNG, TIFF and raw files as a source, the same way as a Capture One catalog (**Source > New photo folders…**). EXIF, XMP and IPTC are read; your edits are kept in C1 Studio's own store and written to the files only if XMP export is on. A photo keeps its id and edits when its file is moved or renamed. A drive that is not connected marks its photos offline instead of losing them; **Rescan** looks at the folders again. Thumbnails and previews go to a central folder (with a size limit), a `.c1studio` folder under each root (they travel with an external drive), a `.previews` folder beside each photo, or a pattern of your own. `c1cat` opens folders too (`--source`, `rescan`, `folders …`) (#14, #44).
- **Choose the kinds of source:** the installer has a *Photo sources* page: Capture One catalogs, photo folders, or both. The source menu then offers only those; **Kinds of source…** at its bottom changes it later. A silent install takes `/KINDS=capture-one,folders` (#62).
- **Refine places from neighbours:** **Neighbours…** on the map has a **Fill missing | Refine** switch. Refine gives a photo placed only roughly its neighbour's more specific place (Kraków → Kraków › Stare Miasto, within 15 minutes), and moves a ≈ position to the GPS of a photo taken within 2 minutes, if it lies in the photo's own place. The dialog shows the counts and every from → to first; Ctrl+Z undoes it (#61).

### Fixed

- **GPS edits that could not be written:** moving some photos on the map queued an edit listed under *Cannot be written*, because a camera position read back from the edit queue one digit off. Positions now read back exactly. An edit already queued that way stays blocked: *Drop these* and move the photo again (#56).

### Install

Run `C1.Studio_0.4.0_x64-setup.exe`. It installs over 0.3.x for the current user and keeps your settings, and asks which kinds of photo source to offer. On first start the search index is rebuilt once. Edits, trips, places and hidden photos are kept.

## 0.3.0 — 2026-10-18

### New

- **Hidden photos:** press **h**, or **Hide** in the details panel, to keep photos (sensitive, private) out of every view: the grid, the map, the timeline, trips, the analysis and every count. **Hidden** in the header shows them again, dimmed and labelled on their tiles, and **h** or **Unhide** brings them back for good; undo works. The mark is C1 Studio's own: nothing is written to Capture One, and it survives a rebuilt library. It is not encryption: the photos stay in the catalog and in the thumbnail cache (#51).
- **Analysis settings:** Settings > Analysis sets when a photo counts as a colour, as dark, bright, clipped or low contrast, as soft or blurry, as square or panorama. The numbers beside each group show how many photos each class would get; Apply sorts the measured photos again without measuring them. Kept per library (#29).
- **Colour by patch:** a colour can count when it *covers* a share of the photo (now measured per pixel, so a small vivid area is no longer swallowed by its neighbours) or when the photo *has a vivid patch* of it (a red coat on a grey street). A grey photo with a vivid patch is no longer black & white (#29).
- **Why a photo is red, dark or soft:** the details panel's Look section names the palette's colours, marks black, white and grey as not counting, and under each class gives the measure and the line: "red 19% · counts from 16.7%", "brightness 31% (dark below 20%, bright above 55%)", "detail 1,240 (soft below 700, blurry below 300)" (#30).
- **Charts that explain themselves:** each Analysis card says what it measures, each bar gives its rule, and a note says how picks combine. Light is split into *Brightness* and *Light problems*. Main colour and Light problems have an any/all switch: "all" asks for photos that are red **and** blue (#31).
- **Analysis tabs:** *Look* holds the charts and a **Stack bursts & duplicates** toggle (also in the header) that shows one photo per group, its sharpest; with a filter on, a frame is hidden only when the sharpest frame of its group matches too, so no burst disappears. *Alike* holds duplicates and series in one sortable list, with sliders for what counts as a duplicate or a series.
- **Sorting:** a *Sort by* box (Date taken, Rating, Colour label, Imported, File name, Sharpness, Brightness, Best match) and a direction button that says what it does ("Newest first", "Lowest first", "Z to A"). **Best match** ranks photos by how well they fit the colours, light and focus you picked; it is offered only while such a filter is picked. Photos not measured yet come last.
- **Splitter:** drag the line between the map (or the charts) and the photo strip to resize it; arrow keys move it, double-click resets.

### Improved

- **Settings:** a wider dialog with categories (Sidebar, Map & places, Analysis).
- **Start-up dialog:** while a catalog opens, a dialog shows the step and the time so far, instead of an empty window; the window is no longer blocked while the library opens.
- **Header:** the map's own tools (grid follows map, GPX…, Neighbours…) are in a row above the map, and on a narrow window the search box shrinks first, so the view buttons are never squeezed.
- **Library open in another window:** the window says so and has a **Try again** button, instead of staying stuck after the other window is closed.

### Install

Run `C1.Studio_0.3.0_x64-setup.exe`. It installs over 0.2.x for the current user and keeps your settings. On first start the search index is rebuilt and every photo is measured once more in the background (about 4 minutes per 100,000 photos) to know its colour patches.

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
