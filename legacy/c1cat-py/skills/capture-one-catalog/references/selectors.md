# The selector

One flag group, shared by `find`, `count`, `show`, `stats`, `paths`, every write
command, `preview export`, `sheets` and `vision batch`. All filters combine with AND.

Mutating commands refuse to run with no filter at all, so there is no way to rate the
whole catalog by accident.

## Identity

| flag | meaning |
|---|---|
| `--id PK` | variant primary key, repeatable |
| `--image-id PK` | image primary key, repeatable |
| `--uuid U` | variant UUID |
| `--ids-file FILE` | one variant PK per line |
| `--from-selection FILE` | a set saved earlier with `--save-selection` |

A *variant* is the unit of selection. One image can have several variants (a colour and
a black-and-white treatment of the same frame), each with its own rating and keywords.

## Filters

| group | flags |
|---|---|
| file | `--filename GLOB`, `--ext raf,dng`, `--folder PATH` (WSL or Windows form, matches the subtree) |
| rating | `--rating '>=4'` `'3'` `'2-4'` `none` `any` |
| colour | `--color red,blue` `none` `any` (`--colour` also works) |
| keywords | `--keyword P` (repeat = require all), `--keyword-any a,b`, `--keyword-under P` (P and everything below), `--no-keyword P`, `--unkeyworded` |
| camera | `--camera X100VI`, `--lens '23mm'`, `--iso 100-1600`, `--aperture '<=2.8'`, `--focal 23-35` |
| dates | `--from`/`--to` (capture date), `--year 2026`, `--month 2026-08`, `--imported-from`/`--imported-to` |
| albums | `--album NAME|#PK`, `--not-in-album NAME` |
| place | `--gps has|none`, `--near LAT,LON,KM`, `--city`, `--country`, `--state` |
| text | `--text 'phrase'` — FTS5 over keywords, caption, headline, title, IPTC and filename |
| develop | `--edited`, `--unedited`, `--style NAME`, `--has-local-layers` |
| trash | `--include-trashed`, `--only-trashed` (excluded by default) |
| variants | `--variant-index 0` (0 is the original) |

Numeric filters accept `>=n`, `<=n`, `>n`, `<n`, `=n`, `n`, or `lo-hi`.

Free-text search ignores diacritics, so `wroclaw` finds `Wrocław`.

## Shaping

`--sort` takes `date`, `rating`, `filename`, `imported`, `id`, `random`, each with a
`-desc` variant (`date-desc` is the default). Also `--limit N` and `--offset N`.

Descending is spelled `date-desc` rather than `-date` because a leading dash would be
read as another flag. `--sort=-date` works too.

## Passing a selection between commands

```bash
c1cat find --rating '>=4' --keyword street --year 2026 --save-selection /tmp/pick.json
c1cat vision batch --from-selection /tmp/pick.json --task describe --out ~/batch
c1cat stage 'shortlist' --from-selection /tmp/pick.json --apply
```

This keeps a multi-step task on exactly the same photos even if the catalog changes
underneath, and lets the user refine the set once rather than repeating filters.

## Examples

```bash
# unkeyworded keepers from this year
c1cat find --rating '>=3' --year 2026 --unkeyworded

# everything shot wide open on the 23mm, best first
c1cat find --lens 23mm --aperture '<=2.0' --sort rating-desc --limit 40

# in an album but never edited
c1cat find --album 'do_pokazania' --unedited

# keywords under "best" but not "best2018"
c1cat find --keyword-under best --no-keyword 'best|best2018'

# what did I shoot near Warsaw
c1cat find --near 52.2297,21.0122,30 --fields variant_pk,filename,date,city

# see the query
c1cat count --rating '>=4' --keyword street --dump-sql
```
