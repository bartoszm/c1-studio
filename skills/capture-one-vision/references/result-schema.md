# The result contract

One object per photo, inside a `photos` array.

```json
{"photos": [
  {
    "label": "v273331 DSCF4365",
    "keep": true,
    "rating": 4,
    "description": "Two men share a cigarette outside a shuttered shopfront; late afternoon light rakes across the wall.",
    "headline": "Smoke break",
    "keywords": ["street", "men", "smoking"],
    "keywords_new": ["shopfront"],
    "location_guess": {"city": "Tel Aviv", "country": "Israel", "confidence": 0.4},
    "reject_reason": null,
    "confidence": 0.82
  }
]}
```

Each task fills a subset; a field the model cannot judge is omitted rather than guessed.

## How a label becomes a photo

Contact sheet cells are captioned `v<variant_pk> <filename>`, and `manifest.json` records
the mapping for every cell. Answers are matched against that manifest — never parsed out
of the label text and never trusted as a primary key.

This matters because filenames are not unique. A 100k catalog has repeats across folders,
and two variants of one image (a colour and a black-and-white treatment) share a filename
exactly. A real batch returned `DSCF0367` twice; the variant PKs kept them apart.

An unrecognised label is dropped with a warning and counted, not guessed at.

## Validation before anything is written

- `rating` is clamped to 0–5.
- Keywords are lowercased. Any containing `,` or `|` is rejected outright — either would
  corrupt every keyword string in the catalog.
- Keywords are split against the user's existing vocabulary: known ones go to `keywords`,
  everything else to `keywords_proposed`, which needs `--allow-new-keywords` to be
  written. This is what keeps the taxonomy from inflating.
- `confidence` below `--min-confidence` (0.7 by default at apply time) excludes the item
  from writing but still shows it in review.
- `location_guess` is never applied automatically. A city inferred from a 450px thumbnail
  is a data-quality hazard; it is surfaced for the user to accept with
  `c1cat iptc --set-city ... --set-country ...`.

## Files in a batch folder

```
batch.json      what was asked for
manifest.json   sheet -> cells -> variant_pk        (the mapping that matters)
sheets/         sheet_001.jpg ...
tracker.json    per-sheet status, attempts, tokens  (resume state)
raw/            the verbatim reply per sheet        (for diagnosing failures)
results/        validated items per sheet
applied.json    written only after a successful apply
```
