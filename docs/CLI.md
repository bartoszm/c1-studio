# c1cat

`c1cat` is the c1-studio library from the command line: search a photo source (a
Capture One catalog, or plain photo folders), count and facet it, render its images, and
edit ratings, colour tags, captions, IPTC, GPS and keywords. Selection flags are spelled as the Python `c1cat` spelled them
(`legacy/c1cat-py`), so the two can be compared command for command.

```powershell
cargo build --release -p c1cat
$c1 = '.\target\release\c1cat.exe'
& $c1 --help
& $c1 find --help
```

## Source and library

Every command (except `bench`, `idbench` and `folders`) works on one source, the way the
desktop app opens one. Two options, accepted by every command, choose it:

- `--source PATH`: a Capture One catalog file, a folder holding one, or a `.c1folders`
  definition (or a folder holding one). `--catalog` is the old name and still works.
- `--provider ID`: use this provider (`capture-one`, `folders`) instead of the first that
  recognises the path; with no `--source`, only sources of that provider are considered.

The source is the first of:

1. `--source PATH`
2. `C1_CATALOG` (a Capture One catalog)
3. the source the app opened last (`%APPDATA%\c1studio\sources.json`)
4. the old Capture One search: `%APPDATA%\c1studio\config.json`
   (`{"catalog": "C:\\c1_catalogs\\fuji\\fiuji.cocatalogdb"}`), then the only
   `*.cocatalogdb` under `C:\c1_catalogs\*\`

A source that was asked for and cannot be opened is an error; the next step is not tried
in its place. When none is found, the error lists what was tried.

The library (redb store + tantivy index), thumbnails and write backups live in
`%LOCALAPPDATA%\c1studio\<source key>\`, the same folder the app uses, so the two share
them. For a Capture One catalog the key is the one `c1cat` always used (the catalog's
name and a hash of its path). The library takes a lock, so `c1cat` cannot run while the
desktop app has the same source open ("Database already open. Cannot acquire lock").

Every command except `bench`, `idbench` and `folders` syncs the library from the source
first; that is quick when nothing changed. Pass `--no-sync` to skip it.

## Reading

| Command | Does |
|---|---|
| `sync` | Pull the source into the library; prints the report as JSON. The first run indexes everything. |
| `rescan [--no-sync]` | Make the source look again for changes it cannot see by itself (photo files added, moved or removed on disk), then sync and print the report. `--no-sync` only asks; the next command's sync reads what it found. Capture One cannot do this, and says so. |
| `status` | Source name and provider, location, format and how far it is tested (read, write), whether writes are possible, library size. |
| `find` | List matching photos. |
| `count` | Print the number of matches. |
| `facets` | Count values per dimension over the matches. |
| `image` | Write JPEGs of matching photos. |

```powershell
& $c1 find --rating '>=4' --keyword street --limit 20
& $c1 find --album 'Best of 2025' --sort rating-desc --ids
& $c1 find --text '"old town" OR market' --json
& $c1 count --year 2026 --unkeyworded
& $c1 facets --year 2026 --dims rating,camera,keyword_tree --top 10
& $c1 image --tier preview -o out --rating 5 --limit 5 --size 2048
```

**find**

- `--sort`: `date`, `date-desc` (default), `rating`, `rating-desc`, `imported`,
  `imported-desc`, `filename`, `filename-desc`, `id`. A leading `-` also means
  descending (`-rating`).
- `--limit N`, `--offset N`: page through the matches; no limit lists them all.
- `--ids`: variant ids only, one per line (to feed `--id` of another command).
- `--json`: full photo records.
- Default output is one line per photo (id, capture time, filename, stars, keywords),
  and `N of TOTAL in X ms` on stderr.

**facets**

- `--dims`: comma-separated, default `rating,color,year,camera,keyword_tree`. Any of
  `keyword`, `keyword_tree`, `collection`, `camera`, `lens`, `country`, `state`, `city`,
  `year`, `month`, `rating`, `color`, `ext`, `style`, `flag`, `source`.
  `keyword_tree` counts a photo under every ancestor of its keywords; `keyword` only
  under the exact ones.
- `--top N`: values shown per dimension (default 15).

**trips**

Trips found from when and where photos were taken, one line each: id, kind (`found`,
`suggested`, `kept`, `dismissed`), first and last day, how many of its photos the
selection flags match of all it spans, and its name. Trips are found only once home is
set (the app's Trips section and this command share the setting).

- `--home 'Country|City'` sets home first (`--home Country`: the whole country, so only
  trips abroad; `--home none` clears it). With no home set, the command names the city
  photographed on the most days.
- `--gap-hours H`: a longer gap without photos starts a new trip (default 36).
- `--json`: the list with each trip's photo counts, cover photo and the settings.

```powershell
& $c1 trips --home 'Poland|Kraków'
& $c1 trips --year 2025 --rating '>=4'
```

**image**

- `--tier`: `thumb`, `preview` (default) or `original`. A thumbnail is the source's own
  or, for a source with none (photo folders), one made and kept in the library's
  `thumbs` folder, as the app does. A preview is the source's own (Capture One's,
  white-balanced here) or one the source makes from the original. `original` is the file
  itself (a raw gives the JPEG embedded in it).
- `-o, --out DIR` (required), `--size PX` for the longer edge (0 keeps the size),
  `--limit N` (default 10).
- Files are named `<id>-<tier>.jpg`. Photos with no image at that tier are reported on
  stderr and skipped.

## Selecting photos

The same flags work on `find`, `count`, `facets`, `image` and every photo edit. Every
flag given narrows the selection (AND). Trashed photos are left out unless asked for.

| Flag | Matches |
|---|---|
| `--id ID` | Variant id; repeatable. |
| `--image-id ID` | Image (asset) id, i.e. every variant of it; repeatable. |
| `--variant-index N` | Position among its image's variants; 0 is the original. |
| `--filename GLOB` | Filename glob, case-insensitive: `'DSCF*.RAF'`, `'IMG_00?2.jpg'`. |
| `--ext LIST` | Extensions, any of: `raf,dng`. |
| `--folder PATH` | Folder and everything below it; either slash direction. |
| `--rating R` | `3`, `>=4`, `<2`, `2-4`, `none` (unrated) or `any` (rated). |
| `--color LIST` | Colour tags, any of: `red,blue`; or `none` / `any`. Alias `--colour`. Colours are red, orange, yellow, green, blue, pink, purple. |
| `--keyword PATH` | Has this exact keyword; repeat to require all. Paths use `\|`: `transport\|taxi`. |
| `--keyword-any LIST` | Has any of these keywords: `night,private`. |
| `--keyword-under PATH` | Has this keyword or any keyword below it; repeatable. |
| `--no-keyword PATH` | Does not have this keyword; repeatable. |
| `--unkeyworded` | Has no keywords. |
| `--camera TEXT`, `--lens TEXT` | Substring, case-insensitive: `--camera x100`. |
| `--iso R`, `--aperture R`, `--focal R` | Ranges as for `--rating`: `--iso '>=3200'`, `--aperture '<=2.8'`, `--focal 23-35`. |
| `--from DATE`, `--to DATE` | Capture date, inclusive, local time. `YYYY`, `YYYY-MM` or `YYYY-MM-DD`; `--to 2025-06` runs to the end of June. |
| `--year YYYY`, `--month YYYY-MM` | Capture year or month. |
| `--imported-from DATE`, `--imported-to DATE` | Import date, as `--from` / `--to`. |
| `--album NAME` | In this album, smart album or folder collection; repeatable (in each). |
| `--not-in-album NAME` | Not in this collection; repeatable. |
| `--gps has\|none` | With or without coordinates. |
| `--near LAT,LON,KM` | Within a square of ±KM around the point (not a true radius). |
| `--city`, `--state`, `--country TEXT` | Substring of the IPTC place, case-insensitive. |
| `--text QUERY` | Free text over keywords, captions, IPTC and filename. Words are AND-ed; `"a phrase"`, `a OR b`. |
| `--edited`, `--unedited` | Develop settings differ from the defaults, or not. |
| `--style TEXT` | Applied style, substring, case-insensitive. |
| `--has-local-layers` | Has local adjustment layers. |
| `--include-trashed`, `--only-trashed` | Take trashed photos too, or only them. |

`--album` takes `#875` (the collection id), an exact name, a name in any case, or the
end of a unique path. When a name matches several, the error lists them with their
ids. Smart albums select by their own rules.

