"""rate / color / describe / iptc / geo - writes to ZVARIANTMETADATA and ZIMAGE."""

from __future__ import annotations

import json
import sys
from pathlib import Path

from . import config, paths, render, safety, selector, source
from .render import say, summary, table

# Every field here lives on ZVARIANTMETADATA and must be written to BOTH the
# adjustment-layer row and the combined-layer row.
FIELDS = {
    "rating": "ZBASIC_RATING",
    "color": "ZCOLOR_TAG_INDEX",
    "description": "ZCONTENT_DESCRIPTION",
    "headline": "ZCONTENT_HEADLINE",
    "title": "ZSTATUS_TITLE",
    "creator": "ZCONTACT_CREATOR",
    "copyright": "ZSTATUS_COPYRIGHTNOTICE",
    "city": "ZIMAGE_CITY",
    "state": "ZIMAGE_STATE",
    "country": "ZIMAGE_COUNTRY",
    "country_code": "ZIMAGE_ISOCOUNTRYCODE",
    "location": "ZIMAGE_LOCATION",
    "instructions": "ZSTATUS_INSTRUCTIONS",
    "job_id": "ZSTATUS_JOBIDENTIFIER",
    "source": "ZSTATUS_SOURCE",
}

MIRROR_COLUMN = {
    "rating": "rating", "color": "color_name", "description": "description",
    "headline": "headline", "title": "title", "creator": "creator", "copyright": "copyright",
    "city": "city", "state": "state", "country": "country", "country_code": "iso_country",
    "location": "location",
}


def write_fields(con, variant_pks: list[int], values: dict[str, object]) -> int:
    """Apply field updates to both metadata rows of each variant. Returns rows touched."""
    assignments = ", ".join(f"{FIELDS[name]} = ?" for name in values)
    params_head = [values[name] for name in values]
    touched = 0
    for pk in variant_pks:
        rows = source.metadata_rows(con, pk)
        for metadata_pk in rows:
            con.execute(
                f"UPDATE ZVARIANTMETADATA SET {assignments} WHERE Z_PK = ?",
                params_head + [metadata_pk])
            touched += 1
    return touched


def _preview(rows, values: dict[str, object], display: dict[str, str]) -> None:
    headers = ["VAR#", "FILENAME"] + [name.upper() for name in values]
    body = []
    for row in rows[:40]:
        cells = [row["variant_pk"], row["filename"]]
        for name in values:
            column = MIRROR_COLUMN.get(name)
            before = row[column] if column and column in row.keys() else None
            cells.append(f"{before if before not in (None, '') else '-'} -> {display[name]}")
        body.append(cells)
    table(headers, body, "rl" + "l" * len(values))
    if len(rows) > 40:
        say(f"... and {len(rows) - 40} more")


def _finish(args, ctx, rows, values, display, extra_lines=None) -> int:
    if not rows:
        say("(nothing matched - no changes)")
        return 0
    variant_pks = [r["variant_pk"] for r in rows]

    if not render.use_json():
        if not args.apply:
            say("DRY RUN - nothing will be written. Re-run with --apply to commit.\n")
        _preview(rows, values, display)

    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        touched = write_fields(con, variant_pks, values)

    lines = [f"{len(variant_pks)} variant(s), {touched} metadata rows "
             f"({len(variant_pks)} adjustment + {len(variant_pks)} combined)"]
    lines += extra_lines or []
    if wctx["applied"]:
        lines.append(f"backup: {Path(wctx['backup']['backup']).name}" if wctx["backup"] else "backup: skipped")
        lines.append("Committed. Reopen Capture One to see the changes.")
    else:
        lines.append("Nothing written. Re-run with --apply to commit.")

    if render.use_json():
        render.emit({
            "command": "write", "dry_run": not args.apply, "applied": wctx["applied"],
            "count": len(variant_pks), "rows_touched": touched,
            "values": {k: display[k] for k in values},
            "variant_pks": variant_pks,
            "backup": wctx["backup"]["backup"] if wctx["backup"] else None,
        })
    else:
        summary(lines)
    return 0


