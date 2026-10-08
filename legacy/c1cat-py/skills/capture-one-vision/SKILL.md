---
name: capture-one-vision
description: Have Gemini look at photos from a Capture One catalog and write the results back. Use when the user wants photos described, captioned, auto-keyworded, culled, rated or their location guessed by AI; when they ask what is in a set of photos; when they want a first pass over a shoot; or when they want contact sheets built from their catalog for review. Builds sheets from Capture One's own cached renders, sends them to Gemini through the Antigravity CLI in resumable batches, and only writes to the catalog after the user approves.
license: MIT
allowed-tools: Bash, Read, Glob
---

# Having Gemini look at the photos

Four stages, always in this order: **batch → run → review → apply.** The review stage
exists so the user sees what the model said before anything touches the catalog.

```
c1cat=${CLAUDE_PLUGIN_ROOT}/bin/c1cat
```

Selecting photos uses the same selector as the **capture-one-catalog** skill; read
that skill for the filter grammar.

On Windows, `c1cat` runs inside WSL: pass `--out` as a WSL path (`~/...`) and read the
paths it prints through `\\wsl.localhost\Ubuntu` (e.g. `\\wsl.localhost\Ubuntu\home\bartoszm\c1-batches\...`).

## 1. Build the batch

```bash
$c1cat vision batch --rating '>=3' --year 2026 --unkeyworded \
       --task keyword --out ~/c1-batches/2026-keywords
```

Creates contact sheets plus a job folder. `--task` is one of:

| task | what it asks for |
|---|---|
| `cull` | keep/drop, a rating, a reason when dropping |
| `describe` | a two-or-three sentence caption and a short headline |
| `keyword` | keywords, chosen from the user's existing vocabulary |
| `geo` | city and country, only where a landmark supports it |
| `all` | all of the above in one pass |

Other flags: `--grid` (default 6, so 36 photos per sheet), `--tier`
(`thumb` by default — Capture One's own 450px render, which already has the crop and
any black-and-white conversion applied), `--model`, `--lang en|pl|both`.

Tell the user the sheet count before running: each sheet is one model call of roughly
15–25k tokens.

## 2. Run

```bash
$c1cat vision run ~/c1-batches/2026-keywords
$c1cat vision run ~/c1-batches/2026-keywords --max-sheets 2   # try a couple first
```

Sheets are processed one at a time and the tracker is written after each, so this is
interruptible and `--resume` is implicit: re-running only picks up what is pending or
failed. For a large batch, run `--max-sheets 1` first and review it before committing
to the whole set.

Model names carry their effort level as a suffix (`gemini-3.1-pro-low`,
`gemini-3.1-pro-high`). Do not also pass `--effort` with those — the CLI rejects the
combination.

## 3. Review — never skip this

```bash
$c1cat vision review ~/c1-batches/2026-keywords
$c1cat vision review ~/c1-batches/2026-keywords --min-confidence 0.7 --json
```

Shows each photo's current value against what the model proposed, plus a summary of
proposed ratings, any keywords that are **not** in the user's vocabulary, and how many
location guesses came back. Present this to the user and let them decide.

## 4. Apply

```bash
$c1cat vision apply ~/c1-batches/2026-keywords --only keywords              # preview
$c1cat vision apply ~/c1-batches/2026-keywords --only keywords --apply      # commit
```

`--only` takes `rating`, `keywords`, `description` — apply one class at a time so the
user can judge each on its own. `--min-confidence` defaults to 0.7.

Writes go through exactly the same path as manual edits, so the Capture One guard, the
backup and the dry run all apply. `applied.json` records what was written.

Two deliberate limits:

- **New keywords are not created by default.** Anything outside the user's existing
  vocabulary is listed as proposed and needs `--allow-new-keywords`. This is what stops
  a 559-keyword taxonomy turning into 3000.
- **Location guesses are never applied.** They are shown so the user can decide, and
  set with `c1cat iptc --set-city ... --set-country ...` if they agree.

For culling, prefer staging over rating: `c1cat stage 'ai-keep' --from-selection ...`
puts the keepers in an album and leaves the judgement with the user.

## Working at full resolution

Contact sheets are for the wide pass. When the user wants a considered opinion on a
few frames, send them one per sheet at preview resolution:

```bash
$c1cat find --album shortlist --save-selection /tmp/short.json
$c1cat vision batch --from-selection /tmp/short.json --grid 1 --tier preview \
       --task describe --out ~/c1-batches/shortlist
```

That is the established workflow: sheets → wide cull → full-resolution second look on
the borderline frames → per-photo results.

## When something fails

Failures are per-sheet and recorded in `tracker.json`; the rest of the batch continues.

- `agy` not found — the Antigravity CLI is expected at `~/.local/bin/agy`.
- `invalid model selection ... conflicts with --effort` — drop `--effort`.
- "no parseable JSON in the reply" — the raw reply is kept in `raw/`; read it, then
  re-run that sheet with `--force`.
- A label the model invented is dropped with a warning rather than guessed at. Cells
  are labelled `v<variant_pk> <filename>` and matched back through `manifest.json`,
  which is why two variants of the same file never get confused.

`references/prompts.md` has the task prompts, `references/result-schema.md` the result
contract, and `references/agy-invocation.md` the exact command line and its failure
modes.
