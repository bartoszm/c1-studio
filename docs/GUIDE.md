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
  own rules. *+ Album*, *+ Smart album* and *+ Group* under the title, drag and drop, and
  a right-click on an album make and change them (see [Albums](#albums)).
- **Date:** years › months › weeks. Picking one opens it; picking it again goes up a
  level. A week is the days of one ISO week inside one month, so a week that spans two
  months shows under each with its own days. *Custom range…* takes a From and a To day
  (either can stay empty) or *Last 7, 30, 90 or 365 days*. A range and a pick in the
  tree replace each other.
- **Location:** Country › State › City › Location, each including everything below it.
  A level the photos have no value for keeps its place as *(no state)*, *(no city)* and
  so on, so a city without a state is not mixed in with the states. Below the tree,
  *N without an ISO country code · Fill in…* lists those photos' countries with the code
  each name gives (change it or untick a row) and fills them as one edit.
  Right-click a place to **Rename…** it (Krakow → Kraków; USA → United States, with
  its ISO code) or **Join into…** another place at the same level (similar names such
  as Krakow / Kraków / krakow are offered first). Every photo at or under it changes,
  keeping the levels below, as one edit; a rename to a name that exists joins them.
  On a missing level (*(no state)*) it is **Set a name…**: e.g. give all of Poland's
  photos without a state one.
- **Folders:** the folder tree; a folder includes everything below it.
- **Keywords:** the keyword tree; a keyword includes the keywords below it. The *Find a
  keyword* box above it shows only matching keywords, with their branches open. Esc
  clears it. Right-click a keyword to change the tree (see [Keyword tree](#keyword-tree)).
- **Rating, Colour, Camera:** one value each. Camera lists the ten most common.

**From the details panel.** The camera, each keyword (or any level of it), each album
and the folder are links: clicking one filters by it.

## Browsing

**Selecting.** Click a photo to focus it. Ctrl+click adds or removes one, Shift+click
adds a range (so do Shift+arrows, Page Up/Down, Home and End), Ctrl+A picks the whole
result, and Esc goes back to the focused photo alone. Picked photos are outlined; edits
apply to all of them.

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
| Keywords of several photos | With several picked, the panel lists every keyword any of them has. One on only some is dimmed, with how many carry it (3/5); + adds it to all, *Add to all* does so for every such keyword, and × removes it from all. |
| Location | Type in the panel's Country, Code, State, City and Location fields. Enter or leaving the field saves, Esc puts the old value back, an empty field clears it. A country fills its ISO code (English, Polish or your language's name, and USA, UK, England, Hong Kong…); clearing the country clears the code; a code typed where there is no country fills the country. |
| GPS | The panel's GPS field: `50.0614, 19.9366`, with N/S/E/W, or degrees, minutes and seconds. Empty clears it. GPS belongs to the image, so every variant of it moves. |
| Albums | Drag the photos onto an album in the sidebar (several picked go together). The panel lists the albums a photo is in; × takes it out. With several picked, an album holding only some shows how many (3/5), and + puts the rest in. While the grid shows an album, Delete takes the picked photos out of it. Only albums take photos; see [Albums](#albums). |
| IPTC fields | Typed in the panel's IPTC sections, grouped as in Capture One: **Content** (Headline, Description), **Image** (Title), **Status** (Job ID, Instructions, Source, Copyright), **Contact** (Creator). Enter or leaving saves, Esc puts the old value back, empty clears. Description takes several lines: Enter adds a line, Ctrl+Enter saves. |

**Title is not a caption.** Capture One follows the strict IPTC meaning: Title is the
*Object Name*, a short formal identifier of the image (a file or job code), and sits
under IPTC – Image. The caption is the Description.

**Copy to the selected photos.** Pick several photos and focus the one to copy from
(click it last). **⇉ Copy to N selected** in the header (Ctrl+Shift+S) lists its fields
with their values: tick what to copy, choose **Overwrite** (their values are replaced)
or **Merge** (only what they leave empty is filled, and keywords are added; nothing of
theirs is replaced or cleared), and it goes onto the other N photos as one edit. The
choice is remembered.

**Copy and paste**, for photos not picked together: *Copy…* (Ctrl+C, or Ctrl+Shift+C) lists the focused photo's
rating, colour, keywords, location, GPS and IPTC fields with their values to tick; for
keywords, choose *add to theirs* or *replace theirs*. *Paste* (Ctrl+V) puts the chosen
fields on the picked photos, overwriting, *Merge* (Ctrl+Alt+V) merges, and *Paste…*
(Ctrl+Shift+V) asks first. With Overwrite, a ticked field that is empty on the source
clears it on the targets. The footer says what a paste did, or that the photos already
held those values. The clipboard keeps the photo as it was when copied, and survives
restarts.

What *Copy to N selected* copied also lands on the clipboard, for pasting elsewhere.

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

## Albums

Capture One has two kinds of album, and so does the app:

- An **album** holds the photos put into it, by hand. Its count is its photos.
- A **smart album** holds no photos of its own: it is a rule (keyword contains *street*,
  rating at least ★★★★, captured in 2025), and shows whatever photos match it now and
  later. Photos cannot be put into one or taken out of it; change its rule instead. Its
  icon differs, and its count is what the rule selects.

**Groups** hold albums, smart albums and other groups; **projects** (made in Capture One)
hold them too.

*+ Album*, *+ Smart album* and *+ Group* under the Albums title make one at the top
level (or choose a group in the dialog). In the tree:

- **Drag photos** from the grid onto an album to put them in it (all the picked photos,
  if the one dragged is picked).
- **Drag an album or group** onto a group (or project) to move it there, with what is
  inside; onto the Albums title for the top level.
- While the grid shows an album, **Delete** takes the picked photos out of it.

Right-click an album, smart album or group for:

- **Put the N selected photos here** (albums).
- **Edit rule…** (smart albums): match *all* or *any* of the conditions; each is
  *Keyword contains* (any level of a keyword's name, ignoring case), *Rating is*,
  *Rating is at least* or *Captured in year*. These are the conditions Capture One's
  own smart albums in your catalogs use; a smart album with another kind of condition
  is shown greyed out and edited in Capture One.
- **New album / smart album / group…**, in the group (or beside the album) clicked.
  A new album can take the selected photos at once.
- **Rename…**, **Move to…** (a group, project or the top level), **Move to the top
  level**, and **Delete…**. Deleting a group takes everything in it. The photos stay in
  the catalog; only their place in the album goes.

Capture One's own *Recent Captures* and *Recent Imports* are left alone: they cannot be
renamed or deleted here, nor given new albums (the albums Capture One made in Recent
Imports can). Projects are moved and deleted in Capture One, and so is a group with a
project in it.

Every album change shows at once (in blue until written), is queued like any edit,
written before the photo edits, and undone with Ctrl+Z.

## Writing to Capture One

Nothing is written to the catalog until you ask, or turn Autosave on.

**Write to Capture One…** (in the footer) first reads the catalog, then lists:

- **Changed in Capture One too:** fields edited both here and in Capture One. Choose
  *Keep mine* or *Take Capture One's* for each, or for all at once. Nothing is written
  until every one is settled.
- **Keyword changes:** tree changes, written first.
- **Album changes:** albums, smart albums and groups made, renamed, given a new rule or
  deleted, written next (photos put into a new album need it to exist).
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

**Restoring a backup.** *Backups…* in the source menu (click the catalog name at the top
left) lists them, newest first, with when, how long ago and how big. *Restore…* asks
first, then puts the catalog back as it was then: everything changed in it since, in
Capture One or written from here, is undone. As with a write, Capture One must be
closed. The catalog as it is now is backed up first and shows at the top of the list
(*before restoring the backup of …*), so a restore is undone by restoring that one.
Edits queued in the app stay queued, and are checked against the restored catalog when
you next write.

The queue is saved with the library and survives restarts.

## Keyboard

| Key | Does |
|---|---|
| Arrows, Page Up/Down, Home, End | Move the focus; with Shift, pick a range |
| Click, Ctrl+click, Shift+click | Focus one; add or remove one; add a range |
| Ctrl+A | Pick the whole result |
| Esc | Back to one photo; close the viewer, a dialog or a menu |
| Enter, double-click | Open the viewer |
| ← → (viewer) | Previous, next photo |
| `i` | Show or hide the details panel |
| `0`–`5` | Rating (0 clears) |
| `-` `+` `*` | Red, green, yellow tag |
| Ctrl+K | Add a keyword (details panel shown) |
| Ctrl+C (or Ctrl+Shift+C) | Copy… from the focused photo |
| Ctrl+V, Ctrl+Alt+V, Ctrl+Shift+V | Paste (overwrite), merge, Paste… (choose what and how) |
| Delete | Take the picked photos out of the album the grid shows |
| Ctrl+Shift+S | Copy to N selected: the focused photo's fields onto the other picked photos |
| Ctrl+Z; Ctrl+Y or Ctrl+Shift+Z | Undo; redo |
| Ctrl+wheel, Ctrl+plus, Ctrl+minus, Ctrl+0 | Bigger, smaller, default tiles |

Keys do not act while you type in a field.

## Where things are kept

- The library, the edit queue, the autosave setting and backups:
  `%LOCALAPPDATA%\c1studio\<catalog id>\`.
- Recent catalogs: `%APPDATA%\c1studio\sources.json`.
- The sidebar layout, tile size and the copy clipboard: the app's own browser storage.