def _select(args, ctx):
    if selector.is_empty(args):
        raise selector.SelectorError(
            "refusing to run without a selection - add at least one filter (e.g. --id, --album, --rating)")
    rows, _ = selector.run(args, ctx.mirror)
    return rows


def cmd_rate(args, ctx) -> int:
    if args.clear:
        value, shown = None, "unrated"
    elif args.set is None:
        raise selector.SelectorError("pass --set 0..5 or --clear")
    else:
        if not 0 <= args.set <= 5:
            raise selector.SelectorError("rating must be between 0 and 5")
        value = args.set or None      # Capture One stores "unrated" as NULL, not 0
        shown = str(args.set) if args.set else "unrated"
    return _finish(args, ctx, _select(args, ctx), {"rating": value}, {"rating": shown})


def cmd_color(args, ctx) -> int:
    name = args.set.strip().lower()
    if name not in config.COLOR_INDEX:
        raise selector.SelectorError(
            f"unknown colour {args.set!r}; choose from none, {', '.join(config.COLOR_NAMES[1:])}")
    index = config.COLOR_INDEX[name] or None
    return _finish(args, ctx, _select(args, ctx), {"color": index}, {"color": name})


def cmd_describe(args, ctx) -> int:
    if args.from_json:
        return _from_json(args, ctx)
    values, display = {}, {}
    for name in ("description", "headline", "title"):
        given = getattr(args, name)
        if given is not None:
            values[name] = given or None
            display[name] = given or "(cleared)"
    if not values:
        raise selector.SelectorError("pass --description, --headline, --title or --from-json")
    return _finish(args, ctx, _select(args, ctx), values, display)


def cmd_iptc(args, ctx) -> int:
    values, display = {}, {}
    for name in ("creator", "copyright", "city", "state", "country", "country_code",
                 "location", "instructions", "job_id", "source"):
        given = getattr(args, f"set_{name}", None)
        if given is not None:
            values[name] = given or None
            display[name] = given or "(cleared)"
    if not values:
        raise selector.SelectorError(
            "pass at least one field to write, e.g. --set-city 'Kraków' --set-country Poland")
    return _finish(args, ctx, _select(args, ctx), values, display)


def cmd_geo(args, ctx) -> int:
    """GPS lives on ZIMAGE, so this affects every variant of the matched images."""
    rows = _select(args, ctx)
    if not rows:
        say("(nothing matched - no changes)")
        return 0

    if args.clear:
        lat_text = lon_text = None
        shown = "(cleared)"
    else:
        if args.lat is None or args.lon is None:
            raise selector.SelectorError("pass --lat and --lon, or --clear")
        lat_text = paths.format_gps(args.lat, is_latitude=True)
        lon_text = paths.format_gps(args.lon, is_latitude=False)
        shown = f"{lat_text} {lon_text}"

    image_pks = sorted({r["image_pk"] for r in rows})
    siblings = ctx.mirror.execute(
        f"SELECT COUNT(*) FROM v WHERE image_pk IN ({','.join('?' * len(image_pks))})",
        image_pks).fetchone()[0]

    if not render.use_json():
        if not args.apply:
            say("DRY RUN - nothing will be written. Re-run with --apply to commit.\n")
        table(["IMG#", "FILENAME", "GPS"],
              [[r["image_pk"], r["filename"],
                f"{r['lat'] if r['lat'] is not None else '-'} -> {shown}"] for r in rows[:40]], "rll")

    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        con.executemany(
            "UPDATE ZIMAGE SET ZGPSLATITUDE = ?, ZGPSLONGITUDE = ? WHERE Z_PK = ?",
            [(lat_text, lon_text, pk) for pk in image_pks])

    lines = [f"{len(image_pks)} image(s) -> {shown}"]
    if siblings > len(rows):
        lines.append(f"note: GPS is stored per image, so this also affects "
                     f"{siblings - len(rows)} sibling variant(s) not in the selection")
    lines.append("Committed. Reopen Capture One to see the changes." if wctx["applied"]
                 else "Nothing written. Re-run with --apply to commit.")

    if render.use_json():
        render.emit({"command": "geo", "dry_run": not args.apply, "applied": wctx["applied"],
                     "image_pks": image_pks, "lat": args.lat, "lon": args.lon,
                     "affects_sibling_variants": siblings - len(rows)})
    else:
        summary(lines)
    return 0


