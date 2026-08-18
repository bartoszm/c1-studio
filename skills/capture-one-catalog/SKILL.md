---
name: capture-one-catalog
description: Search, tag and organise a Capture One catalog from the command line. Use when the user asks about their photo catalog or their photos by rating, colour tag, keyword, camera, lens, date or place; when they want to rate, colour-tag, keyword, caption, geo-locate photos or put them into albums and smart albums; when they ask which photos are edited or what develop settings were applied; or when they mention Capture One, a cocatalogdb, or their fuji catalog. Reads are instant, every write previews first and refuses to run while Capture One is open.
license: MIT
allowed-tools: Bash, Read, Glob, Grep
---

# Working with a Capture One catalog

A Capture One catalog is a SQLite database. Capture One for Windows has no scripting
API, so the database is the only programmable surface. Everything here goes through
`c1cat`, a CLI that owns the schema details, the safety rules and the output shapes.

```
c1cat() { "${CLAUDE_PLUGIN_ROOT}/bin/c1cat" "$@"; }
```

Use `${CLAUDE_PLUGIN_ROOT}/bin/c1cat` in every command. Do not query the catalog with
`sqlite3` directly: the schema is unobvious in ways that produce confidently wrong
answers (see "Why not raw SQL" below).

## Start here

```bash
${CLAUDE_PLUGIN_ROOT}/bin/c1cat status
```

Reports the catalog path, its version, whether Capture One is running, whether the
local index is current, and how much is in the catalog. Run it first in a session.
If something looks wrong, `c1cat doctor` checks the whole environment.

The first command of a session may pause ~25s to build a local index (the catalog
lives on a Windows drive where a single scan costs 30s; the index makes queries
~0.1s). It rebuilds automatically whenever Capture One has changed the catalog.

## The two rules that matter

**1. Close Capture One before writing.** It keeps the whole catalog in memory and
rewrites it on save, so anything written underneath a running Capture One is silently
lost. `c1cat` refuses to commit while it detects Capture One, and tells the user to
reopen it afterwards. Never work around this with `--force` unless the user explicitly
says Capture One is closed.

**2. Every write is a dry run unless `--apply` is passed.** The dry run performs the
real changes and rolls them back, so the preview is exactly what would happen. Show
the user the preview, then apply. A commit backs the catalog up first.

## Finding photos

One selector grammar is shared by every command. Filters combine with AND.

```bash
c1cat find --rating '>=4' --keyword street --limit 20
c1cat find --camera X100VI --year 2026 --unkeyworded --sort date
c1cat find --album 'for street' --color red --fields variant_pk,filename,keywords,camera
c1cat find --text 'wroclaw'                     # free text over keywords, caption, IPTC
c1cat find --near 52.2297,21.0122,25            # within 25 km
c1cat count --rating 5 --edited
c1cat stats --by keyword --rating '>=4'         # also: camera, lens, year, album, style, color
```

Useful flags: `--rating '>=4'|'2-4'|none|any`, `--color red,blue|none|any`,
`--keyword` (repeat to require all), `--keyword-any`, `--keyword-under`,
`--no-keyword`, `--unkeyworded`, `--camera`, `--lens`, `--iso`, `--aperture`,
`--focal`, `--from`/`--to`, `--year`, `--month`, `--album`, `--not-in-album`,
`--folder`, `--gps has|none`, `--city`/`--country`, `--edited`/`--unedited`,
`--style`, `--filename` (glob), `--ext`, `--id`, `--sort`, `--limit`.

Trashed photos are excluded unless you pass `--include-trashed`.

`--json` on any command gives structured output. `--save-selection FILE` writes the
matched set so a later command can reuse it with `--from-selection FILE` — use this
to keep a multi-step task consistent while the user refines what they want.

The full flag list with worked examples is in `references/selectors.md`.

## Changing photos

```bash
c1cat rate     --album 'for street' --rating 3 --set 4          # preview
c1cat rate     --album 'for street' --rating 3 --set 4 --apply  # commit
c1cat rate     --id 273331 --clear
c1cat color    --keyword portfolio --set blue --apply
c1cat describe --id 273331 --description 'Two men share a cigarette.' --apply
c1cat iptc     --album iceland --set-city Reykjavik --set-country Iceland --apply
c1cat geo      --album iceland --lat 64.1466 --lon -21.9426 --apply
```

