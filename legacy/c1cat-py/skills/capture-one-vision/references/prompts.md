# Task prompts

Built by `c1cat/vision.py`. Every prompt shares a preamble that explains the sheet
layout, insists the printed label be copied verbatim, tells the model to omit rather
than guess, and demands a single fenced JSON block.

## cull

Keep or drop, a 1–5 rating, a short reason when dropping, and a confidence. Judged on:
is the subject sharp where it matters, is the moment or gesture strong, is the
composition deliberate, is the exposure recoverable.

Culling results are best staged into an album rather than written as ratings — the
judgement stays with the photographer.

## describe

Two or three sentences on what is happening, the light and the mood, written as a
curator would for a portfolio, plus a headline of at most five words.

## keyword

The prompt embeds the photographer's actual vocabulary — the 250 most-used keyword paths
from `c1cat keyword list` — and asks the model to choose from it, putting anything
genuinely new in `keywords_new`. Supplying the vocabulary is what makes the allow-list at
apply time a formality rather than a filter that discards most of the output.

## geo

City and country only where a landmark, a sign or unmistakable architecture supports it.
The model is told to leave the field out entirely when unsure, and the result is never
applied automatically.

## Language

`--lang en` (default), `pl`, or `both`. With `pl` every description and headline comes
back in Polish; with `both` the model adds a `description_pl` alongside the English.

Polish place names and idiom come out noticeably better when the task itself is framed
in Polish, so prefer `--lang pl` for Polish subject matter.
