"""develop report / styles / diff - read-only.

What the user changed is a straight row diff between the variant's adjustment
layer and its own default layer, so there is no table of per-camera defaults to
maintain and no drift when Capture One adds a slider.

The combined layer is deliberately not used as a baseline: it is Capture One's
own computed cache and holds style-derived values, not defaults. One sampled
variant has adjustment exposure 0.3525 against combined 0.0, and combined
contrast 12.31 which came from its style.
"""

from __future__ import annotations

from . import develop_names, render, selector
from .render import say, summary, table


def _layer(con, layer_pk: int) -> dict:
    row = con.execute("SELECT * FROM ZVARIANTLAYER WHERE Z_PK = ?", (layer_pk,)).fetchone()
    return dict(row) if row else {}


def _diff(adjustment: dict, default: dict) -> list[tuple[str, object, object]]:
    """Columns the user actually changed.

    The adjustment layer only carries what was overridden: a NULL there means "inherit
    this file's default", not "cleared". So a column counts as changed only when the
    adjustment layer holds a value of its own that differs from the default layer.
    """
    changes = []
    for column, value in adjustment.items():
        if column in develop_names.IGNORED:
            continue
        if value in (None, ""):
            continue
        baseline = default.get(column)
        if value == baseline:
            continue
        if baseline in (None, "") and value in (0, 0.0):
            continue
        changes.append((column, baseline, value))
    changes.sort(key=lambda c: (develop_names.group(c[0]), develop_names.label(c[0])))
    return changes


def _styles(con, variant_pk: int) -> list[dict]:
    rows = con.execute(
        "SELECT ZNAME, ZSTYLEUUID, ZSTYLESOURCE FROM ZVARIANTLAYER "
        "WHERE ZVARIANT = ? AND Z_ENT = 19 ORDER BY ZINDEX", (variant_pk,)).fetchall()
    return [{"name": r[0], "uuid": r[1], "source": r[2]} for r in rows]


def report(args, ctx) -> int:
    con = ctx.mirror       # the mirror is a copy of the catalog, so ZVARIANTLAYER is here too
    rows, _ = selector.run(args, con)
    if not rows:
        say("(nothing matched)")
        return 0

    edited = sum(1 for r in rows if r["is_modified"])
    if not args.verbose and len(rows) > 1:
        return _aggregate(args, ctx, rows, edited)

    payload = []
    for row in rows[: args.limit or len(rows)]:
        adjustment = _layer(con, row["adj_layer"])
        default = _layer(con, row["def_layer"])
        changes = _diff(adjustment, default)
        styles = _styles(con, row["variant_pk"])
        payload.append({
            "variant_pk": row["variant_pk"], "filename": row["filename"],
            "edited": bool(row["is_modified"]),
            "styles": styles,
            "local_layers": row["n_local_layers"], "retouching_layers": row["n_retouch_layers"],
            "adjustments": [
                {"column": c, "field": develop_names.label(c), "group": develop_names.group(c),
                 "from": develop_names.format_value(c, b), "to": develop_names.format_value(c, v)}
                for c, b, v in changes],
        })

    if render.use_json():
        render.emit({"command": "develop report", "count": len(payload), "variants": payload})
        return 0

    for item in payload:
        say(f"\n{item['filename']}   (variant {item['variant_pk']})   "
            f"edited: {'yes' if item['edited'] else 'no'}")
        if item["styles"]:
            say("  styles:  " + ", ".join(
                f"{s['name']} ({s['source'] or 'user'})" for s in item["styles"]))
        if item["local_layers"] or item["retouching_layers"]:
            say(f"  layers:  {item['local_layers']} local adjustment, "
                f"{item['retouching_layers']} retouching")
        if item["adjustments"]:
            say("  changed from this file's defaults:")
            current = None
            for adj in item["adjustments"]:
                if adj["group"] != current:
                    current = adj["group"]
                    say(f"    [{current}]")
                say(f"      {adj['field']:<28} {adj['from']}  ->  {adj['to']}")
        else:
            say("  no adjustments differ from the file's defaults")
    summary([f"{len(payload)} variant(s), {sum(1 for p in payload if p['edited'])} edited"])
    return 0