In PowerShell, quote values with `>`, `<`, `|`, `*` or spaces: `--rating '>=4'`,
`--keyword 'transport|taxi'`.

## Editing

| Command | Writes |
|---|---|
| `rate --set 0-5` / `rate --clear` | Star rating; 0 and `--clear` both mean unrated. |
| `color --set NAME` | Colour tag; `none` clears. Alias `colour`. |
| `describe --description/--headline/--title TEXT` | Description (the caption), headline, and title: IPTC's Object Name, a short identifier, not a caption. |
| `iptc --set-creator/--set-copyright/--set-job-id/--set-instructions/--set-source TEXT` | IPTC contact and status fields. |
| `iptc --set-city/--set-state/--set-country/--set-country-code/--set-location TEXT` | IPTC location fields. |
| `geo --lat LAT --lon LON` / `geo --at '44°58′N 14°45′E'` / `geo --clear` | GPS. `--at` takes coordinates as written anywhere (decimal, N/S/E/W, degrees and minutes). Stored per image, so every variant of it moves. |
| `keyword add PATH...` / `keyword remove PATH...` | Keywords on the photos. Alias `kw`. |

Text fields change only when given; an empty value (`--headline ''`) clears the field.
The `iptc` place fields carry a `--set-` prefix so they never read as the `--city` /
`--country` filters.

