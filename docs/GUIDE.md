# C1 Studio user guide

C1 Studio is a fast browser for a Capture One catalog. It finds photos by any mix of
album, date, folder, keyword, rating, colour, camera, place and free text, shows them
in a grid and a full-screen viewer, and edits ratings, colour tags, keywords, the
keyword tree, location and GPS. Edits wait in a queue and go to Capture One when you
write them, or by themselves with Autosave on.

For the command-line tool see [CLI.md](CLI.md).

## Starting

```powershell
cd app
pnpm install
pnpm tauri build --debug --no-bundle    # then run target\debug\c1studio.exe
pnpm tauri dev                          # or a live-reloading window while developing
```

## Opening a catalog

On start the app opens the first of:

1. a path on the command line: `c1studio.exe C:\c1_catalogs\fuji` (the `.cocatalogdb`
   file, or the folder holding exactly one);
2. `C1_CATALOG` (for scripts and test instances; not remembered);
3. the catalog opened last;
4. the only catalog under `C:\c1_catalogs\*\`.

**The source menu** (*C1 Studio ▾* at the top left, showing the open catalog's name)
opens another one: pick one from *Recent* (× takes it off the list), browse with
*Capture One catalog…*, or type a path and press Enter. The window reloads on the new
catalog, with its own library, queue and settings. If nothing could be opened, the menu
opens by itself.

**Versions.** The menu shows which Capture One made the catalog, and whether reading and
writing are checked for it. Catalogs from Capture One 16.8 are read and written; 16.5
catalogs are read only; any other version is tried for reading only, untested. A
read-only or untested catalog shows a badge in the footer ("read-only · 16.5"); you can
still edit, and the edits stay queued in the app. When a catalog cannot be read at all,
the grid says why.

## Syncing

On start the app reads what changed in the catalog since last time. The first start
indexes the whole catalog; after that it takes a moment, and the footer says "syncing…"
until it is done. Capture One can stay open while you browse and edit.

The app reads the catalog again when you open *Write to Capture One…*, when it writes,
and when you undo a keyword tree change. Changes made in Capture One at other times show
up at the next of these, or on the next start.

`c1cat` and the app share the library and cannot run on the same catalog at once.

## The window

- **Header:** the source menu, the search box, tile size controls, the *Details* toggle and the number
  of matching photos with how long the query took.
- **Sidebar** (left): Albums, Date, Folders, Keywords, then Rating, Colour, Camera and
  Country. Click a section's title to fold it. Which sections are folded and which
  branches are open is remembered.
- **Filter bar:** above the grid while anything is filtered, a chip per filter value.
  × on a chip drops that value; *Clear filters* drops them all.
- **Grid:** the matching photos, newest first.
- **Details panel** (right): everything about the focused photo. `i` or *Details*
  shows and hides it.
- **Footer:** the catalog (with a badge if it is read-only or untested), how many photos the library holds, whether Capture One is
  open, sync and autosave messages, the Autosave switch and, when edits are waiting,
  *N edits waiting · Write to Capture One…*.

## Finding photos

Every filter narrows the result; the grid and every count update as you go.

**Search.** Type in the header box to search keywords, captions, places and
filenames. Words must all match; `"a phrase"` matches the words together and `a OR b`
either.

**Sidebar.** Click a value to filter by it, click it again to let go. Each group counts
as if its own pick were not made, so the alternatives stay visible: with *2025* picked,
the Date tree still shows every year.

- **Albums:** albums, smart albums and their groups. Smart albums are counted by their
  own rules.
- **Date:** years › months › weeks. Picking one opens it; picking it again goes up a
  level. A week is the days of one ISO week inside one month, so a week that spans two
  months shows under each with its own days. *Custom range…* takes a From and a To day
  (either can stay empty) or *Last 7, 30, 90 or 365 days*. A range and a pick in the
  tree replace each other.
- **Folders:** the folder tree; a folder includes everything below it.
- **Keywords:** the keyword tree; a keyword includes the keywords below it. The *Find a
  keyword* box above it shows only matching keywords, with their branches open. Esc
  clears it. Right-click a keyword to change the tree (see [Keyword tree](#keyword-tree)).
- **Rating, Colour, Camera, Country:** one value each. Camera and Country list the ten
  most common.

**From the details panel.** The camera, each keyword (or any level of it), each album
and the folder are links: clicking one filters by it.

## Browsing

**Selecting.** Click a photo to focus it. Ctrl+click adds or removes one, Shift+click
adds a range, Ctrl+A picks the whole result, and Esc goes back to the focused photo
alone. Picked photos are outlined; edits apply to all of them.

**Moving.** Arrow keys, Page Up/Down, Home and End move the focus.

**Viewer.** Enter or a double-click opens the focused photo full size. ← and → step
through the result, Esc or a click closes it. Capture One's thumbnail shows at once and
the sharp preview replaces it when ready; the next one loads in the background. With
the details panel on, it shows beside the photo.

**Tile size.** The header slider sets about how wide the tiles are, and the count per
row follows the window. Typing a number in *per row* keeps that many whatever the
window. *3:2* switches to square tiles and back. Ctrl+wheel over the grid, Ctrl+plus and
Ctrl+minus step one tile per row; Ctrl+0 resets. Tiles bigger than the thumbnail load
the sharp preview once scrolling stops.

The app shows Capture One's thumbnails and previews, never the original files.

## Editing

Edits apply to the picked photos, or the focused one if only one is picked. They show
at once everywhere (grid, counts, panel) and are queued for Capture One; a blue dot marks
photos with edits not yet written, and the panel lists which fields.

| Edit | How |
|---|---|
| Rating | `1`–`5`; `0` clears. Or click a star in the panel; clicking the current one clears it. |
| Colour tag | `-` red, `+` green, `*` yellow. Or click a dot in the panel; clicking the current one clears it. |
| Add a keyword | Ctrl+K (with the panel shown), or the *Add a keyword* field in the panel. It completes from the catalog, most used first. Nested keywords are typed with `\|`: `trips \| japan`. A keyword that does not exist yet is created. |
| Remove a keyword | × next to it in the panel. Keywords under it go too; the × says so. |
| Location | Type in the panel's Country, Code, State, City and Location fields. Enter or leaving the field saves, Esc puts the old value back, an empty field clears it. |
| GPS | The panel's GPS field: `50.0614, 19.9366`, with N/S/E/W, or degrees, minutes and seconds. Empty clears it. GPS belongs to the image, so every variant of it moves. |
| IPTC fields | Typed in the panel's IPTC sections, grouped as in Capture One: **Content** (Headline, Description), **Image** (Title), **Status** (Job ID, Instructions, Source, Copyright), **Contact** (Creator). Enter or leaving saves, Esc puts the old value back, empty clears. Description takes several lines: Enter adds a line, Ctrl+Enter saves. |

**Title is not a caption.** Capture One follows the strict IPTC meaning: Title is the
*Object Name*, a short formal identifier of the image (a file or job code), and sits
under IPTC – Image. The caption is the Description.

**Copy and paste.** *Copy…* in the panel (Ctrl+Shift+C) lists the focused photo's
rating, colour, keywords, location, GPS and IPTC fields with their values. Tick what
to copy; for keywords, choose *add to theirs* or *replace theirs*. *Paste* (Ctrl+Shift+V)
puts them on the picked photos as one edit. A ticked field that is empty on the source
clears it on the targets. The clipboard keeps the photo as it was when copied, and
survives restarts.

**Undo.** Ctrl+Z undoes the last edit, however many photos it touched; Ctrl+Y or
Ctrl+Shift+Z redoes it. Editing a field back to what Capture One holds takes it out of
the queue.

## Keyword tree

Right-click a keyword in the sidebar:

- **Rename…**: a new name; the keyword keeps its place in the tree.
- **Join or replace…**: every photo with these keywords gets the target instead, and
  they leave the tree; keywords under them move along. Add more keywords to join with
  *Add another keyword*. The target can be an existing keyword or a new one. With one
  keyword this is a replace.
- **Group under…**: these keywords, with everything under them, go under a parent,
  existing or new. Empty puts them at the top level.
- **Move to the top level**, for a nested keyword.
- **Ungroup**, for a keyword with children: they move up a level, and the keyword goes
  if no photo carries it itself.

The dialog says how many photos carry the keywords, and when the target is new. The
change shows at once, is queued like any edit, written before the photo edits, and
undone with Ctrl+Z.

## Writing to Capture One

Nothing is written to the catalog until you ask, or turn Autosave on.

**Write to Capture One…** (in the footer) first reads the catalog, then lists:

- **Changed in Capture One too:** fields edited both here and in Capture One. Choose
  *Keep mine* or *Take Capture One's* for each, or for all at once. Nothing is written
  until every one is settled.
- **Keyword changes:** tree changes, written first.
- **To write:** each edit, with the old and new value.
- **Cannot be written:** edits Capture One can no longer take (the photo is gone), with
  *Drop these*.
- Edits Capture One already matches leave the queue.

*Write N edits* works only on a catalog the app can write (see
[Versions](#opening-a-catalog)) and while **Capture One is closed**: it keeps the catalog in
memory and would overwrite the changes. Close it and click *Check again*. Before
writing, the catalog is backed up; the dialog shows where. *Undo all…* throws the whole
queue away.

**Autosave** (footer checkbox, off by default, remembered per catalog) writes the queue
by itself 8 seconds after your last edit, whenever Capture One is closed, under the same
rules and backups. The footer says what it is doing: waiting for Capture One to close,
saved N edits at a time, or paused for conflicts; click it to settle them. A pause or a
failure is not retried until you edit again.

**Backups** go to `%LOCALAPPDATA%\c1studio\<catalog id>\work\backups\`. Writes in a row
within an hour share one backup as long as nothing else changed the catalog in between;
the newest five are kept.

The queue is saved with the library and survives restarts.

## Keyboard

| Key | Does |
|---|---|
| Arrows, Page Up/Down, Home, End | Move the focus |
| Click, Ctrl+click, Shift+click | Focus one; add or remove one; add a range |
| Ctrl+A | Pick the whole result |
| Esc | Back to one photo; close the viewer, a dialog or a menu |
| Enter, double-click | Open the viewer |
| ← → (viewer) | Previous, next photo |
| `i` | Show or hide the details panel |
| `0`–`5` | Rating (0 clears) |
| `-` `+` `*` | Red, green, yellow tag |
| Ctrl+K | Add a keyword (details panel shown) |
| Ctrl+Shift+C, Ctrl+Shift+V | Copy…, paste |
| Ctrl+Z; Ctrl+Y or Ctrl+Shift+Z | Undo; redo |
| Ctrl+wheel, Ctrl+plus, Ctrl+minus, Ctrl+0 | Bigger, smaller, default tiles |

Keys do not act while you type in a field.

## Where things are kept

- The library, the edit queue, the autosave setting and backups:
  `%LOCALAPPDATA%\c1studio\<catalog id>\`.
- Recent catalogs: `%APPDATA%\c1studio\sources.json`.
- The sidebar layout, tile size and the copy clipboard: the app's own browser storage.

## Not yet

- Adding photos to albums and creating albums.
- A list of backups with restore.
- Typing captions in the details panel.
