"""find / count / show / stats / paths - the read-only side."""

from __future__ import annotations

from . import keywords, render, selector
from .render import say, summary, table, when

DEFAULT_FIELDS = ["variant_pk", "filename", "date", "rating", "color", "keywords", "camera"]

FIELD_LABELS = {
    "variant_pk": ("VAR#", "r"), "image_pk": ("IMG#", "r"), "filename": ("FILENAME", "l"),
    "date": ("DATE", "l"), "rating": ("R", "r"), "color": ("C", "l"),
    "keywords": ("KEYWORDS", "l"), "camera": ("CAMERA", "l"), "lens": ("LENS", "l"),
    "iso": ("ISO", "r"), "aperture": ("F", "r"), "focal": ("MM", "r"),
    "description": ("DESCRIPTION", "l"), "city": ("CITY", "l"), "country": ("COUNTRY", "l"),
    "path": ("PATH", "l"), "styles": ("STYLES", "l"), "edited": ("ED", "l"),
    "album": ("ALBUMS", "l"), "size": ("SIZE", "l"),
}


def _cell(row, field, con=None):
    if field == "date":
        return when(row["capture_ts"], "%Y-%m-%d")
    if field == "rating":
        return row["rating"] if row["rating"] else "-"
    if field == "color":
        return row["color_name"] or "-"
    if field == "keywords":
        return ", ".join(p for p, _ in keywords.parse(row["keywords_raw"]))
    if field == "camera":
        return row["camera_model"] or ""
    if field == "path":
        return row["path_wsl"] or row["path_win"] or ""
    if field == "styles":
        return (row["style_names"] or "").replace(";", ", ")
    if field == "edited":
        return "yes" if row["is_modified"] else "no"
    if field == "size":
        return f"{row['width']}x{row['height']}" if row["width"] else ""
    if field == "album" and con is not None:
        names = con.execute(
            "SELECT c.name FROM member m JOIN coll c ON c.pk = m.coll_pk "
            "WHERE m.variant_pk = ? AND c.kind IN ('album','project') ORDER BY c.name",
            (row["variant_pk"],)).fetchall()
        return ", ".join(n[0] for n in names)
    if field in ("aperture", "focal"):
        value = row[field]
        return f"{value:g}" if value else ""
    return row[field] if field in row.keys() else ""


def find(args, ctx) -> int:
    con = ctx.mirror

    if getattr(args, "count", False):
        # Counting ignores --limit/--offset: "how many are there" is not "how many did you show me".
        total = selector.count(args, con)
        query = selector.build(args, con)
        if args.dump_sql:
            say(query.sql, "\n-- params:", str(query.params))
        if render.use_json():
            render.emit({"command": "count", "query": query.described, "count": total})
        else:
            print(total)
        return 0

    rows, query = selector.run(args, con)
    if args.dump_sql:
        say(query.sql, "\n-- params:", str(query.params))

    if render.use_json():
        render.emit({
            "command": "find",
            "catalog": str(ctx.catalog),
            "query": query.described,
            "count": len(rows),
            "variants": [render.variant_dict(r) for r in rows],
        })
        return 0

    fields = [f.strip() for f in args.fields.split(",")] if args.fields else DEFAULT_FIELDS
    unknown = [f for f in fields if f not in FIELD_LABELS]
    if unknown:
        say(f"unknown field(s): {', '.join(unknown)}")
        say(f"available: {', '.join(sorted(FIELD_LABELS))}")
        return 1
    headers = [FIELD_LABELS[f][0] for f in fields]
    aligns = "".join(FIELD_LABELS[f][1] for f in fields)
    table(headers, [[_cell(r, f, con) for f in fields] for r in rows], aligns)

    lines = [f"{len(rows)} variant(s)"]
    rated = {}
    for r in rows:
        key = r["rating"] or 0
        rated[key] = rated.get(key, 0) + 1
    if rated:
        lines.append("ratings: " + "  ".join(
            f"{'unrated' if k == 0 else str(k) + '*'}x{v}" for k, v in sorted(rated.items(), reverse=True)))
    if args.save_selection:
        lines.append(f"selection saved to {args.save_selection}")
    summary(lines)
    return 0


def count(args, ctx) -> int:
    total = selector.count(args, ctx.mirror)
    if render.use_json():
        render.emit({"command": "count", "count": total})
    else:
        print(total)
    return 0