def _aggregate(args, ctx, rows, edited: int) -> int:
    con = ctx.mirror
    styles: dict[str, int] = {}
    with_local = 0
    for row in rows:
        for name in (row["style_names"] or "").split(";"):
            if name:
                styles[name] = styles.get(name, 0) + 1
        if row["n_local_layers"]:
            with_local += 1
    ranked = sorted(styles.items(), key=lambda kv: -kv[1])

    if render.use_json():
        render.emit({"command": "develop report", "aggregate": True, "count": len(rows),
                     "edited": edited, "unedited": len(rows) - edited,
                     "with_local_layers": with_local,
                     "styles": [{"name": n, "count": c} for n, c in ranked]})
        return 0

    say(f"{len(rows)} variant(s): {edited} edited, {len(rows) - edited} untouched, "
        f"{with_local} with local adjustment layers")
    if ranked:
        say("")
        table(["STYLE", "PHOTOS"], [[n, c] for n, c in ranked[:25]], "lr")
    summary([f"pass --verbose (and ideally --limit) to see per-photo adjustments"])
    return 0


def styles(args, ctx) -> int:
    con = ctx.mirror
    rows = con.execute(
        "SELECT ZNAME, ZSTYLESOURCE, COUNT(*) AS n FROM ZVARIANTLAYER "
        "WHERE Z_ENT = 19 AND ZNAME IS NOT NULL AND ZNAME <> '' "
        "GROUP BY ZNAME, ZSTYLESOURCE ORDER BY n DESC").fetchall()
    if render.use_json():
        render.emit({"command": "develop styles", "count": len(rows),
                     "styles": [{"name": r[0], "source": r[1], "count": r[2]} for r in rows]})
        return 0
    table(["STYLE", "SOURCE", "PHOTOS"], [[r[0], r[1] or "", r[2]] for r in rows], "llr")
    summary([f"{len(rows)} style(s) applied across the catalog"])
    return 0


def diff(args, ctx) -> int:
    con = ctx.mirror
    rows = con.execute("SELECT * FROM v WHERE variant_pk IN (?, ?)", (args.a, args.b)).fetchall()
    if len(rows) != 2:
        say("both --a and --b must be existing variant primary keys")
        return 1
    first, second = (r for r in sorted(rows, key=lambda r: 0 if r["variant_pk"] == args.a else 1))
    left, right = _layer(con, first["adj_layer"]), _layer(con, second["adj_layer"])

    changes = []
    for column in sorted(set(left) | set(right)):
        if column in develop_names.IGNORED:
            continue
        if left.get(column) != right.get(column):
            changes.append((column, left.get(column), right.get(column)))

    if render.use_json():
        render.emit({"command": "develop diff", "a": args.a, "b": args.b,
                     "differences": [{"field": develop_names.label(c),
                                      "a": develop_names.format_value(c, x),
                                      "b": develop_names.format_value(c, y)}
                                     for c, x, y in changes]})
        return 0
    say(f"{first['filename']} (#{args.a})   vs   {second['filename']} (#{args.b})")
    table(["FIELD", str(args.a), str(args.b)],
          [[develop_names.label(c), develop_names.format_value(c, x),
            develop_names.format_value(c, y)] for c, x, y in changes], "lll")
    summary([f"{len(changes)} difference(s)"])
    return 0


def register(sub) -> None:
    parser = sub.add_parser("develop", help="report develop settings (read-only)")
    inner = parser.add_subparsers(dest="develop_command", required=True)

    p = inner.add_parser("report", help="what was edited, and how")
    selector.add_arguments(p)
    p.add_argument("--verbose", action="store_true", help="per-photo adjustments instead of a summary")
    p.set_defaults(func=report)

    p = inner.add_parser("styles", help="which styles are applied across the catalog")
    p.set_defaults(func=styles)

    p = inner.add_parser("diff", help="compare two variants' adjustments")
    p.add_argument("--a", type=int, required=True, metavar="PK")
    p.add_argument("--b", type=int, required=True, metavar="PK")
    p.set_defaults(func=diff)
