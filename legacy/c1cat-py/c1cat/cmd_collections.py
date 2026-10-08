"""album / group / smart / stage - the collection tree."""

from __future__ import annotations

import sys

from . import collections as coll
from . import config, render, safety, selector
from .render import say, summary, table


def _resolve(ctx, name: str) -> int:
    return selector.resolve_collection(ctx.mirror, name)


def _parent_pk(ctx, args) -> int:
    if getattr(args, "parent", None):
        return _resolve(ctx, args.parent)
    return config.ROOT_COLLECTION_PK


def cmd_list(args, ctx) -> int:
    con = ctx.mirror
    kinds = ("album", "smart", "group", "project") if not args.all_kinds else None
    where = "" if kinds is None else f"WHERE kind IN ({','.join('?' * len(kinds))})"
    rows = con.execute(
        f"SELECT pk, kind, name, path, parent, n_variants FROM coll {where} ORDER BY path",
        list(kinds) if kinds else []).fetchall()

    if render.use_json():
        render.emit({"command": "album list", "count": len(rows),
                     "collections": [{"pk": r["pk"], "kind": r["kind"], "name": r["name"],
                                      "path": r["path"], "photos": r["n_variants"]} for r in rows]})
        return 0
    table(["PK", "KIND", "PHOTOS", "PATH"],
          [[r["pk"], r["kind"], r["n_variants"] or "", r["path"]] for r in rows], "rlrl")
    summary([f"{len(rows)} collection(s)"])
    return 0


def cmd_show(args, ctx) -> int:
    pk = _resolve(ctx, args.album)
    row = ctx.mirror.execute("SELECT * FROM coll WHERE pk = ?", (pk,)).fetchone()
    conditions = coll.parse_criteria(row["smart_xml"])

    if render.use_json():
        render.emit({"command": "album show", "pk": pk, "kind": row["kind"], "name": row["name"],
                     "path": row["path"], "photos": row["n_variants"],
                     "criteria": [c for c in conditions if c["enabled"]],
                     "smart_xml": row["smart_xml"] if args.xml else None})
        return 0

    say(f"{row['path']}   (#{pk}, {row['kind']})")
    say(f"  photos      {row['n_variants']}")
    say(f"  sort order  {row['sort_order']}")
    active = [c for c in conditions if c["enabled"]]
    if active:
        say("  criteria:")
        for c in active:
            say(f"    {c['key']} {c['operator_name']} {c['criterion']!r}")
    if args.xml and row["smart_xml"]:
        say("\n" + row["smart_xml"])
    return 0


