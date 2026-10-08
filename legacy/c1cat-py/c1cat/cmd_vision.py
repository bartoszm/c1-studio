"""vision batch / run / review / apply - the Gemini delegation pipeline."""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path

from . import cmd_keywords, cmd_meta, keywords, render, safety, selector, sheets, vision
from .render import say, summary, table


def batch(args, ctx) -> int:
    rows, query = selector.run(args, ctx.mirror)
    if not rows:
        say("(nothing matched)")
        return 0
    out = Path(args.out).expanduser()
    out.mkdir(parents=True, exist_ok=True)

    say(f"building contact sheets for {len(rows)} photo(s) ...")
    problems: list[str] = []
    built = sheets.build_all(
        rows, out / "sheets", con=ctx.mirror, grid=args.grid, long_edge=args.long_edge,
        tier=args.tier, label="both", progress=say,
        on_error=lambda pk, message: problems.append(f"{pk}: {message}"))

    manifest = sheets.write_manifest(built, out / "manifest.json", extra={
        "grid": args.grid, "tier": args.tier, "catalog": str(ctx.catalog),
        "selected": len(rows), "query": query.described,
    })

    vocabulary = [r[0] for r in ctx.mirror.execute(
        "SELECT path FROM kw WHERE n_variants > 0 ORDER BY n_variants DESC LIMIT 250")]

    tracker = {
        "task": args.task,
        "model": args.model,
        "language": args.lang,
        "created": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "catalog": str(ctx.catalog),
        "grid": args.grid,
        "tier": args.tier,
        "totals": {"sheets": len(built), "photos": sum(len(s.cells) for s in built)},
        "sheets": [{"file": s.path.name, "photos": len(s.cells), "status": "pending",
                    "attempts": 0} for s in built],
        "vocabulary": vocabulary,
        "problems": problems,
    }
    vision.save_tracker(out, tracker)
    (out / "results").mkdir(exist_ok=True)
    (out / "raw").mkdir(exist_ok=True)

    if render.use_json():
        render.emit({"command": "vision batch", "out": str(out), "sheets": len(built),
                     "photos": tracker["totals"]["photos"], "task": args.task})
        return 0
    summary([
        f"{len(built)} sheet(s), {tracker['totals']['photos']} photo(s) in {out}",
        f"task: {args.task}   model: {args.model}",
        f"next: c1cat vision run {out}",
    ])
    return 0


def run(args, ctx) -> int:
    out = Path(args.dir).expanduser()
    tracker = vision.load_tracker(out)
    manifest = json.loads((out / "manifest.json").read_text())
    label_map = sheets.labels_to_variants(manifest)
    known = {r[0] for r in ctx.mirror.execute("SELECT path FROM kw")}

    pending = [s for s in tracker["sheets"]
               if s["status"] != "done" or args.force]
    if args.max_sheets:
        pending = pending[: args.max_sheets]
    if not pending:
        say("every sheet is already done")
        return 0

    model = args.model or tracker.get("model", "gemini-3.1-pro-low")
    say(f"sending {len(pending)} sheet(s) to {model} ...")

    for entry in pending:
        sheet_path = out / "sheets" / entry["file"]
        prompt = vision.build_prompt(tracker["task"], sheet_path,
                                     vocabulary=tracker.get("vocabulary"),
                                     language=tracker.get("language", "en"))
        (out / "prompt.txt").write_text(prompt)
        entry["attempts"] = entry.get("attempts", 0) + 1
        try:
            result = vision.run_sheet(sheet_path, prompt, model=model, effort=args.effort,
                                      timeout=args.timeout)
        except Exception as exc:  # noqa: BLE001 - one bad sheet must not end the batch
            entry["status"] = "failed"
            entry["error"] = str(exc)[:300]
            vision.save_tracker(out, tracker)
            say(f"  {entry['file']}: FAILED - {exc}")
            continue

        (out / "raw" / f"{Path(entry['file']).stem}.txt").write_text(result["response"] or "")
        if not result["ok"]:
            entry["status"] = "failed"
            detail = result["error"] or (result["response"] or "")[:300]
            entry["error"] = detail
            vision.save_tracker(out, tracker)
            say(f"  {entry['file']}: FAILED ({result['status']}) - "
                + (detail[:200] if detail else "no parseable JSON in the reply"))
            continue

        items, warnings = vision.validate(result["parsed"], label_map, known)
        (out / "results" / f"{Path(entry['file']).stem}.json").write_text(
            json.dumps({"sheet": entry["file"], "items": items, "warnings": warnings},
                       indent=2, ensure_ascii=False) + "\n")
        entry.update({
            "status": "done", "conversation_id": result["conversation_id"],
            "duration_s": result["duration_s"], "usage": result["usage"],
            "n_items": len(items), "n_warnings": len(warnings),
        })
        entry.pop("error", None)
        vision.save_tracker(out, tracker)
        say(f"  {entry['file']}: {len(items)} photo(s) in {result['duration_s']}s"
            + (f", {len(warnings)} warning(s)" if warnings else ""))

    done = sum(1 for s in tracker["sheets"] if s["status"] == "done")
    failed = sum(1 for s in tracker["sheets"] if s["status"] == "failed")
    tokens = sum(s.get("usage", {}).get("total_tokens", 0) for s in tracker["sheets"])
    tracker["totals"].update({"done": done, "failed": failed, "tokens": tokens})
    vision.save_tracker(out, tracker)

    if render.use_json():
        render.emit({"command": "vision run", "done": done, "failed": failed, "tokens": tokens})
        return 0
    summary([f"{done} sheet(s) done, {failed} failed, {tokens:,} tokens",
             f"next: c1cat vision review {out}"])
    return 0 if not failed else safety.EXIT_EXTERNAL