`--set-*` writes an IPTC field; the bare `--city`/`--country` flags are *filters*.

GPS is stored per image, not per variant, so a geo write touches every variant of the
matched images. `c1cat` says so in the summary when that affects photos outside the
selection.

## Keywords

Keywords are hierarchical, written `parent|child`.

```bash
c1cat keyword tree                      # the hierarchy with usage counts
c1cat keyword list --unused
c1cat keyword add street 'best|best2026' --album 'do_pokazania' --apply
c1cat keyword remove street --id 273331 --apply
c1cat keyword set 35mm street --id 273331 --apply     # replace the whole set
c1cat keyword rename --path 'best|best2018' --to '2018' --apply
c1cat keyword merge --from bw --from 'b&w' --into 'b&w' --apply
c1cat keyword delete obsolete --strip-usages --apply
```

`keyword add` creates any missing keyword in the library, including intermediate
parents, and reports which nodes it created. Renaming or merging rewrites every
photo that used the old path — the summary says how many, and that can be tens of
thousands, so always show the preview first.

Details and the storage format are in `references/keywords.md`.

## Albums, groups and smart albums

```bash
c1cat album list --tree
c1cat album show 'for street'
c1cat album create 'portfolio 2026' --parent best --apply
c1cat album add 'portfolio 2026' --rating 5 --keyword street --apply
c1cat album remove 'portfolio 2026' --id 273331 --apply
c1cat stage 'to-edit' --rating 4 --unedited --apply     # create if needed, then add
c1cat group create trips --apply
c1cat smart create 'street 5' --rating '>=5' --keyword street --apply
c1cat smart show 'street 4+' --xml
```

Album membership is stored in two tables at once; `c1cat` keeps them consistent.
Deleting a collection never touches the photos themselves.

Smart-album criteria use Capture One's own condition vocabulary, only part of which
is decoded. `--rating`, `--keyword` and `--year` are known good. For anything else,
have the user build one smart album by hand in Capture One, read it back with
`c1cat smart show NAME --xml`, and reuse it via `--raw-condition KEY:OP:VALUE`.
See `references/collections.md`.

## What was edited

Read-only: `c1cat` reports develop settings but never changes them. Rendering a
raw file is Capture One's job.

```bash
c1cat develop styles                          # which styles are used, and how often
c1cat develop report --rating 5               # how many are edited, top styles
c1cat develop report --id 267113 --verbose    # per-slider, against that file's defaults
c1cat develop diff --a 267113 --b 267114
```

The typical request — "which of my good photos still need work" — ends by staging a
selection for the user to open in Capture One:

```bash
c1cat find --rating '>=4' --unedited --count
c1cat stage 'to-edit 2026' --rating '>=4' --unedited --apply
```

## Getting at the pixels

```bash
c1cat paths --album iceland --tier original   # file paths on disk
c1cat preview export --id 267113 --tier preview --long-edge 1600 --out /tmp/look
c1cat sheets --rating 5 --keyword street --limit 36 --out /tmp/sheets
```

Three tiers: `thumb` (450px, instant, shows the applied edit), `preview`
(1920–2560px), `original` (the raw file, no edits). `--tier` picks one and falls
back when a file is missing; the summary reports which tier each photo used.

To have Gemini look at photos, use the **capture-one-vision** skill instead.

## Why not raw SQL

Four things make hand-written queries wrong in ways that still return a number:

- User metadata is stored **twice**, on the adjustment layer and the combined layer,
  reached through `ZVARIANT.ZADJUSTMENTLAYER` and `ZVARIANT.ZCOMBINEDSETTINGS`. Read
  the wrong one and ratings differ by ~900 photos; write only one and Capture One
  shows stale values.
- Unrated is `NULL`, not `0`.
- Keywords are a comma-separated `path||index` string, so `LIKE '%street%'` also
  matches `streetlight` and `street art`.
- The catalog is on a Windows drive: a `LIKE` scan takes ~30 s.

`c1cat --dump-sql` shows the compiled query if the user wants to see it, and
`references/schema.md` documents the schema for genuine one-offs.

## Reference files

- `references/selectors.md` — every selector flag, with examples
- `references/keywords.md` — the serialisation format and the keyword tree
- `references/collections.md` — albums and smart-album criteria
- `references/develop-settings.md` — how develop values are stored
- `references/schema.md` — the verified `Z*` schema
- `references/safety.md` — the write protocol, backups and recovery
