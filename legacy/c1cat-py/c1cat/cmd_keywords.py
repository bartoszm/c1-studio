"""keyword add / remove / set / list / tree / create / rename / move / merge / delete."""

from __future__ import annotations

import sys
from pathlib import Path

from . import keywords, render, safety, selector, source
from .keywords import SEP, KeywordError
from .render import say, summary, table


def _write_variant_keywords(con, variant_pk: int, pairs: list[tuple[str, int]]) -> int:
    """Write the same keyword set to both metadata rows, in each layer's own order."""
    kinds = source.layer_kinds(con, variant_pk)
    touched = 0
    for metadata_pk, ent in kinds.items():
        order = "path" if ent != 20 else "index"
        con.execute("UPDATE ZVARIANTMETADATA SET ZCONTENT_KEYWORDS = ? WHERE Z_PK = ?",
                    (keywords.serialise(pairs, order), metadata_pk))
        touched += 1
    return touched


def _select(args, ctx):
    if selector.is_empty(args):
        raise selector.SelectorError(
            "refusing to run without a selection - add at least one filter")
    rows, _ = selector.run(args, ctx.mirror)
    return rows


def _apply_to_selection(args, ctx, transform, label: str, wanted: list[str],
                        create: bool = False) -> int:
    rows = _select(args, ctx)
    if not rows:
        say("(nothing matched - no changes)")
        return 0

    plan = []
    for row in rows:
        before = keywords.parse(row["keywords_raw"])
        after = transform(before)
        if [p for p, _ in before] != [p for p, _ in after]:
            plan.append((row, before, after))

    if not render.use_json():
        if not args.apply:
            say("DRY RUN - nothing will be written. Re-run with --apply to commit.\n")
        table(["VAR#", "FILENAME", "KEYWORDS"],
              [[r["variant_pk"], r["filename"],
                f"{', '.join(p for p, _ in b) or '-'}  ->  {', '.join(p for p, _ in a) or '-'}"]
               for r, b, a in plan[:40]], "rll")
        if len(plan) > 40:
            say(f"... and {len(plan) - 40} more")

    created: list[str] = []
    touched = 0
    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        if create:
            for path in wanted:
                keywords.ensure_path(con, path, created=created)
        for row, _, after in plan:
            touched += _write_variant_keywords(con, row["variant_pk"], after)
        problems = keywords.check_tree(con)
        if problems:
            raise safety.Refused("keyword tree would be left inconsistent: " + "; ".join(problems),
                                 safety.EXIT_VERIFY)

    lines = [f"{label}: {len(plan)} of {len(rows)} variant(s) change, {touched} metadata rows"]
    if created:
        lines.append(f"new keyword node(s): {', '.join(created)}")
    lines.append("Committed. Reopen Capture One to see the changes." if wctx["applied"]
                 else "Nothing written. Re-run with --apply to commit.")

    if render.use_json():
        render.emit({"command": f"keyword {label}", "dry_run": not args.apply,
                     "applied": wctx["applied"], "changed": len(plan), "matched": len(rows),
                     "rows_touched": touched, "created_keywords": created,
                     "variant_pks": [r["variant_pk"] for r, _, _ in plan]})
    else:
        summary(lines)
    return 0


def cmd_add(args, ctx) -> int:
    wanted = [k.strip() for k in args.keyword_arg]
    for path in wanted:
        keywords.split_path(path)          # validates names up front
    return _apply_to_selection(
        args, ctx, lambda pairs: keywords.add_paths(pairs, wanted), "add", wanted, create=True)


def cmd_remove(args, ctx) -> int:
    wanted = [k.strip() for k in args.keyword_arg]
    return _apply_to_selection(
        args, ctx, lambda pairs: keywords.remove_paths(pairs, wanted, args.recursive),
        "remove", wanted)


def cmd_set(args, ctx) -> int:
    wanted = [k.strip() for k in args.keyword_arg]
    for path in wanted:
        keywords.split_path(path)
    return _apply_to_selection(
        args, ctx, lambda pairs: keywords.add_paths([], wanted), "set", wanted, create=True)


def cmd_clear(args, ctx) -> int:
    return _apply_to_selection(args, ctx, lambda pairs: [], "clear", [])