def review(args, ctx) -> int:
    out = Path(args.dir).expanduser()
    items = vision.merged_items(out)
    if not items:
        say("no results yet - run 'c1cat vision run' first")
        return 0

    by_pk = {i["variant_pk"]: i for i in items}
    rows = ctx.mirror.execute(
        f"SELECT * FROM v WHERE variant_pk IN ({','.join('?' * len(by_pk))})", list(by_pk)).fetchall()
    lookup = {r["variant_pk"]: r for r in rows}

    kept = [i for i in items if (i.get("confidence") is None
                                 or i["confidence"] >= args.min_confidence)]
    proposed: dict[str, int] = {}
    for item in items:
        for keyword in item.get("keywords_proposed", []):
            proposed[keyword] = proposed.get(keyword, 0) + 1

    if render.use_json():
        render.emit({"command": "vision review", "count": len(items),
                     "above_confidence": len(kept), "min_confidence": args.min_confidence,
                     "proposed_new_keywords": proposed, "items": items})
        return 0

    body = []
    for item in kept[: args.limit]:
        row = lookup.get(item["variant_pk"])
        body.append([
            item["variant_pk"],
            row["filename"] if row else "?",
            f"{row['rating'] or '-'} -> {item.get('rating', '-')}" if row else "",
            ", ".join(item.get("keywords", [])) or "-",
            (item.get("description") or "")[:60],
            f"{item['confidence']:.2f}" if item.get("confidence") is not None else "-",
        ])
    table(["VAR#", "FILENAME", "RATING", "KEYWORDS", "DESCRIPTION", "CONF"], body, "rlllll")

    lines = [f"{len(items)} photo(s) analysed, {len(kept)} at or above confidence {args.min_confidence}"]
    ratings = {}
    for item in kept:
        if "rating" in item:
            ratings[item["rating"]] = ratings.get(item["rating"], 0) + 1
    if ratings:
        lines.append("ratings proposed: " + "  ".join(
            f"{k}*x{v}" for k, v in sorted(ratings.items(), reverse=True)))
    if proposed:
        lines.append(f"{len(proposed)} keyword(s) NOT in your vocabulary would be created: "
                     + ", ".join(sorted(proposed)[:12]))
    guesses = [i for i in kept if i.get("location_guess")]
    if guesses:
        lines.append(f"{len(guesses)} location guess(es) - these are never applied automatically")
    lines.append("Nothing has been written. Apply with: "
                 f"c1cat vision apply {out} --only keywords --apply")
    summary(lines)
    return 0


