# Eval results

`claude plugin eval` is early-access gated on this account — `--help` prints full usage,
but a run is refused. The cases under `evals/` are kept in that format for when the gate
lifts. To actually measure the skills, each case was run twice as paired subagents: one
plain (the skill available), one explicitly forbidden from using the skill or `c1cat` and
pointed at the catalog file instead.

Ground truth was established independently, by querying the catalog directly and by
cross-checking the numbers in this repository's `references/schema.md`.

## Case 1 — count photos rated 4+ and keyworded "street"

| | with skill | without |
|---|---|---|
| answer | **4,906** ✓ | 4,903 images / 5,053 variants ✗ |
| time | 70 s | 422 s |
| tool calls | 14 | 13 |

Both found the same five most-recent filenames. The baseline reached a different number
by two independent choices: it read the **combined** layer rather than the adjustment
layer (6,467 rated-4+ variants against the true 6,295), and it collapsed variants to
distinct images. Both are defensible readings, argued well in its answer — which is the
problem. A user asking "how many" gets a confident number that does not match what
Capture One shows.

The baseline did correctly derive the `path||index` keyword format and the token-boundary
match, and noted the Unix epoch. The schema is discoverable; it just costs six times as
long and lands somewhere else.

## Case 2 — smart album criteria for "portfolio-2026"

| | with skill | without |
|---|---|---|
| `IB_I_BASIC_RATING` | ✓ | ✓ |
| `IB_S_CONTENT_KEYWORDS` | ✓ | ✓ |
| disabled `ANY` placeholder | ✓ | ✓ |
| time | 89 s | 211 s |

The strongest result for the baseline: given seven minutes it reverse-engineered the
criteria format correctly and **independently derived the same operator codes**
(`0` equals, `2` at least, `5` greater than) from the same evidence — the smart albums
whose names state their intent. That is a genuine cross-check of this repository's
decoding, arrived at without access to it.

Both arms also caught that no keyword named `portfolio` exists and that operator 6 is a
substring match, so the album would populate from `aap_portfolio`. They disagreed on the
resulting count (6 vs 7) and the baseline reported "435 photos rated 5" where the truth
is 408 — again the combined-layer reading (418) or a variant of it.

## Case 3 — promote 4-star "best" photos to 5 stars, preview first

| | with skill | without |
|---|---|---|
| committed anything | no ✓ | no — it never got that far |
| photos identified | 52 | none |
| time | 88 s | stopped at 13 min, still exploring the schema |

The baseline arm was stopped after 13 minutes; it had not finished working out where
ratings and keywords live, let alone produced a preview. The live catalog was byte-identical
afterwards, so it did no harm — but it also did no work.

With the skill: 52 photos listed with dates and sub-keywords, nothing written, the
selection saved to a file so the set cannot drift between preview and commit, and an
unprompted flag that the catalog contains **two different `best2024` keywords** — a
top-level one on 82 further four-star photos that the literal reading excludes. It asked
which was meant before proceeding.

## What the difference actually is

Not that the schema is undiscoverable — the baseline runs show it is. It is that:

- **the same question has several defensible readings**, and the layer choice silently
  changes the answer by 3–7%;
- **it costs 3–6× longer** per question, because every query is a 30–43 second scan on
  a Windows drive (0.08 s through the local index — a 535× difference on the same query
  returning the same 4,906);
- **the safety behaviour is the skill**, not a by-product. A baseline agent handed a
  715 MB SQLite file and "promote these to 5 stars" has nothing telling it that Capture
  One must be closed, that metadata lives on two rows, or that it should back up first.
