# capture-one

Work with a Capture One catalog from the command line, and let Claude do it for you.

A Capture One catalog is a SQLite database, and Capture One for Windows has no scripting
API — so the database is the only programmable surface. This repository is both:

- **`c1cat`**, a Python CLI that owns the schema details and the safety rules;
- **a Claude Code plugin** with two skills (`capture-one-catalog`, `capture-one-vision`)
  that drive it.

## Install

```bash
ln -s /home/bartoszm/capture_one_playground ~/.claude/skills/capture-one
echo '{"catalog": "/mnt/c/c1_catalogs/fuji/fiuji.cocatalogdb"}' > ~/.config/c1cat/config.json
./bin/c1cat doctor
```

`doctor` reports anything missing. Optional: `pip install imagecodecs` for the JPEG-XL
preview tier (contact sheets work without it).

## Use

```bash
c1cat=./bin/c1cat

$c1cat status
$c1cat find --rating '>=4' --keyword street --limit 20
$c1cat stats --by keyword --rating 5
$c1cat rate --album 'for street' --rating 3 --set 4          # preview
$c1cat rate --album 'for street' --rating 3 --set 4 --apply  # commit
$c1cat keyword add street 'best|best2026' --id 273331 --apply
$c1cat stage 'to-edit' --rating '>=4' --unedited --apply
$c1cat develop report --rating 5
$c1cat sheets --rating 5 --limit 36 --out /tmp/sheets
$c1cat vision batch --year 2026 --unkeyworded --task keyword --out ~/batch
```

Two rules the tool enforces:

1. **Close Capture One before writing.** It caches the catalog in memory and rewrites it
   on save, so external edits made while it is open are lost. `c1cat` refuses to commit
   while it detects Capture One running.
2. **Every write previews first.** `--apply` commits; without it you get a dry run that
   performs the change and rolls it back, so the preview is the truth. The first commit
   in a process backs the catalog up to `~/.local/share/c1cat/backups/`.

## How it is put together

```
c1cat/
  config.py      catalog discovery, colour names, entity ids
  source.py      read-only / read-write handles on the live catalog
  mirror.py      the local index: a byte copy plus a denormalised table, FTS5 and R*Tree
  selector.py    one filter grammar, shared by every command
  safety.py      the guard chain, backups, the write transaction
  keywords.py    the two serialisation orders and the nested-set surgery
  collections.py albums, groups, smart-album criteria
  develop.py     read-only reporting, diffed against each file's own defaults
  images.py      thumb / preview / original tiers
  sheets.py      contact sheets (adapted from photo_analyser/create_sheets.py)
  vision.py      Gemini delegation through the Antigravity CLI
skills/          the two SKILL.md files and their references
evals/           `claude plugin eval` cases
tests/           48 unit tests against the real DDL
```

The catalog lives on a Windows drive where one scan costs ~30 s. `c1cat` copies it to
WSL-native disk (~6 s) and indexes it (~23 s total), which takes the same query to
0.13 s. The index rebuilds itself whenever Capture One has changed the catalog.

## What is verified, and what is not

Checked against the live catalog: the two-metadata-row rule, both keyword orderings
(against all 62 292 non-empty values), the nested-set arithmetic, album membership,
smart-album operator codes, GPS formatting, the preview and thumbnail cache paths, and a
full write round-trip on a copy — after which `PRAGMA integrity_check` passes and the
nested set is still contiguous.

**Not yet confirmed:** that Capture One *displays* those writes. Open a copy of the
catalog in Capture One and check the rating, colour tag, keyword hierarchy, caption and
album before trusting the tool with the real one. Also unconfirmed: that the Windows lock
probe reports `LOCKED` while Capture One is running — it does correctly detect an open
handle, but the process check is the primary guard.

## Tests

```bash
python3 -m pytest tests/ -q
```