def cmd_list(args, ctx) -> int:
    con = ctx.mirror
    where = ""
    if args.used:
        where = "WHERE n_variants > 0"
    elif args.unused:
        where = "WHERE n_variants = 0"
    order = "n_variants DESC, path" if args.sort == "count" else "path"
    rows = con.execute(f"SELECT path, name, depth, n_variants FROM kw {where} ORDER BY {order}").fetchall()

    if render.use_json():
        render.emit({"command": "keyword list", "count": len(rows),
                     "keywords": [{"path": r["path"], "photos": r["n_variants"]} for r in rows]})
        return 0
    table(["KEYWORD", "PHOTOS"], [[r["path"], r["n_variants"]] for r in rows], "lr")
    summary([f"{len(rows)} keyword(s), {sum(1 for r in rows if r['n_variants'] == 0)} unused"])
    return 0


def cmd_tree(args, ctx) -> int:
    con = ctx.mirror
    rows = con.execute("SELECT pk, name, parent, path, n_variants FROM kw ORDER BY path").fetchall()
    children: dict[int | None, list] = {}
    for row in rows:
        children.setdefault(row["parent"], []).append(row)

    if render.use_json():
        def node(row):
            return {"path": row["path"], "name": row["name"], "photos": row["n_variants"],
                    "children": [node(c) for c in children.get(row["pk"], [])]}
        render.emit({"command": "keyword tree",
                     "keywords": [node(r) for r in children.get(None, [])]})
        return 0

    lines = []

    def walk(parent, prefix=""):
        group = children.get(parent, [])
        for i, row in enumerate(group):
            last = i == len(group) - 1
            branch = "└─ " if last else "├─ "
            label = f"{prefix}{branch if prefix or parent else ''}{row['name']}"
            lines.append([label, row["n_variants"]])
            walk(row["pk"], prefix + ("   " if last else "│  ") if parent else "")

    walk(None)
    table(["KEYWORD", "PHOTOS"], lines, "lr")
    parents = sum(1 for r in rows if children.get(r["pk"]))
    summary([f"{len(rows)} keyword(s), {parents} with children"])
    return 0


def cmd_create(args, ctx) -> int:
    path = args.path
    keywords.split_path(path)
    created: list[str] = []
    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        keywords.ensure_path(con, path, created=created)
        problems = keywords.check_tree(con)
        if problems:
            raise safety.Refused("keyword tree would be inconsistent: " + "; ".join(problems),
                                 safety.EXIT_VERIFY)
    if render.use_json():
        render.emit({"command": "keyword create", "dry_run": not args.apply,
                     "applied": wctx["applied"], "created": created})
    else:
        say(f"{'created' if wctx['applied'] else 'would create'}: {', '.join(created) or '(already exists)'}")
        summary(["Committed." if wctx["applied"] else "Nothing written. Re-run with --apply."])
    return 0


def _rewrite_paths(con, mapping: dict[str, str], variant_pks: list[int]) -> int:
    """Rewrite keyword paths in the metadata of the given variants."""
    touched = 0
    for pk in variant_pks:
        kinds = source.layer_kinds(con, pk)
        pairs = None
        for metadata_pk, ent in kinds.items():
            row = con.execute(
                "SELECT ZCONTENT_KEYWORDS FROM ZVARIANTMETADATA WHERE Z_PK = ?", (metadata_pk,)).fetchone()
            if pairs is None:
                pairs = keywords.parse(row[0])
                renamed = []
                for path, index in pairs:
                    for old, new in mapping.items():
                        if path == old:
                            path = new
                            break
                        if path.startswith(old + SEP):
                            path = new + path[len(old):]
                            break
                    renamed.append((path, index))
                # A rename can collide with an existing keyword; keep one copy.
                seen, deduped = set(), []
                for path, index in sorted(renamed, key=lambda t: t[1]):
                    if path not in seen:
                        seen.add(path)
                        deduped.append((path, index))
                pairs = keywords.renumber(deduped)
            order = "path" if ent != 20 else "index"
            con.execute("UPDATE ZVARIANTMETADATA SET ZCONTENT_KEYWORDS = ? WHERE Z_PK = ?",
                        (keywords.serialise(pairs, order), metadata_pk))
            touched += 1
    return touched


def _affected(con_mirror, path: str) -> list[int]:
    rows = con_mirror.execute(
        "SELECT variant_pk FROM v WHERE keywords_norm LIKE ? OR keywords_norm LIKE ?",
        (f"%\x1f{path}\x1f%", f"%\x1f{path}|%")).fetchall()
    return [r[0] for r in rows]