```powershell
& $c1 rate --set 5 --id 1234 --id 1235
& $c1 color --set green --album 'Picks' --apply
& $c1 describe --description 'Kazimierz at dusk' --title 'KRK-2026-014' --id 1234 --apply
& $c1 iptc --set-job-id 'JOB-7' --set-source 'Archiwum' --album 'Picks' --apply
& $c1 iptc --set-city 'Kraków' --set-country Poland --folder 'H:\fotos\2026\krakow' --apply
& $c1 geo --lat 50.0614 --lon 19.9366 --filename 'DSCF04*' --apply
& $c1 kw add 'place|Kraków' street --year 2026 --city krak --apply
& $c1 kw remove draft --keyword draft --apply
```

`keyword add` creates missing keywords in the tree; a keyword name may not contain `,`.
`keyword remove` takes exactly the paths named off the photos (not the keywords under
them); the tree keeps them.

### Keyword tree

These change the tree itself, on every photo, and take no selection.

| Command | Does |
|---|---|
| `kw rename --path PATH --to NAME` | Rename a keyword everywhere. |
| `kw move --path PATH --to-parent PATH` / `--to-root` | Move a keyword and what is under it. |
| `kw merge --from PATH --from PATH --into PATH` | Join keywords into one, existing or new. |
| `kw group --path PATH --path PATH --under PATH` | Put keywords under a parent, existing or new. |
| `kw ungroup --path PATH` | Move a group's keywords up a level; the group goes if no photo carries it. |

### Albums

An album holds the photos put into it; a smart album holds none, only a rule. Albums are
named as the selector's `--album` names them: `#906`, a name, or the end of a path.