def _from_json(args, ctx) -> int:
    """Apply per-variant values from a JSON file (the shape vision apply produces)."""
    payload = json.loads(Path(args.from_json).read_text())
    items = payload["items"] if isinstance(payload, dict) else payload
    by_pk = {int(item["variant_pk"]): item for item in items}
    if not by_pk:
        say("(nothing in the file)")
        return 0

    rows = ctx.mirror.execute(
        f"SELECT * FROM v WHERE variant_pk IN ({','.join('?' * len(by_pk))})",
        sorted(by_pk)).fetchall()
    known = {r["variant_pk"] for r in rows}
    missing = sorted(set(by_pk) - known)
    if missing:
        render.warn(f"{len(missing)} variant(s) in the file are not in the catalog: {missing[:5]}")

    if not args.apply:
        say("DRY RUN - nothing will be written. Re-run with --apply to commit.\n")
        table(["VAR#", "FILENAME", "FIELDS"],
              [[r["variant_pk"], r["filename"],
                ", ".join(k for k in by_pk[r["variant_pk"]] if k != "variant_pk")]
               for r in rows[:40]], "rll")

    touched = 0
    with safety.writer(ctx.catalog, apply=args.apply, force=args.force,
                       backup=not args.no_backup, argv=sys.argv[1:], keep=args.keep) as (con, wctx):
        for row in rows:
            item = by_pk[row["variant_pk"]]
            values = {name: (item[name] or None) for name in FIELDS if name in item}
            if "rating" in values and values["rating"] is not None:
                values["rating"] = int(values["rating"]) or None
            if values:
                touched += write_fields(con, [row["variant_pk"]], values)

    summary([f"{len(rows)} variant(s), {touched} metadata rows",
             "Committed. Reopen Capture One to see the changes." if wctx["applied"]
             else "Nothing written. Re-run with --apply to commit."])
    return 0


def register(sub, write_flags) -> None:
    p = sub.add_parser("rate", help="set or clear the star rating")
    selector.add_arguments(p)
    p.add_argument("--set", type=int, metavar="0-5")
    p.add_argument("--clear", action="store_true")
    write_flags(p)
    p.set_defaults(func=cmd_rate)

    p = sub.add_parser("color", aliases=["colour"], help="set the colour tag")
    selector.add_arguments(p)
    p.add_argument("--set", required=True, metavar="NAME",
                   help="none, " + ", ".join(config.COLOR_NAMES[1:]))
    write_flags(p)
    p.set_defaults(func=cmd_color)

    p = sub.add_parser("describe", help="set caption, headline or title")
    selector.add_arguments(p)
    p.add_argument("--description")
    p.add_argument("--headline")
    p.add_argument("--title")
    p.add_argument("--from-json", metavar="FILE", help="per-variant values from a JSON file")
    write_flags(p)
    p.set_defaults(func=cmd_describe)

    p = sub.add_parser("iptc", help="set IPTC contact and location fields")
    selector.add_arguments(p)
    # Prefixed with --set- because the selector already owns --city/--country/--state
    # as *filters*; keeping them distinct avoids "did I mean match or write?".
    for name in ("creator", "copyright", "city", "state", "country", "country-code",
                 "location", "instructions", "job-id", "source"):
        p.add_argument(f"--set-{name}", dest=f"set_{name.replace('-', '_')}", metavar="TEXT")
    write_flags(p)
    p.set_defaults(func=cmd_iptc)

    p = sub.add_parser("geo", help="set or clear GPS coordinates (stored per image)")
    selector.add_arguments(p)
    p.add_argument("--lat", type=float)
    p.add_argument("--lon", type=float)
    p.add_argument("--clear", action="store_true")
    write_flags(p)
    p.set_defaults(func=cmd_geo)