def _create(args, ctx, ent: int, label: str, smart_xml: str | None = None) -> int:
    parent = _parent_pk(ctx, args)
    parent_row = ctx.mirror.execute("SELECT path, kind FROM coll WHERE pk = ?", (parent,)).fetchone()
    parent_path = parent_row["path"] if parent_row else "(root)"

    existing = ctx.mirror.execute(
        "SELECT pk FROM coll WHERE parent = ? AND name = ? COLLATE NOCASE", (parent, args.name)).fetchone()
    if existing and not getattr(args, "allow_duplicate", False):
        say(f"a collection named {args.name!r} already exists under {parent_path} (#{existing['pk']})")
        return safety.EXIT_USAGE

    if not args.apply:
        say(f"DRY RUN - would create {label} {args.name!r} under {parent_path or '(root)'}")
        if smart_xml:
            say("\ncriteria:")
            for c in coll.parse_criteria(smart_xml):
                if c["enabled"]:
                    say(f"  {c['key']} {c['operator_name']} {c['criterion']!r}")
            if getattr(args, "xml", False):
                say("\n" + smart_xml)
        say("\nRe-run with --apply to create it.")
        if render.use_json():
            render.emit({"command": f"{label} create", "dry_run": True, "name": args.name,
                         "parent": parent_path, "smart_xml": smart_xml})
        return 0

    with safety.writer(ctx.catalog, apply=True, force=args.force, backup=not args.no_backup,
                       argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        pk = coll.create(con, args.name, ent, parent, smart_xml=smart_xml)
    if render.use_json():
        render.emit({"command": f"{label} create", "dry_run": False, "pk": pk, "name": args.name})
    else:
        say(f"created {label} {args.name!r} (#{pk}) under {parent_path or '(root)'}")
        summary(["Committed. Reopen Capture One to see it."])
    return 0


def cmd_album_create(args, ctx) -> int:
    return _create(args, ctx, config.ENT_ALBUM, "album")


def cmd_group_create(args, ctx) -> int:
    return _create(args, ctx, config.ENT_GROUP, "group")


def cmd_smart_create(args, ctx) -> int:
    conditions = []
    if args.rating:
        conditions.append(coll.rating_condition(args.rating))
    for keyword in args.keyword or []:
        conditions.append((coll.KEY_KEYWORDS, coll.OP_CONTAINS, keyword))
    if args.year:
        conditions.append((coll.KEY_DATE, coll.OP_EQUALS, str(args.year)))
    for raw in args.raw_condition or []:
        try:
            key, operator, criterion = raw.split(":", 2)
        except ValueError:
            raise selector.SelectorError("--raw-condition expects KEY:OPERATOR:VALUE") from None
        conditions.append((key, int(operator), criterion))
    if not conditions:
        raise selector.SelectorError(
            "a smart album needs at least one condition (--rating, --keyword, --year or --raw-condition)")
    xml = coll.smart_criteria(conditions, match_all=(args.match == "all"))
    return _create(args, ctx, config.ENT_SMART, "smart album", smart_xml=xml)


def cmd_rename(args, ctx) -> int:
    pk = _resolve(ctx, args.album)
    row = ctx.mirror.execute("SELECT name, path FROM coll WHERE pk = ?", (pk,)).fetchone()
    say(f"{'renaming' if args.apply else 'would rename'} {row['path']} -> {args.to!r}")
    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        con.execute("UPDATE ZCOLLECTION SET ZNAME = ? WHERE Z_PK = ?", (args.to, pk))
        coll.touch(con, pk)
    summary(["Committed." if wctx["applied"] else "Nothing written. Re-run with --apply."])
    return 0


def cmd_delete(args, ctx) -> int:
    pk = _resolve(ctx, args.album)
    row = ctx.mirror.execute("SELECT name, path, kind, n_variants FROM coll WHERE pk = ?", (pk,)).fetchone()
    children = ctx.mirror.execute("SELECT COUNT(*) FROM coll WHERE parent = ?", (pk,)).fetchone()[0]
    if children and not args.force_children:
        say(f"{row['path']} has {children} child collection(s); pass --force-children to delete anyway")
        return safety.EXIT_USAGE

    say(f"{'deleting' if args.apply else 'would delete'} {row['kind']} {row['path']} "
        f"({row['n_variants']} photo membership(s){', ' + str(children) + ' children' if children else ''})")
    say("The photos themselves are not touched.")
    if not args.apply:
        say("Re-run with --apply to delete it.")

    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        targets = [pk]
        if children:
            frontier = [pk]
            while frontier:
                current = frontier.pop()
                for (child,) in con.execute("SELECT Z_PK FROM ZCOLLECTION WHERE ZPARENT = ?", (current,)):
                    targets.append(child)
                    frontier.append(child)
        for target in reversed(targets):
            coll.delete(con, target)
    summary([f"{len(targets) if args.apply else children + 1} collection(s)",
             "Committed." if wctx["applied"] else "Nothing written. Re-run with --apply."])
    return 0


def _membership(args, ctx, adding: bool) -> int:
    if selector.is_empty(args):
        raise selector.SelectorError("refusing to run without a selection - add at least one filter")
    rows, _ = selector.run(args, ctx.mirror)
    if not rows:
        say("(nothing matched - no changes)")
        return 0
    variant_pks = [r["variant_pk"] for r in rows]
    album = args.target_album

    created = False
    if adding and getattr(args, "create", False):
        existing = ctx.mirror.execute(
            "SELECT pk FROM coll WHERE name = ? COLLATE NOCASE", (album,)).fetchone()
        created = existing is None
    pk = None if created else _resolve(ctx, album)

    verb = "add" if adding else "remove"
    if not args.apply:
        say(f"DRY RUN - would {verb} {len(variant_pks)} photo(s) "
            f"{'to' if adding else 'from'} {album!r}"
            + (" (creating it)" if created else ""))
        table(["VAR#", "FILENAME", "DATE"],
              [[r["variant_pk"], r["filename"], render.when(r["capture_ts"], "%Y-%m-%d")]
               for r in rows[:20]], "rll")
        if len(rows) > 20:
            say(f"... and {len(rows) - 20} more")
        say("\nRe-run with --apply to commit.")

    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        target = pk
        if created:
            target = coll.create(con, album, config.ENT_ALBUM, _parent_pk(ctx, args))
        before = coll.dangling_links(con, target)
        if adding:
            images, variants = coll.add_members(con, target, variant_pks)
        else:
            images, variants = coll.remove_members(con, target, variant_pks)
        problems = coll.check_no_worse(before, coll.dangling_links(con, target))
        if problems:
            raise safety.Refused("this would leave the album's links inconsistent: "
                                 + "; ".join(problems), safety.EXIT_VERIFY)

    lines = [f"{len(variant_pks)} selected, {variants} variant link(s) and {images} image link(s) "
             f"{'added' if adding else 'removed'}"]
    if adding and variants < len(variant_pks):
        lines.append(f"{len(variant_pks) - variants} were already in the album")
    lines.append("Committed. Reopen Capture One to see the changes." if wctx["applied"]
                 else "Nothing written. Re-run with --apply to commit.")

    if render.use_json():
        render.emit({"command": f"album {verb}", "dry_run": not args.apply,
                     "applied": wctx["applied"], "album": album,
                     "selected": len(variant_pks), "variant_links": variants, "image_links": images})
    else:
        summary(lines)
    return 0


def cmd_add(args, ctx) -> int:
    return _membership(args, ctx, adding=True)


def cmd_remove(args, ctx) -> int:
    return _membership(args, ctx, adding=False)


def cmd_stage(args, ctx) -> int:
    """Create the album if needed, then add - the end of a develop-triage workflow."""
    args.create = True
    return _membership(args, ctx, adding=True)


def register(sub, write_flags) -> None:
    parser = sub.add_parser("album", help="albums, groups and smart albums")
    inner = parser.add_subparsers(dest="album_command", required=True)

    p = inner.add_parser("list", help="all collections")
    p.add_argument("--all-kinds", action="store_true", help="include folders and system collections")
    p.set_defaults(func=cmd_list)

    p = inner.add_parser("show", help="one collection and its criteria")
    p.add_argument("album", metavar="NAME|#PK")
    p.add_argument("--xml", action="store_true", help="print the raw criteria blob")
    p.set_defaults(func=cmd_show)

    p = inner.add_parser("create", help="create an album")
    p.add_argument("name")
    p.add_argument("--parent", metavar="NAME|#PK")
    p.add_argument("--allow-duplicate", action="store_true")
    write_flags(p)
    p.set_defaults(func=cmd_album_create)

    p = inner.add_parser("rename", help="rename a collection")
    p.add_argument("album", metavar="NAME|#PK")
    p.add_argument("--to", required=True)
    write_flags(p)
    p.set_defaults(func=cmd_rename)

    p = inner.add_parser("delete", help="delete a collection (photos are untouched)")
    p.add_argument("album", metavar="NAME|#PK")
    p.add_argument("--force-children", action="store_true")
    write_flags(p)
    p.set_defaults(func=cmd_delete)

    # The target album is positional: --album is already a selector *filter* here,
    # so "album add portfolio --album street" reads as "copy street into portfolio".
    p = inner.add_parser("add", help="add the matching photos to an album")
    p.add_argument("target_album", metavar="ALBUM")
    selector.add_arguments(p)
    p.add_argument("--create", action="store_true", help="create the album if it does not exist")
    p.add_argument("--parent", metavar="NAME|#PK", help="where to create it")
    write_flags(p)
    p.set_defaults(func=cmd_add)

    p = inner.add_parser("remove", help="remove the matching photos from an album")
    p.add_argument("target_album", metavar="ALBUM")
    selector.add_arguments(p)
    write_flags(p)
    p.set_defaults(func=cmd_remove)

    p = sub.add_parser("group", help="collection groups")
    ginner = p.add_subparsers(dest="group_command", required=True)
    g = ginner.add_parser("create")
    g.add_argument("name")
    g.add_argument("--parent", metavar="NAME|#PK")
    g.add_argument("--allow-duplicate", action="store_true")
    write_flags(g)
    g.set_defaults(func=cmd_group_create)

    p = sub.add_parser("smart", help="smart albums")
    sinner = p.add_subparsers(dest="smart_command", required=True)
    s = sinner.add_parser("create")
    s.add_argument("name")
    s.add_argument("--parent", metavar="NAME|#PK")
    s.add_argument("--match", default="all", choices=["all", "any"])
    s.add_argument("--rating", help="'>=4', '>3' or '5'")
    s.add_argument("--keyword", action="append")
    s.add_argument("--year", type=int)
    s.add_argument("--raw-condition", action="append", metavar="KEY:OP:VALUE",
                   help="an untranslated Capture One condition")
    s.add_argument("--xml", action="store_true", help="show the generated criteria blob")
    s.add_argument("--allow-duplicate", action="store_true")
    write_flags(s)
    s.set_defaults(func=cmd_smart_create)

    s = sinner.add_parser("show")
    s.add_argument("album", metavar="NAME|#PK")
    s.add_argument("--xml", action="store_true")
    s.set_defaults(func=cmd_show)

    p = sub.add_parser("stage", help="put the matching photos into an album, creating it if needed")
    p.add_argument("target_album", metavar="ALBUM")
    selector.add_arguments(p)
    p.add_argument("--parent", metavar="NAME|#PK")
    write_flags(p)
    p.set_defaults(func=cmd_stage)