def apply(args, ctx) -> int:
    out = Path(args.dir).expanduser()
    items = [i for i in vision.merged_items(out)
             if i.get("confidence") is None or i["confidence"] >= args.min_confidence]
    if not items:
        say("nothing to apply at this confidence level")
        return 0
    fields = {f.strip() for f in args.only.split(",")} if args.only else {"rating", "keywords",
                                                                          "description"}

    plan_meta: list[tuple[int, dict]] = []
    plan_keywords: list[tuple[int, list[str]]] = []
    for item in items:
        values = {}
        if "rating" in fields and "rating" in item:
            values["rating"] = item["rating"] or None
        if "description" in fields and item.get("description"):
            values["description"] = item["description"]
        if "description" in fields and item.get("headline"):
            values["headline"] = item["headline"]
        if values:
            plan_meta.append((item["variant_pk"], values))
        if "keywords" in fields:
            chosen = list(item.get("keywords", []))
            if args.allow_new_keywords:
                chosen += item.get("keywords_proposed", [])
            if chosen:
                plan_keywords.append((item["variant_pk"], chosen))

    if not args.apply:
        say("DRY RUN - nothing will be written. Re-run with --apply to commit.\n")
        say(f"fields: {', '.join(sorted(fields))}")
        say(f"metadata updates: {len(plan_meta)} variant(s)")
        say(f"keyword updates:  {len(plan_keywords)} variant(s)")
        if not args.allow_new_keywords:
            skipped = sum(len(i.get("keywords_proposed", [])) for i in items)
            if skipped:
                say(f"{skipped} suggested keyword(s) are not in your vocabulary and are being "
                    "skipped; pass --allow-new-keywords to create them")

    touched = 0
    created: list[str] = []
    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        for variant_pk, values in plan_meta:
            touched += cmd_meta.write_fields(con, [variant_pk], values)
        for variant_pk, chosen in plan_keywords:
            for path in chosen:
                keywords.ensure_path(con, path, created=created)
            existing = con.execute(
                "SELECT ZCONTENT_KEYWORDS FROM ZVARIANTMETADATA WHERE ZLAYER = "
                "(SELECT ZADJUSTMENTLAYER FROM ZVARIANT WHERE Z_PK = ?)", (variant_pk,)).fetchone()
            pairs = keywords.add_paths(keywords.parse(existing[0] if existing else None), chosen)
            touched += cmd_keywords._write_variant_keywords(con, variant_pk, pairs)
        problems = keywords.check_tree(con)
        if problems:
            raise safety.Refused("keyword tree would be inconsistent: " + "; ".join(problems),
                                 safety.EXIT_VERIFY)

    if args.apply:
        (out / "applied.json").write_text(json.dumps(
            {"applied_at": time.strftime("%Y-%m-%dT%H:%M:%S"), "fields": sorted(fields),
             "metadata": [{"variant_pk": pk, "values": v} for pk, v in plan_meta],
             "keywords": [{"variant_pk": pk, "keywords": k} for pk, k in plan_keywords]},
            indent=2, ensure_ascii=False) + "\n")

    summary([f"{len(plan_meta)} metadata update(s), {len(plan_keywords)} keyword update(s), "
             f"{touched} rows",
             f"new keyword node(s): {', '.join(sorted(set(created)))}" if created else "",
             "Committed. Reopen Capture One to see the changes." if wctx["applied"]
             else "Nothing written. Re-run with --apply to commit."])
    return 0


def register(sub, write_flags) -> None:
    parser = sub.add_parser("vision", help="have Gemini look at the photos")
    inner = parser.add_subparsers(dest="vision_command", required=True)

    p = inner.add_parser("batch", help="build contact sheets and a job folder")
    selector.add_arguments(p)
    p.add_argument("--out", required=True, metavar="DIR")
    p.add_argument("--task", default="describe", choices=list(vision.TASKS))
    p.add_argument("--grid", type=int, default=6)
    p.add_argument("--long-edge", type=int, default=2000)
    p.add_argument("--tier", default="thumb", choices=["thumb", "preview", "original"])
    p.add_argument("--model", default="gemini-3.1-pro-low")
    p.add_argument("--lang", default="en", choices=["en", "pl", "both"])
    p.set_defaults(func=batch)

    p = inner.add_parser("run", help="send the sheets to the model")
    p.add_argument("dir", metavar="DIR")
    p.add_argument("--model")
    p.add_argument("--effort", choices=["low", "medium", "high"],
                   help="only for models whose name does not already end in -low/-medium/-high")
    p.add_argument("--timeout", type=int, default=600)
    p.add_argument("--max-sheets", type=int)
    p.add_argument("--force", action="store_true", help="redo sheets already done")
    p.set_defaults(func=run)

    p = inner.add_parser("review", help="read the results before writing anything")
    p.add_argument("dir", metavar="DIR")
    p.add_argument("--min-confidence", type=float, default=0.0)
    p.add_argument("--limit", type=int, default=60)
    p.set_defaults(func=review)

    p = inner.add_parser("apply", help="write approved results back to the catalog")
    p.add_argument("dir", metavar="DIR")
    p.add_argument("--only", help="comma-separated: rating,keywords,description")
    p.add_argument("--min-confidence", type=float, default=0.7)
    p.add_argument("--allow-new-keywords", action="store_true")
    write_flags(p)
    p.set_defaults(func=apply)