def show(args, ctx) -> int:
    con = ctx.mirror
    rows, _ = selector.run(args, con)
    if not rows:
        say("(nothing matched)")
        return 0
    if render.use_json():
        payload = []
        for row in rows:
            item = render.variant_dict(row)
            item["albums"] = [
                {"pk": a["pk"], "name": a["name"], "path": a["path"]}
                for a in con.execute(
                    "SELECT c.pk, c.name, c.path FROM member m JOIN coll c ON c.pk = m.coll_pk "
                    "WHERE m.variant_pk = ? ORDER BY c.path", (row["variant_pk"],))]
            payload.append(item)
        render.emit({"command": "show", "count": len(payload), "variants": payload})
        return 0

    for row in rows:
        say(f"\n{row['filename']}   (variant {row['variant_pk']}, image {row['image_pk']})")
        say(f"  file       {row['path_wsl'] or row['path_win']}")
        say(f"  captured   {when(row['capture_ts'])}    imported {when(row['import_ts'])}")
        say(f"  camera     {row['camera_model'] or '?'} / {row['lens'] or '?'}"
            f"   ISO {row['iso'] or '?'}  f/{row['aperture'] or '?'}  {row['focal'] or '?'}mm")
        say(f"  size       {row['width']}x{row['height']}")
        say(f"  rating     {row['rating'] or '-'}    colour {row['color_name'] or '-'}"
            f"    edited {'yes' if row['is_modified'] else 'no'}")
        kws = ", ".join(p for p, _ in keywords.parse(row["keywords_raw"]))
        say(f"  keywords   {kws or '-'}")
        if row["description"]:
            say(f"  caption    {row['description']}")
        place = " / ".join(x for x in (row["city"], row["state"], row["country"]) if x)
        if place:
            say(f"  place      {place}")
        if row["lat"] is not None:
            say(f"  gps        {row['lat']:.6f}, {row['lon']:.6f}")
        if row["style_names"]:
            say(f"  styles     {row['style_names'].replace(';', ', ')}")
        albums = con.execute(
            "SELECT c.path FROM member m JOIN coll c ON c.pk = m.coll_pk "
            "WHERE m.variant_pk = ? ORDER BY c.path", (row["variant_pk"],)).fetchall()
        if albums:
            say(f"  albums     {', '.join(a[0] for a in albums)}")
        say(f"  thumb      {row['thumb_path']}")
        if row["preview_path"]:
            say(f"  preview    {row['preview_path']}")
    return 0


GROUPINGS = {
    "rating": ("CASE WHEN rating IS NULL THEN 'unrated' ELSE rating || '*' END", "rating DESC"),
    "color": ("COALESCE(color_name, 'none')", "COUNT(*) DESC"),
    "camera": ("COALESCE(camera_model, '?')", "COUNT(*) DESC"),
    "lens": ("COALESCE(lens, '?')", "COUNT(*) DESC"),
    "year": ("COALESCE(capture_year, 0)", "1 ASC"),
    "month": ("COALESCE(capture_month, '?')", "1 ASC"),
    "folder": ("COALESCE(folder_wsl, '?')", "COUNT(*) DESC"),
    "country": ("COALESCE(country, '?')", "COUNT(*) DESC"),
    "city": ("COALESCE(city, '?')", "COUNT(*) DESC"),
    "ext": ("ext", "COUNT(*) DESC"),
    "style": ("COALESCE(style_names, '(none)')", "COUNT(*) DESC"),
    "edited": ("CASE WHEN is_modified THEN 'edited' ELSE 'unedited' END", "COUNT(*) DESC"),
}


def stats(args, ctx) -> int:
    con = ctx.mirror
    query = selector.build(args, con)
    inner = query.sql.split("\n ORDER BY")[0]

    if args.by == "keyword":
        rows = con.execute(f"SELECT keywords_raw FROM ({inner})", query.params).fetchall()
        tally: dict[str, int] = {}
        for (serialised,) in rows:
            for path, _ in keywords.parse(serialised):
                tally[path] = tally.get(path, 0) + 1
        items = sorted(tally.items(), key=lambda kv: (-kv[1], kv[0]))
    elif args.by == "album":
        items = [(r[0], r[1]) for r in con.execute(
            f"SELECT c.path, COUNT(*) FROM ({inner}) x JOIN member m ON m.variant_pk = x.variant_pk "
            f"JOIN coll c ON c.pk = m.coll_pk GROUP BY c.path ORDER BY COUNT(*) DESC", query.params)]
    else:
        expr, order = GROUPINGS[args.by]
        items = [(str(r[0]), r[1]) for r in con.execute(
            f"SELECT {expr} AS g, COUNT(*) AS n FROM ({inner}) GROUP BY g ORDER BY {order}",
            query.params)]

    if args.limit_groups:
        items = items[: args.limit_groups]

    if render.use_json():
        render.emit({"command": "stats", "by": args.by, "query": query.described,
                     "groups": [{"key": k, "count": n} for k, n in items]})
        return 0

    total = sum(n for _, n in items) or 1
    table([args.by.upper(), "COUNT", "SHARE"],
          [[k, n, f"{100 * n / total:5.1f}%"] for k, n in items], "lrr")
    summary([f"{len(items)} group(s), {total} variant(s)"])
    return 0


def paths_cmd(args, ctx) -> int:
    rows, _ = selector.run(args, ctx.mirror)
    tier = args.tier
    out = []
    for row in rows:
        if tier == "thumb":
            value = row["thumb_path"]
        elif tier == "preview":
            value = row["preview_path"]
        elif tier == "windows":
            value = row["path_win"]
        else:
            value = row["path_wsl"]
        out.append({"variant_pk": row["variant_pk"], "path": value})
    if render.use_json():
        render.emit({"command": "paths", "tier": tier, "count": len(out), "paths": out})
    else:
        for item in out:
            if item["path"]:
                print(item["path"])
    return 0
