# c1cat

`c1cat` is the c1-studio library from the command line: search a Capture One catalog,
count and facet it, render its images, and edit ratings, colour tags, captions, IPTC,
GPS and keywords. Selection flags are spelled as the Python `c1cat` spelled them
(`legacy/c1cat-py`), so the two can be compared command for command.

```powershell
cargo build --release -p c1cat
$c1 = '.\target\release\c1cat.exe'
& $c1 --help
& $c1 find --help
```

## Catalog and library

The catalog is the first of:

1. `--catalog PATH` (accepted by every command)
2. `C1_CATALOG`
3. `%APPDATA%\c1studio\config.json`: `{"catalog": "C:\\c1_catalogs\\fuji\\fiuji.cocatalogdb"}`
4. the only `*.cocatalogdb` under `C:\c1_catalogs\*\`

The library (redb store + tantivy index) and write backups live in
`%LOCALAPPDATA%\c1studio\<catalog id>\`. The library takes a lock, so `c1cat` cannot
run while the desktop app has the same catalog open ("Database already open. Cannot
acquire lock").

Every command except `bench` syncs the library from the catalog first; that is quick
when nothing changed. Pass `--no-sync` to skip it.

## Reading

| Command | Does |
|---|---|
| `sync` | Pull the catalog into the library; prints the report as JSON. The first run indexes everything. |
| `status` | Catalog path, source schema, whether writes are possible, library size. |
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

**image**

- `--tier`: `thumb`, `preview` (default; Capture One's preview, white-balanced here) or
  `original` (the JPEG embedded in the raw).
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
| `geo --lat LAT --lon LON` / `geo --clear` | GPS. Stored per image, so every variant of it moves. |
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
| `backups [--json]` | The backups made before writes, newest first: when, size, file. |
| `restore BACKUP [--apply]` | Put a backup (its file name, or the end of it) back as the catalog. The catalog as it is now is backed up first, and the path printed; restoring that undoes it. Capture One must be closed. |

### How writes work

- **Dry run by default.** Without `--apply` nothing is written: the command prints how
  many photos would change, how many already hold the value, and any conflicts.
- **A selection is required.** Photo edits refuse to run with no filter, so a bare
  `rate --set 0` cannot touch the whole catalog.
- **Capture One must be closed** for `--apply`. Writes are also refused while another
  program holds the catalog, a rollback journal is present, or the catalog's schema is
  not the one the writer was verified against; `status` shows which (`writes Ready`).
- **Backup first.** Before writing, the catalog is copied to
  `%LOCALAPPDATA%\c1studio\<catalog id>\work\backups\`; the path is printed. Writes in a
  row within an hour share one backup if nothing else touched the catalog meanwhile.
  The newest 5 are kept.
- **Conflicts.** A change is planned against the value the library last saw. If the
  catalog now holds something else, the change is reported as a conflict and skipped.
- `--json` prints the outcome as JSON: `applied`, `backup`, and the ids to `apply`,
  `unchanged` and in `conflicts` (album commands add `collections`, what they do).

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
