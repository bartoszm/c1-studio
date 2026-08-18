# Calling the Antigravity CLI

## The command

```bash
/home/bartoszm/.local/bin/agy \
  --print "<prompt>" \
  --add-dir <batch>/sheets \
  --model gemini-3.1-pro-low \
  --output-format json \
  --print-timeout 600s
```

The model reads images by absolute path named in the prompt; `--add-dir` grants access
to the folder. `~/.gemini/antigravity-cli/settings.json` already has
`allowNonWorkspaceAccess: true`.

## The envelope

```json
{"conversation_id": "...", "status": "SUCCESS", "response": "...",
 "duration_seconds": 27.9, "num_turns": 1,
 "usage": {"input_tokens": 18053, "output_tokens": 561, "total_tokens": 18614}}
```

On failure `status` is `ERROR`, `response` is empty and the reason is in a separate
`error` field — read that, or a failure looks like silence.

## Two traps

**Do not pass `--effort` with a model whose name ends in `-low`, `-medium` or `-high`.**
The suffix already carries the effort level and the CLI rejects the combination outright:

```
invalid model selection (--model "gemini-3.1-pro-low" --effort "medium"):
--model gemini-3.1-pro-low conflicts with --effort=medium
```

**Do not depend on `--json-schema`.** With a schema attached, the model answers in prose
and then runs a *separate* turn to populate the schema; that turn was observed returning
`{"photos": []}` while the prose answer was complete and correct. Silent data loss.

So results are extracted from the prose, in this order:

1. `structured_output`, if present and non-empty;
2. the last ```` ```json ```` fenced block in `response` — this is what the model does
   spontaneously when asked, and it is reliable;
3. the first balanced `{...}` in the response;
4. otherwise the sheet is marked failed, the raw reply is kept in `raw/`, and the batch
   continues.

Every prompt therefore ends with "Reply with a single fenced json block and nothing
else."

## Cost and pacing

Roughly 15–25k tokens per sheet of 36 photos, ~15–30 s each. Sheets run one at a time
and the tracker is saved after each, so interrupting costs at most one sheet. Tell the
user the sheet count before starting a large batch.

`gemini-3.1-pro-low` is the default and handles contact sheets well.
`gemini-3.1-pro-high` is available for harder judgement calls.