def cmd_rename(args, ctx) -> int:
    old, new_name = args.path, keywords.validate_name(args.to)
    node_rows = ctx.mirror.execute("SELECT pk, parent, path FROM kw WHERE path = ?", (old,)).fetchone()
    if node_rows is None:
        raise KeywordError(f"no keyword at path {old!r}")
    parent_path = old.rsplit(SEP, 1)[0] if SEP in old else None
    new_path = f"{parent_path}{SEP}{new_name}" if parent_path else new_name
    variant_pks = _affected(ctx.mirror, old)

    say(f"{'renaming' if args.apply else 'would rename'} {old!r} -> {new_path!r} "
        f"({len(variant_pks)} photo(s) affected)")
    if not args.apply:
        say("DRY RUN - nothing will be written. Re-run with --apply to commit.")

    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        con.execute("UPDATE ZKEYWORD SET ZNAME = ? WHERE Z_PK = ?", (new_name, node_rows["pk"]))
        touched = _rewrite_paths(con, {old: new_path}, variant_pks)
        problems = keywords.check_tree(con)
        if problems:
            raise safety.Refused("keyword tree would be inconsistent: " + "; ".join(problems),
                                 safety.EXIT_VERIFY)
    summary([f"{len(variant_pks)} photo(s), {touched} metadata rows",
             "Committed. Reopen Capture One to see the changes." if wctx["applied"]
             else "Nothing written. Re-run with --apply to commit."])
    return 0


def cmd_move(args, ctx) -> int:
    old = args.path
    node = ctx.mirror.execute("SELECT pk, name, path FROM kw WHERE path = ?", (old,)).fetchone()
    if node is None:
        raise KeywordError(f"no keyword at path {old!r}")
    new_parent_path = None if args.to_root else args.to_parent
    if not args.to_root and not args.to_parent:
        raise selector.SelectorError("pass --to-parent PATH or --to-root")
    new_path = f"{new_parent_path}{SEP}{node['name']}" if new_parent_path else node["name"]
    variant_pks = _affected(ctx.mirror, old)

    say(f"{'moving' if args.apply else 'would move'} {old!r} -> {new_path!r} "
        f"({len(variant_pks)} photo(s) affected)")
    if not args.apply:
        say("DRY RUN - nothing will be written. Re-run with --apply to commit.")

    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        parent_pk = keywords.ensure_path(con, new_parent_path) if new_parent_path else None
        if parent_pk == node["pk"]:
            raise KeywordError("a keyword cannot be its own parent")
        con.execute("UPDATE ZKEYWORD SET ZPARENT = ? WHERE Z_PK = ?", (parent_pk, node["pk"]))
        keywords.rebuild_tree(con)
        touched = _rewrite_paths(con, {old: new_path}, variant_pks)
        problems = keywords.check_tree(con)
        if problems:
            raise safety.Refused("keyword tree would be inconsistent: " + "; ".join(problems),
                                 safety.EXIT_VERIFY)
    summary([f"{len(variant_pks)} photo(s), {touched} metadata rows",
             "Committed. Reopen Capture One to see the changes." if wctx["applied"]
             else "Nothing written. Re-run with --apply to commit."])
    return 0


def cmd_merge(args, ctx) -> int:
    sources = [s.strip() for s in args.from_paths]
    target = args.into
    if ctx.mirror.execute("SELECT 1 FROM kw WHERE path = ?", (target,)).fetchone() is None:
        raise KeywordError(f"target keyword {target!r} does not exist - create it first")
    variant_pks = sorted({pk for s in sources for pk in _affected(ctx.mirror, s)})

    say(f"{'merging' if args.apply else 'would merge'} {', '.join(sources)} -> {target!r} "
        f"({len(variant_pks)} photo(s) affected)")
    if not args.apply:
        say("DRY RUN - nothing will be written. Re-run with --apply to commit.")

    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        touched = _rewrite_paths(con, {s: target for s in sources}, variant_pks)
        for path in sources:
            node = keywords.resolve(con, path)
            if node:
                con.execute("DELETE FROM ZKEYWORD WHERE ZLEFT >= ? AND ZRIGHT <= ?",
                            (node.left, node.right))
        keywords.rebuild_tree(con)
        problems = keywords.check_tree(con)
        if problems:
            raise safety.Refused("keyword tree would be inconsistent: " + "; ".join(problems),
                                 safety.EXIT_VERIFY)
    summary([f"{len(variant_pks)} photo(s), {touched} metadata rows",
             "Committed. Reopen Capture One to see the changes." if wctx["applied"]
             else "Nothing written. Re-run with --apply to commit."])
    return 0