| Command | Does |
|---|---|
| `album create NAME [--parent G]` | Make an album, at the top level or in group (or project) `G`. |
| `group create NAME [--parent G]` | Make a group. |
| `smart create NAME [--parent G] [--match all\|any] [--rating R] [--keyword K]... [--year Y]` | Make a smart album. `--rating` is `'>=4'`, `'>3'` (written as at least 4) or `'5'`; `--keyword` matches any level of a keyword's name; conditions are joined by `--match` (all by default). |
| `smart rule ALBUM` + the same rule flags | Give a smart album a new rule. |
| `album rename ALBUM --to NAME` | Rename an album, smart album or group. |
| `album delete ALBUM [--force-children]` | Delete it (and, with the flag, a group's contents). Photos stay. |
| `album add ALBUM <selection> [--create [--parent G]]` | Put the matching photos into an album; `--create` makes it if there is none of that name. |
| `album remove ALBUM <selection>` | Take the matching photos out. |

```powershell
& $c1 group create 'trips' --apply
& $c1 album add 'japan 2026' --create --parent trips --keyword japan --year 2026 --apply
& $c1 album remove 'japan 2026' --rating 1 --apply
& $c1 smart create 'street 4+' --rating '>=4' --keyword street --apply
& $c1 album delete 'trips' --force-children --apply
```

`album add` positions the album first because `--album` is a filter here: `album add
portfolio --album street` copies *street* into *portfolio*. A name already used in the
same place is refused unless `--allow-duplicate`; Capture One's own *Recent Captures*
and *Recent Imports* are never changed, and projects are deleted in Capture One.

### Backups

| Command | Does |
|---|---|
| `backups [--json]` | The backups made before writes, newest first: when, size, where. |
| `restore BACKUP [--apply]` | Put a backup (its name, or the end of its path) back as the source. The source as it is now is backed up first, and the path printed; restoring that undoes it. Capture One must be closed. |

### How writes work

- **Dry run by default.** Without `--apply` nothing is written: the command prints how
  many photos would change, how many already hold the value, and any conflicts.
- **A selection is required.** Photo edits refuse to run with no filter, so a bare
  `rate --set 0` cannot touch the whole catalog.
- **Capture One must be closed** for `--apply` on a catalog (a photo folders source has
  no such program to close). Writes are also refused while another
  program holds the catalog, a rollback journal is present, or the catalog's schema is
  not the one the writer was verified against; `status` shows which (`writes Ready`).
- **Backup first.** Before writing, the catalog is copied to
  `%LOCALAPPDATA%\c1studio\<source key>\work\backups\`; the path is printed. Writes in a
  row within an hour share one backup if nothing else touched the catalog meanwhile.
  The newest 5 are kept.
- **Conflicts.** A change is planned against the value the library last saw. If the
  catalog now holds something else, the change is reported as a conflict and skipped.
- `--json` prints the outcome as JSON: `applied`, `backup`, and the ids to `apply`,
  `unchanged` and in `conflicts` (album commands add `collections`, what they do).

## Photo folders

A photo folders source is a few folders of JPEG, PNG, TIFF and raw files, described by a
definition file (`*.c1folders`). The definition lists the folders (roots), what to leave
out and where thumbnails and previews are kept; its path is the source's location. Photos
keep their id when files are moved or renamed, or a root is picked again, so the ratings,
keywords and albums made here stay with them.

Make a definition, then open it like any source:

```powershell
& $c1 folders init --name 'Family archive' --exclude '**/tmp/**' D:\Photos E:\Scans
# prints the file (by default in %APPDATA%\c1studio\sources) and:
#   hint: c1cat --source <file> sync
$def = 'C:\Users\me\AppData\Roaming\c1studio\sources\family-archive-1a2b3c4d.c1folders'
& $c1 --source $def sync
& $c1 --source $def find --rating '>=4' --limit 20
& $c1 --source $def rescan                  # after files were added on disk
```

The roots are walked when the source is opened and by `rescan`; in between, commands read
what was kept. Edits (`rate`, `keyword`, `album`, ...) are the same commands as for
Capture One. They are kept in the source's own store, on top of what the files say, and
still need `--apply`.

| Command | Does |
|---|---|
| `folders init --name NAME [--exclude GLOB]... [--out FILE] [--force] ROOT...` | Make a definition and save it, by default in `%APPDATA%\c1studio\sources`; `--out` is a file, or a folder to put it in. Prints the path. Refuses to replace a file unless `--force` (which gives the source a new id). |
| `folders show [PATH] [--json]` | Print a definition: roots, excludes, image store. Defaults to the one `--source` names. |
| `folders add-root ROOT [--label TEXT] [--file FILE]` | Add a folder. |
| `folders remove-root ROOT [--file FILE]` | Take a folder out (give its path, or its id from `show`). Its photos are kept, detached, and come back if it is added again. |
| `folders set-store central\|per-root\|beside\|custom [--limit-mb MB] [--thumb PATTERN --preview PATTERN] [--move\|--regenerate] [--file FILE]` | Where thumbnails and previews are kept: the app's data folder (limited to `--limit-mb`, 0 for none), `.c1studio` under each root, a `.previews` folder beside each photo, or your own path patterns (`custom`, which needs both `--thumb` and `--preview`, such as `{dir}/.previews/{stem}.{tier}.jpg`). The patterns are checked first, and, once the source has been read, against its photos: a store that would give two photos one file (a RAW and its JPEG with `{stem}`) is refused. If images were already made in the old place, one of `--move` (move them to where the new store puts them) or `--regenerate` (delete them; they are made again as needed) is required, and the command changes nothing without it. It prints how many were moved, deleted or failed. Close the app first: it holds the source's store open. |

`--file` defaults to the definition `--source` names. Excludes are globs matched against a
path below its root, with `/` separators: `**/tmp/**`, `*.tmp`. The `.c1studio` and
`.previews` folders are always left out.

### idbench

`idbench DIR [--limit N] [--parallel] [--json]` times the stable ids and looks for the risk in them.
A photo is told apart by its size and a hash of the first and last 64 KB of the file, so
each photo costs at most 128 KB of reading. `idbench` walks `DIR` for photo files (at
most `--limit`), keys them one at a time (or with `--parallel` on all cores at once, which
is how a scan does it), and reports:

- files and total size, seconds, files per second, and MB per second actually read;
- the keys more than one file has, split into files with the same content (copies, which
  is fine) and files with different content (two photos that would share an id: should
  be none);
- the time the whole-file hashes of those files took.

```powershell
& $c1 idbench D:\Photos --limit 5000
& $c1 idbench D:\Photos --json
```

Run it on a folder not read recently to see disk speed rather than the cache's; it does
not change anything.

## Analysis

`analyse` measures each photo's thumbnail (shape, main colours, light, focus, and a
hash for duplicates and bursts), only photos not measured yet or whose thumbnail
changed, then works out duplicate groups and series again.

```powershell
& $c1 analyse                 # all cores but one
& $c1 analyse --threads 4 --stats
```

`--stats` prints each measure's spread (percentiles) and how many photos fall in each
class, for checking the thresholds against a catalog.

## Benchmark

`bench` times the library against the stage-0 speed gate on synthetic photos; it does
not need a catalog.

```powershell
& $c1 bench                                   # 250k photos, 2k keywords, 500 queries per class
& $c1 bench --photos 50000 --queries 100 --dir C:\tmp\bench
```

`--photos`, `--keywords`, `--queries`, `--seed` set the data; `--dims` the facets counted
with every query. The library in `--dir` is reused across runs with the same size and
seed; `--rebuild` builds it again.

## Differential tests

`tools/difftest` runs the same commands through the Rust `c1cat` and the Python oracle
(`legacy/c1cat-py`, run in WSL) against the same catalog. Build the release binary first.

```powershell
cargo build --release -p c1cat
python tools/difftest/difftest.py                    # selector flags: compares the ids each side returns
python tools/difftest/difftest.py -k rating          # only cases whose name contains "rating"
python tools/difftest/writetest.py --work C:\tmp\wt  # edits on two copies, compared row by row
```

Both take `--catalog PATH` and `--rust EXE`. `difftest.py` exits 1 if a case differs,
apart from those listed in its `EXPECTED` as differing by design. `writetest.py` never
writes the catalog itself: it copies it twice into `--work` (allow about three times
its size), edits each copy, compares them and removes them unless `--keep` is given.
