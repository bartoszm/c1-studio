# C1 Studio user guide

C1 Studio is a fast browser for a Capture One catalog. It finds photos by any mix of
album, date, folder, keyword, rating, colour, camera, place and free text, shows them
in a grid and a full-screen viewer, and edits ratings, colour tags, keywords, the
keyword tree, location and GPS. Edits wait in a queue and go to Capture One when you
write them, or by themselves with Autosave on.

For the command-line tool see [CLI.md](CLI.md).

## Starting

Install it with `C1.Studio_<version>_x64-setup.exe` from the
[latest release](https://github.com/bartoszm/c1-studio/releases/latest) and start
*C1 Studio* from the Start menu. The installer is not code-signed, so Windows may warn
about an unknown publisher: choose *More info* → *Run anyway*. The first time, nothing
is open: choose a catalog from the source menu (see below).

The installer asks which kinds of photo source to offer: *Capture One catalogs*, *Photo
folders*, or both. The source menu then offers only those. Change the choice later
under *Kinds of source…* at the bottom of the source menu, where at least one kind stays
on. Turning a kind off leaves the source that is open as it is, and hides that kind's
recent sources from the menu. A silent install (`/S`) keeps the choice made before.
`/KINDS=capture-one,folders` sets the choice instead. The choice is kept in
`%APPDATA%\c1studio\providers.json`. Without that file every kind is offered. `c1cat`
always opens every kind.

To build it from source instead:

```powershell
cd app
pnpm install
pnpm tauri build --debug --no-bundle    # then run target\debug\c1studio.exe
pnpm tauri dev                          # or a live-reloading window while developing
```

## Opening a catalog

On start the app opens the first of:

1. a path on the command line: `c1studio.exe D:\Catalogs\Mine` (the `.cocatalogdb`
   file, or the folder holding exactly one);
2. `C1_CATALOG` (for scripts and test instances; not remembered);
3. the catalog opened last.

The first time, nothing is open: choose a catalog from the source menu.

**The source menu** (*C1 Studio ▾* at the top left, showing the open catalog's name)
opens another one: pick one from *Recent* (× takes it off the list), browse with
*Capture One catalog…*, or type a path and press Enter. The window reloads on the new
catalog, with its own library, queue and settings. If nothing could be opened, the menu
opens by itself.

To open plain photo folders instead of a catalog, see [Photo folders](#photo-folders).

**Versions.** The menu shows which Capture One made the catalog, and whether reading and
writing are checked for it. Catalogs from Capture One 16.8 are read and written; 16.5
catalogs are read only; any other version is tried for reading only, untested. A
read-only or untested catalog shows a badge in the footer ("read-only · 16.5"); you can
still edit, and the edits stay queued in the app. When a catalog cannot be read at all,
the grid says why.

**One window per catalog.** A second C1 Studio window cannot open a library another
window has open: it says so ("This library is open in another C1 Studio window"). Close
the other window and press *Try again* (in the window, or in the source menu).

## Syncing

On start the app reads what changed in the catalog since last time. The first start
indexes the whole catalog; after that it takes a moment, and the footer says "syncing…"
until it is done. Capture One can stay open while you browse and edit.

The app reads the catalog again when you open *Write to Capture One…*, when it writes,
and when you undo a keyword tree change. Changes made in Capture One at other times show
up at the next of these, or on the next start.

`c1cat` and the app share the library and cannot run on the same catalog at once.

## The window

- **Header:** the source menu, the search box, the *Photos*, *Map* and *Analysis* tabs, the
  sort box (see [Sorting](#sorting)), *Stack* (see [Look](#look)), tile size controls,
  *Hidden* (see [Hidden photos](#hidden-photos)), the *Timeline* and *Details* toggles, ⚙
  Settings, and the number of matching photos with how long the query took. On a narrow
  window the search box gives way first; the map's own tools sit in a row above the map.
- **Settings** (⚙): *Sidebar* (which sections show, in what order), *Map & places*
  (online map services, places by name), *Analysis* (the lines between classes; see
  [Look](#look)) and *History* (the History tab on or off; see [History](#history)).
- **Sidebar** (left): Albums, Date, Folders, Keywords, then Rating, Colour, Camera and
  Country. Click a section's title to fold it. Which sections are folded and which
  branches are open is remembered.
- **Filter bar:** above the grid while anything is filtered, a chip per filter value.
  × on a chip drops that value; *Clear filters* drops them all.
- **Grid:** the matching photos, newest first unless the sort box says otherwise.
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
- **Look:** what each photo looks like, measured from its thumbnail: see the
  **Analysis** tab and [Look](#look).
  Several values in one group show photos with any of them (the Analysis tab's *all*
  switch asks for every one).
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

For a Capture One catalog the app shows Capture One's thumbnails and previews, never the
original files. A photo folders source makes its own; see [Photo folders](#photo-folders).

### Sorting

The sort box in the header picks what the photos are ordered by: *Date taken*,
*Rating*, *Colour label*, *Imported*, *File name*, *Sharpness*, *Brightness* or *Best
match*. The button beside it says which way in words that fit the field (*Newest
first*, *Highest first*, *A to Z*, *Sharpest first*…); click it to turn the order round.
A new field starts the usual way. Photos not measured yet (see [Look](#look)) and photos
without a colour label come last either way. The order applies to the grid and to the
strip under the map and the analysis, and is remembered.

**Best match** is offered while you pick a colour, light or focus value (in the
Analysis tab or the sidebar's Look). It puts first the photos that fit the picks best:
for *red* and *blue*, the photo with the most red or blue in it; for *dark*, the darkest;
for *sharp*, the sharpest; with picks in several cards, the scores are averaged. With a
card set to *all* (red **and** blue), a photo is as good as its weakest colour. *Worst
match first* turns it round. Without such a pick it is not offered, and clearing the
picks goes back to *Date taken*.

**The strip's height.** On the Map and Analysis tabs, drag the line between the map (or
the charts) and the photo strip under it; the arrow keys move it once it has focus,
Shift for bigger steps, and a double-click or Home resets it. Remembered.

## Editing

Edits apply to the picked photos, or the focused one if only one is picked. They show
at once everywhere (grid, counts, panel), with the grid, the panel's scroll and the field
being typed in left where they were, and are queued for Capture One; a blue dot marks
photos with edits not yet written, and the panel lists which fields.

| Edit | How |
|---|---|
| Rating | `1`–`5`; `0` clears. Or click a star in the panel; clicking the current one clears it. |
| Colour tag | `-` red, `+` green, `*` yellow. Or click a dot in the panel; clicking the current one clears it. |
| Add a keyword | Ctrl+K (with the panel shown), or the *Add a keyword* field in the panel. It completes from the catalog, most used first. Nested keywords are typed with `\|`: `trips \| japan`. A keyword that does not exist yet is created. |
| Remove a keyword | × next to it in the panel. Keywords under it go too; the × says so. |
| Keywords of several photos | With several picked, the panel lists every keyword any of them has. One on only some is dimmed, with how many carry it (3/5); + adds it to all, *Add to all* does so for every such keyword, and × removes it from all. Long lists show their first 8 (those on only some first); *+N more* shows the rest. |
| Location | Type in the panel's Country, Code, State, City and Location fields. Enter or leaving the field saves, Esc puts the old value back, an empty field clears it. A country fills its ISO code (English, Polish or your language's name, and USA, UK, England, Hong Kong…); clearing the country clears the code; a code typed where there is no country fills the country. |
| GPS | The panel's GPS field: `50.0614, 19.9366`, with N/S/E/W, or degrees, minutes and seconds. Empty clears it. GPS belongs to the image, so every variant of it moves. |
| Albums | Drag the photos onto an album in the sidebar (several picked go together). The panel lists the albums a photo is in; × takes it out. With several picked, an album holding only some shows how many (3/5), and + puts the rest in. While the grid shows an album, Delete takes the picked photos out of it. Only albums take photos; see [Albums](#albums). Ctrl+Delete moves the photos to the [trash](#trash). |
| IPTC fields | Typed in the panel's IPTC sections, grouped as in Capture One: **Content** (Headline, Description), **Image** (Title), **Status** (Job ID, Instructions, Source, Copyright), **Contact** (Creator). Enter or leaving saves, Esc puts the old value back, empty clears. Description takes several lines: Enter adds a line, Ctrl+Enter saves. |

**Several photos picked.** The panel opens with an *Edit N photos* block: rating, colour,
the keyword field, Country, Code, State, City, Location, GPS and the IPTC fields, always in
that order and at the same height, so a field you are typing in never moves. A field shows
the value all of them share; otherwise its placeholder says how they differ (*3 values*,
*on 4 of 10*, *empty on 10*), with more in its tooltip. Below the block: how many of them
have edits waiting, the keyword, album and place lists, then the focused photo's details
(EXIF, Look, File). While a new pick is read, the lists from before stay, dimmed. The
footer says *Editing 20,559 photos…* while a big edit or an undo runs.

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

## Hidden photos

Photos you would rather not show (sensitive, private) can be hidden: press `h`, or
*Hide* in the details panel (for the picked photos too). They leave every view and every
count at once: the grid, the sidebar, the map, the timeline, trips, the analysis and its
groups. Nothing is written to Capture One and nothing is queued: the mark is C1 Studio's
own, kept with the library (it survives a rebuilt index), and not in the change log.

*Hidden* in the header shows them again, marked *hidden* on their tiles and in the
details panel; it is off whenever the app starts. With them shown, `h` or *Unhide* on
hidden photos brings them back for good. Ctrl+Z undoes a hide or an unhide.

Hiding is not encryption: the photos stay in the catalog, in Capture One and in its
thumbnail cache.

## Trash

**Delete** moves the picked photos (or the one in the viewer) to C1 Studio's trash; so
does *Delete* in the details panel. While the grid shows an album, Delete takes the photos
out of the album instead, and **Ctrl+Delete** moves them to the trash. They leave every
view and count at once, as hidden photos do, and nothing reaches the source yet: Ctrl+Z
brings them back.

*Trash* at the bottom of the sidebar shows them, with its count. There, the bar above the
grid has:

- **Restore**: the picked photos go back into every view.
- **Delete for good…**: the picked photos are deleted at their source now.
- **Empty trash…**: every photo in the trash is.

Both ask first, and say what happens; it cannot be undone in C1 Studio:

- **Photo folders:** the files go to the Windows Recycle Bin, with the photo's own `.xmp`
  sidecar (one another photo shares, as `X.RAF` and `X.JPG` share `X.xmp`, stays). Their
  edits in C1 Studio go too. A photo whose drive is not connected, or whose file changed
  since the last scan, stays in the trash. A file another program holds open is moved
  later, at the next delete or scan; it is never found again as a new photo.
- **Capture One:** the photos are marked deleted in the catalog, as Capture One's own
  Delete does, and show in its Trash, where it can restore them or empty them for good.
  Every variant of the same image goes with them: Capture One trashes images, not
  variants, so deleting one variant puts its siblings in the trash too. The catalog is
  backed up first, and Capture One must be closed, as for any write. Their queued edits
  are dropped.

Hidden photos you delete show in the trash too, marked hidden, so they can be restored.
Photos already in Capture One's own Trash stay out of every view, the app's trash
included. The trash is kept with the library (`library\bin.redb`), so it survives a
restart and a rebuilt index.

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
- While the grid shows an album, **Delete** takes the picked photos out of it (**Ctrl+Delete**
  moves them to the [trash](#trash)).

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

## Map

The **Map** tab fills the area with a map and keeps a strip of the photos below it (OpenFreeMap tiles, so it needs a
network; everything else works without one).

- It shows the photos the filters select that have a position. Far out, a disc with the
  number of photos; closer, a tile with the best photo of each cluster (highest rating,
  then sharpest) and the count in its corner; at street level, single photos.
- Click a cluster to zoom to its photos; click a single photo to show it in the
  details panel.
- Zooms and flights (to a cluster, a place found, a trip's route) fit the photos, pins and
  all, in what the find box, the *Not on the map* tray (open or not) and the route bar
  leave free; the map doesn't move when these open or close.
- **Grid follows map** (on by default) makes the grid show only the photos in view: pan
  and zoom to filter. It applies at once: when you tick it, after *Clear filters*, and
  when a filter that kept the grid off the map (photos with no position, or placed only
  by name) is dropped. Removing the chip *Map: area* turns it off. Off, the grid keeps
  its own filter. A view across the 180° meridian (the middle of the Pacific) takes in
  every photo in its latitudes.
- **Pick an area:** Shift+drag a box on the map. The grid then shows just its photos
  (and stops following the map); the chip *Map: area* drops it.
- **Find a place:** the box at the map's top left searches OpenStreetMap's Nominatim by
  name and flies there, with the place in the middle: zoomed to its outline when that
  is around it, else by what it is (a country, a region, a city, a street), so far-off
  islands (Tokyo's, France's) don't pull the view away. With photos picked in the strip, *Put N picked photos here*
  places them at the place found.
- **GPX…** (Map tab) gives photos a position from a recorded track by when they were
  taken. Choose the file, then say what the camera's clock showed against UTC (hours
  east: Poland is 1 in winter, 2 in summer; Capture One keeps the camera's own time).
  The dialog counts the photos in view the track places, as you change the offset; a
  wrong offset places none. Photos in a gap longer than the limit (15 minutes) are left
  alone, as are photos that already have a position unless you tick *Also move…*.
  Positions between two points are interpolated. One undoable edit.
- **Neighbours…** (Map tab) places the photos the filter selects from the photos taken
  just before and after them, by any camera. *Fill missing* gives photos with no
  position one between their neighbours' (or the nearest one's, if close), and photos
  with no place their neighbours' shared place. *Refine* improves rough ones: a photo
  whose place a neighbour within 15 minutes names more specifically (London › Barbican
  for London) takes that place, and a ≈ position (from the place's name) moves to the
  GPS of a neighbour within 2 minutes, if it lies in the photo's own place. The dialog
  shows the counts and every from → to before anything changes; Apply queues one
  undoable edit.
- **Setting a position:** drag a single photo's tile to a new place, or drag photos
  (one or several picked) from the grid onto the map. This is a Location edit like any
  other: queued, undoable, written with *Write to Capture One*. A position belongs to
  the image, so every variant of the photo moves.

## Trips and the timeline

**Trips** (left panel, under Date) are found from when and where photos were taken.

- **Home first.** The section asks where home is and offers the city photographed on the
  most days. Photos taken at home are never part of a trip. With only a country as home,
  only trips abroad are found. *Home: …* at the bottom of the section changes it, and
  sets how long a gap without photos starts a new trip (36 hours by default).
- **How a trip is found.** Each day is home, away or unknown, by the places (country,
  city) of its photos, or by their positions (near home or far from it) when they have no
  place. Days follow one another into one trip while the gap between photos stays under
  the limit, or up to 4 days when both sides are in the same country. A day at home ends
  it. A stretch with a day away is a trip, named after its main places (*Tokyo, Hong
  Kong*). A trip with places named in none of its photos is *Unnamed trip*.
- **Suggested** (in grey, *No place*): a busy stretch (2 days or more, 100 photos or more)
  where no photo has a place. Trips nobody tagged look like that, and so do some weekends
  at home. Hover a suggestion: ✓ keeps it as a trip, × dismisses it. *N suggested* at the
  bottom hides or shows them; *N dismissed* shows the dismissed ones, ↺ brings one back.
- **Click a trip** to show its photos (the chip *Trip: …*). Each trip counts its photos
  under the other filters, and shows the best one.
- **Right-click a trip:** *Rename…* (or double-click it), *Keep as a trip*, *Merge with*
  the trip before or after it, *Split before the focused photo* (focus the first photo of
  the second part in the grid first), *Make an album of it* (a new album at the top level
  holding its photos, written with *Write to Capture One* like any album), *Not a trip*,
  and *Forget my changes*.
- **What stays:** trips are found again whenever the catalog changes, but a trip you
  renamed, kept, merged, split or dismissed stays as you left it; a kept trip only grows
  over new photos taken right after (or before) it. Trips live in C1 Studio
  (`library\trips.redb`), not in Capture One.

**The timeline** (header button *Timeline*) is a strip above the grid and the map: photos
per day, week or month, as room allows, with the trips as bands under the bars.

- It counts the photos the other filters select, whatever dates or trip are picked, so the
  rest of the time stays in view.
- **Drag** across it to pick those days; **click** a bar to pick its day, week or month.
  The chip *Dates: …* drops the pick, as does the Date section.
- **Click a band** to pick that trip. With a trip picked the strip shows just its days
  (and a little either side); *⤢ all* goes back to every day.

**Route** (Map tab, with a trip picked): the trip's photos that have a position, joined in
the order they were taken. ▶ plays it photo by photo, about twenty seconds for the whole
way (no faster than three photos a second): the way travelled is drawn in amber and the
photo of the moment rides at its head. The slider moves along it by hand; ⤢ shows the
whole route. A trip needs two photos with a position for a route.

## History

The **History** tab shows the catalog over time. It needs the change log, which starts the
day the app first read the catalog: the chart marks where it starts, and anything before
is not known.

- **The chart.** Photos imported per day (the bars grow with the square root of the count,
  so one big import does not flatten the rest), a green line for the running total, and
  under the bars a row of marks for the days with log entries. It follows the filters
  (picks from the sidebar narrow it). Drag across it to pick days, or click a bar; *⤢ all*
  lets go.
- **Compare.** With days picked, the panel says how many log entries there are (by who
  made them), and how many photos ended up different: changed, added, removed, deleted
  for good, per field, the 20 keywords gained and lost the most, and per album the photos
  put in and taken out. Changes that cancelled out do not count. Click any number to show
  those photos in the grid; the chip *History: …* above the grid drops the pick.
- **The log.** Newest first. Entries made together (a rating on 1,240 photos from one
  action, or one read of Capture One) are one row. The chips limit it by who (here,
  Capture One, GPX, backup) and by kind; *More* reads the next page. Click a row to show
  its photos. A keyword tree change or an album delete shows the photos it touched, when
  the app recorded them (from this version on).
- Each photo's own history, in the details panel, now also shows keyword tree changes and
  album deletes that touched it.

For the tab, the change log also keeps what a change replaced: a photo as it was when it
was removed or deleted, an album as it was before a rename, move or delete (and who was
in it), and the keywords a keyword tree change rewrote. One read of the catalog keeps at
most 5,000 removed photos whole; past that, they are logged without it, so a catalog that
comes back empty for a moment does not fill the log.

**Turning it off.** *Settings › History › Show the History tab* (on unless you turn it
off) is kept per catalog. Off, the tab is hidden and a pick from it leaves the grid; the
change log goes on as before this version, every change logged but nothing of what it
replaced, and the app does no history work (no compare, no log pages, no day counts).
The details panel still shows each photo's history. Entries logged so far stay as they
are, so turning it back on finds them; what happened while it was off is in the log too,
only without what those changes replaced.

Restoring a past state is not in this version.

## Look

The app measures every photo from Capture One's thumbnail (which carries your edits and
crop) in the background: after a sync, the footer says *measuring photos N / M*. The
first time takes a few minutes for a big catalog (about four for 100,000 photos here);
after that only new photos, and photos whose thumbnail Capture One rendered again, are
measured. Updating to a version that measures something new measures every photo once
more. The results stay with the library and are never written to the catalog.

The **Analysis** tab (beside *Photos* and *Map*; all three share one filter) has two
tabs of its own, **Look** and **Alike**. Under them, a strip shows the photos the filter
picks, as the map's does: pick a bar and see its photos without leaving the tab. Click
one to see it in the details panel; the arrow keys move through them, while Page Up and
Page Down scroll the charts and groups.

**Look** has a card per measure. Each card says what it measures and where its lines
are; each bar's tooltip gives its rule. Clicking a bar filters by it.

- **Shape:** landscape, portrait, square (width ÷ height within 1.1) or panorama (2:1 and
  longer), as cropped.
- **Main colour:** a photo can have several colours (red, orange, brown, yellow, green,
  teal, blue, purple, pink). A colour counts when it *covers* a share of the photo (a
  sixth by default), or, set to *has a vivid patch* in Settings, when the photo has one
  vivid patch of it big enough (2% by default): a red coat on a grey street. When no
  colour counts, the biggest stands for the photo if it shows at all, else it is
  *muted*. *Black & white* is a photo with almost no colour and no vivid patch, so a
  grey photo with one red coat is not black and white. Black, white and grey are never
  colours.
- **Brightness:** dark, medium or bright by mean brightness; one per photo.
- **Light problems:** *clipped highlights* (pure white pixels), *blocked shadows* (pure
  black pixels) and *low contrast*; a photo can have several.
- **Focus:** sharp, soft or blurry, by the detail in the sharpest part of the picture
  (so a sharp subject on a soft background is sharp). Fog and plain walls have little
  detail too, and show as soft.

A photo can count in several bars of a card. Picks in one card mean *any of them*;
picks in different cards, *all of them*. *Main colour* and *Light problems* have an
*any / all* switch: *all* asks for photos that are red **and** blue. The chips say so
("Main colour, all of"). A card goes back to *any* when its picks are cleared.

**Stack** (on the Look tab, and in the header) shows one photo per duplicate group or
series in the grid: its sharpest. With a filter on, a frame is hidden only when the
sharpest frame of its group matches the filter too, so a burst never disappears; the
counts follow. Picking a group shows all of its photos.

**Alike** lists the groups of photos that look alike, *Duplicates* or *Series*:

- *Duplicates* are one picture twice: taken at the same instant and looking the same
  (imported twice, or saved again; variants of one image do not count).
- A *series* is a burst: three or more photos from one camera, seconds apart, each
  looking much like the one before.

Each group's badge is its size; under a filter that picks only part of it, *2 of 7*. A
group the filter leaves too little of is not listed. The sort box above the list orders
the groups by matching photos, size, date or sharpness, either way. The first 60 show,
and *Show more (N left)* adds the next 60. *Lines* opens sliders for what counts as a
duplicate or a series (how alike, how far apart, how many photos); the line under them
says how many photos they would group, and *Apply* groups them again without measuring.

**Settings › Analysis** (⚙ in the header) moves the lines for shape, colour, light and
focus, per library: each group shows how many photos each class would get, and *Apply*
classes the measured photos again without measuring them. *Defaults* puts the lines back.

The details panel's **Look** says why the photo is in its classes: the palette's colours
by name (black, white and grey marked as not counting); under each class the measure and
the line ("red 19% · counts from 16.7%", "brightness 31% (dark below 20%, bright above
55%)", "detail 1,240 (soft below 700, blurry below 300)"); and for a duplicate or a
burst a link to the others (*Looks like 1 other photo: show them*, *One of a series of 7:
show it*). The link and a group's card on the Alike tab show the group's photos within
the other filters, which stay; drop a filter's chip to see the whole group. A photo not
measured yet says so.

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

## Photo folders

C1 Studio can also open plain photo folders, with no Capture One catalog. A photo folders
source is a named set of folders, scanned recursively, holding JPEG, PNG, TIFF and camera
raw files. The window is the same: search, the sidebar, the grid, the viewer and the
details panel work as they do for a catalog. This section covers what differs.

**Making a source.** Choose *New photo folders…* in the source menu. The dialog asks for:

- **Name:** what the menu and the window title show.
- **Folders:** *Add folder…* picks one, *Remove* takes it out. Every photo in a folder,
  and in the folders below it, is listed. A label, if you give one, is shown instead of
  the folder's name.
- **Leave out:** one pattern per line, matched against each path below its folder, with
  `/` between folders. `**/tmp/**` leaves out every folder called `tmp`, and `*.tmp`
  every file whose name ends in `.tmp`. A folder that matches is not entered. The
  `.c1studio` and `.previews` folders are always left out.
- **Thumbnails and previews**, and **XMP**: see below.

*Save and open* saves the definition and opens the source. The definition is a
`.c1folders` file, by default in `%APPDATA%\c1studio\sources`. Its path is the source's
location. To open one made earlier, choose *Open photo folders…* in the source menu.

**Renaming and moving.** The source keeps its data under an id stored inside the
`.c1folders` file. Renaming the source, or moving the file, keeps everything: edits,
albums and the rest of its data.

**Editing the folders.** *Edit folders…* in the source menu changes the folders, the
excludes, where images are kept and the XMP options of the open source. *Save* reads the
source again. Removing a folder keeps its photos' edits in the app; they come back if the
folder is added again.

**Rescanning.** The folders are read when the source opens, and again on *Rescan*, in the
source menu or the button in the footer. Files whose path, size and modification time are
unchanged are not read again. Between scans the library is not checked against the disk.
The footer shows the progress of a scan.

**Moved and renamed files.** A photo keeps its id when its file is moved or renamed, or
when its folder is removed and added back. So its rating, keywords, albums and places stay
with it. Files are told apart by their size and a hash of their first and last 64 KB, and
by the whole file where two files share those. A copy made while the original is still
there is a new photo. So is a file that is both moved and changed between two rescans.

**Offline drives.** A folder that cannot be reached, such as a drive that is not plugged
in, makes its photos offline. So does a folder that is empty although it held photos
before, such as an empty mount point. Nothing is removed in either case. Offline photos
keep their edits, show dimmed in the grid and carry an *offline* tag in the details
panel. The footer says how many folders are offline. When the drive is back, choose
*Rescan*.

**Thumbnails and previews.** The app makes its own. Thumbnails, 450 pixels on the long
edge, are made in the background after a scan. A preview, 2000 pixels on the long edge,
is made when a photo is opened in the viewer. A JPEG up to 3 MB, with its long edge up to
2560 pixels, is shown from the original, with no copy made. A raw file is shown through
the JPEG its camera embedded.

**Where they are kept** is set in the dialog:

- **Central** (the default): in the app's data folder,
  `%LOCALAPPDATA%\c1studio\folders-<id>\work\images`. It has a size limit of 4 GB by
  default; leave the field empty for no limit. Over the limit, the least recently used
  images go first, previews before thumbnails.
- **Per root:** in a `.c1studio` folder under each root, so the images travel with an
  external drive.
- **Beside the file:** in a `.previews` folder next to each photo.
- **Custom:** patterns of your own, for thumbnails and previews. The tokens are `{data}`,
  `{root}`, `{dir}`, `{relpath}`, `{name}`, `{stem}`, `{ext}`, `{hash}` (or `{hash:N}` for
  its first N digits), `{tier}` and `{size}`. A pattern must end in `.jpg`. Save stays off
  while the patterns would give two photos one image file, or put an image on a photo;
  the dialog says which.

Only the central store is trimmed. The other stores are yours and are never trimmed.
If the source is open and the place of the images changes, the dialog asks whether to
*Move existing images* or to *Make them again*, which deletes the old ones to be made as
photos are shown. Changing only the size limit asks nothing.

**XMP export.** Edits are kept in the app whatever the XMP options. Both options are off
by default.

- **Write XMP:** after each write, a Lightroom-style sidecar is written next to each raw
  file: `DSCF0001.xmp`, or `DSCF0001.RAF.xmp` if *Sidecar file name* is set to the second
  form (used by darktable). The sidecar gets the rating, colour label, keywords, IPTC
  fields, place and GPS. An existing sidecar, such as one from another program holding
  develop settings, is merged, so what is in it stays. An existing sidecar that cannot be
  merged is left alone.
- **Allow changing original JPEG, TIFF and DNG files:** the XMP goes into those files
  rather than a sidecar. This needs *Write XMP*. Raw files still get sidecars. Without
  this option, original JPEG, TIFF and DNG files are never touched. PNG files never get
  XMP.

Lightroom reads XMP only when it imports a photo, or when you choose *Read Metadata from
Files*. So this is an export for another program to pick up, not a sync. A file that
cannot be written does not undo the edit.

**Edits.** Edits are queued as they are for a catalog. They are written with *Write to the
photo folders…* in the footer, or by themselves with Autosave on. They go to the app's own
store for the source, and the original files change only as the XMP options allow. Ratings,
colour tags, keywords, IPTC fields, place, GPS and albums can all be edited. Albums,
groups and smart albums work as they do for a catalog. The folders show in the Albums
section as read-only collections.

**Backups.** Before each write, the app saves its own edits for the source in
`%LOCALAPPDATA%\c1studio\folders-<id>\work\backups\`. The newest five are kept. Restore one
from *Backups…* in the source menu, as for a catalog. A backup holds the app's edits, not
the photo files.

**The command line.** `c1cat folders init` makes a definition, and `c1cat --source FILE`
opens one. See [CLI.md](CLI.md#photo-folders).

**Not in this version.**

- Moving, renaming or deleting files on disk.
- More than one source open at once.
- Full raw development: a raw file shows the JPEG its camera embedded.
- HEIC, video and CR3 files: they are not read.
- A file watcher: changes on disk show up at the next rescan.

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
| `h` | Hide the picked photos from every view; with hidden photos shown, unhide them |
| `0`–`5` | Rating (0 clears) |
| `-` `+` `*` | Red, green, yellow tag |
| Ctrl+K | Add a keyword (details panel shown) |
| Ctrl+C (or Ctrl+Shift+C) | Copy… from the focused photo |
| Ctrl+V, Ctrl+Alt+V, Ctrl+Shift+V | Paste (overwrite), merge, Paste… (choose what and how) |
| Delete | Move the picked photos (or the one in the viewer) to the [trash](#trash); in an album, take them out of it |
| Ctrl+Delete | Move the picked photos to the trash, in an album too |
| Ctrl+Shift+S | Copy to N selected: the focused photo's fields onto the other picked photos |
| Ctrl+Z; Ctrl+Y or Ctrl+Shift+Z | Undo; redo |
| Ctrl+wheel, Ctrl+plus, Ctrl+minus, Ctrl+0 | Bigger, smaller, default tiles |

Keys do not act while you type in a field.

## Where things are kept

- The library, the edit queue, the analysis and its lines (`library\analysis.redb`),
  hidden photos (`library\hidden.redb`), the trash (`library\bin.redb`), trips and their settings (`library\trips.redb`),
  the autosave setting and backups: `%LOCALAPPDATA%\c1studio\<catalog id>\`.
- Recent catalogs: `%APPDATA%\c1studio\sources.json`.
- The kinds of source offered: `%APPDATA%\c1studio\providers.json`.
- Photo folders: the same, under `%LOCALAPPDATA%\c1studio\folders-<id>\`, where `<id>` is
  the first eight characters of the source's `id` in its `.c1folders` file.
- The sidebar layout, tile size, sort order, stacking, the strip's height and the copy
  clipboard: the app's own browser storage.