def cmd_delete(args, ctx) -> int:
    path = args.path
    node = ctx.mirror.execute("SELECT pk, path FROM kw WHERE path = ?", (path,)).fetchone()
    if node is None:
        raise KeywordError(f"no keyword at path {path!r}")
    variant_pks = _affected(ctx.mirror, path) if args.strip_usages else []
    still_used = ctx.mirror.execute(
        "SELECT n_variants FROM kw WHERE path = ?", (path,)).fetchone()["n_variants"]
    if still_used and not args.strip_usages:
        raise KeywordError(
            f"{path!r} is still on {still_used} photo(s); pass --strip-usages to remove it from them too")

    say(f"{'deleting' if args.apply else 'would delete'} {path!r}"
        + (f" and remove it from {len(variant_pks)} photo(s)" if variant_pks else ""))
    if not args.apply:
        say("DRY RUN - nothing will be written. Re-run with --apply to commit.")

    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        touched = 0
        for pk in variant_pks:
            kinds = source.layer_kinds(con, pk)
            first = next(iter(kinds), None)
            if first is None:
                continue
            row = con.execute("SELECT ZCONTENT_KEYWORDS FROM ZVARIANTMETADATA WHERE Z_PK = ?",
                              (first,)).fetchone()
            pairs = keywords.remove_paths(keywords.parse(row[0]), [path], recursive=True)
            touched += _write_variant_keywords(con, pk, pairs)
        target = keywords.resolve(con, path)
        if target:
            con.execute("DELETE FROM ZKEYWORD WHERE ZLEFT >= ? AND ZRIGHT <= ?",
                        (target.left, target.right))
        keywords.rebuild_tree(con)
        problems = keywords.check_tree(con)
        if problems:
            raise safety.Refused("keyword tree would be inconsistent: " + "; ".join(problems),
                                 safety.EXIT_VERIFY)
    summary([f"{len(variant_pks)} photo(s), {touched} metadata rows",
             "Committed. Reopen Capture One to see the changes." if wctx["applied"]
             else "Nothing written. Re-run with --apply to commit."])
    return 0


def register(sub, write_flags) -> None:
    parser = sub.add_parser("keyword", aliases=["kw"], help="keywords on photos and in the library")
    inner = parser.add_subparsers(dest="keyword_command", required=True)

    def with_selection(name, help_text):
        p = inner.add_parser(name, help=help_text)
        # Positional, because --keyword is already a selector *filter* on these parsers.
        p.add_argument("keyword_arg", nargs="+", metavar="KEYWORD",
                       help="keyword path, e.g. street or 'best|best2018'")
        selector.add_arguments(p)
        write_flags(p)
        return p

    p = with_selection("add", "add keywords to the matching photos")
    p.set_defaults(func=cmd_add)

    p = with_selection("remove", "remove keywords from the matching photos")
    p.add_argument("--recursive", action="store_true", help="also remove child keywords")
    p.set_defaults(func=cmd_remove)

    p = with_selection("set", "replace the whole keyword set on the matching photos")
    p.set_defaults(func=cmd_set)

    p = inner.add_parser("clear", help="remove all keywords from the matching photos")
    selector.add_arguments(p)
    write_flags(p)
    p.set_defaults(func=cmd_clear)

    p = inner.add_parser("list", help="the keyword library")
    p.add_argument("--used", action="store_true")
    p.add_argument("--unused", action="store_true")
    p.add_argument("--sort", default="name", choices=["name", "count"])
    p.set_defaults(func=cmd_list)

    p = inner.add_parser("tree", help="the keyword hierarchy")
    p.set_defaults(func=cmd_tree)

    p = inner.add_parser("create", help="add a keyword to the library")
    p.add_argument("path", help="e.g. 'best|best2027'")
    write_flags(p)
    p.set_defaults(func=cmd_create)

    p = inner.add_parser("rename", help="rename a keyword and rewrite its usages")
    p.add_argument("--path", required=True)
    p.add_argument("--to", required=True, metavar="NAME")
    write_flags(p)
    p.set_defaults(func=cmd_rename)

    p = inner.add_parser("move", help="re-parent a keyword and rewrite its usages")
    p.add_argument("--path", required=True)
    p.add_argument("--to-parent")
    p.add_argument("--to-root", action="store_true")
    write_flags(p)
    p.set_defaults(func=cmd_move)

    p = inner.add_parser("merge", help="fold keywords into another")
    p.add_argument("--from", dest="from_paths", action="append", required=True, metavar="PATH")
    p.add_argument("--into", required=True, metavar="PATH")
    write_flags(p)
    p.set_defaults(func=cmd_merge)

    p = inner.add_parser("delete", help="remove a keyword from the library")
    p.add_argument("path")
    p.add_argument("--strip-usages", action="store_true", help="also remove it from photos that use it")
    write_flags(p)
    p.set_defaults(func=cmd_delete)
