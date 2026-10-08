"""preview export / sheets - turning a selection into files on disk."""

from __future__ import annotations

from pathlib import Path

from . import images, render, selector, sheets
from .render import say, summary


def export(args, ctx) -> int:
    rows, _ = selector.run(args, ctx.mirror)
    if not rows:
        say("(nothing matched)")
        return 0
    out = Path(args.out).expanduser()
    out.mkdir(parents=True, exist_ok=True)

    written, failed, tiers = [], [], {}
    for row in rows:
        try:
            image, used = images.load(row, con=ctx.mirror, tier=args.tier,
                                      fallback=not args.strict_tier)
            image = images.fit(image, args.long_edge)
            stem = (f"v{row['variant_pk']}" if args.name_by == "variant"
                    else Path(row["filename"]).stem)
            if args.name_by == "both":
                stem = f"v{row['variant_pk']}_{Path(row['filename']).stem}"
            path = out / f"{stem}.{ 'png' if args.format == 'png' else 'jpg'}"
            if args.format == "png":
                image.save(path)
            else:
                image.save(path, "JPEG", quality=args.quality, optimize=True)
            written.append({"variant_pk": row["variant_pk"], "path": str(path), "tier": used,
                            "size": list(image.size)})
            tiers[used] = tiers.get(used, 0) + 1
        except images.ImageError as exc:
            failed.append({"variant_pk": row["variant_pk"], "error": str(exc)})

    if render.use_json():
        render.emit({"command": "preview export", "out": str(out), "count": len(written),
                     "tiers": tiers, "written": written, "failed": failed})
        return 0
    for item in written[:20]:
        say(f"  {Path(item['path']).name}  ({item['tier']}, {item['size'][0]}x{item['size'][1]})")
    if len(written) > 20:
        say(f"  ... and {len(written) - 20} more")
    lines = [f"{len(written)} file(s) written to {out}"]
    if tiers:
        lines.append("tiers used: " + ", ".join(f"{k} x{v}" for k, v in sorted(tiers.items())))
    if failed:
        lines.append(f"{len(failed)} failed (first: {failed[0]['error'][:80]})")
    summary(lines)
    return 0


def contact_sheets(args, ctx) -> int:
    rows, _ = selector.run(args, ctx.mirror)
    if not rows:
        say("(nothing matched)")
        return 0
    out = Path(args.out).expanduser()

    if args.grid <= 4 and args.tier == "thumb":
        cell = (args.long_edge - (args.grid + 1) * 20) // args.grid
        render.warn(f"a {args.grid}x{args.grid} grid gives {cell}px cells but thumbnails are "
                    f"only 450px - consider --tier preview")

    problems: list[str] = []
    built = sheets.build_all(
        rows, out, con=ctx.mirror, grid=args.grid, long_edge=args.long_edge,
        tier=args.tier, label=args.label, progress=say,
        on_error=lambda pk, message: problems.append(f"{pk}: {message}"))

    manifest_path = out / "manifest.json"
    sheets.write_manifest(built, manifest_path, extra={
        "grid": args.grid, "tier": args.tier, "label": args.label,
        "catalog": str(ctx.catalog), "selected": len(rows),
    })

    tiers: dict[str, int] = {}
    for sheet in built:
        for cell in sheet.cells:
            tiers[cell.tier] = tiers.get(cell.tier, 0) + 1

    if render.use_json():
        render.emit({"command": "sheets", "out": str(out), "sheets": len(built),
                     "photos": sum(len(s.cells) for s in built), "tiers": tiers,
                     "manifest": str(manifest_path), "problems": problems})
        return 0
    lines = [f"{len(built)} sheet(s), {sum(len(s.cells) for s in built)} photo(s) in {out}",
             f"manifest: {manifest_path}"]
    if tiers:
        lines.append("tiers used: " + ", ".join(f"{k} x{v}" for k, v in sorted(tiers.items())))
    if problems:
        lines.append(f"{len(problems)} photo(s) could not be rendered")
    summary(lines)
    return 0


def register(sub) -> None:
    p = sub.add_parser("preview", help="export viewable images")
    inner = p.add_subparsers(dest="preview_command", required=True)
    e = inner.add_parser("export", help="write JPEGs for the matching photos")
    selector.add_arguments(e)
    e.add_argument("--out", required=True, metavar="DIR")
    e.add_argument("--tier", default="thumb", choices=list(images.TIERS))
    e.add_argument("--strict-tier", action="store_true", help="fail instead of falling back")
    e.add_argument("--long-edge", type=int, default=0, help="downscale to this long edge")
    e.add_argument("--format", default="jpeg", choices=["jpeg", "png"])
    e.add_argument("--quality", type=int, default=90)
    e.add_argument("--name-by", default="both", choices=["variant", "filename", "both"])
    e.set_defaults(func=export)

    p = sub.add_parser("sheets", help="build contact sheets from a selection")
    selector.add_arguments(p)
    p.add_argument("--out", required=True, metavar="DIR")
    p.add_argument("--grid", type=int, default=6)
    p.add_argument("--long-edge", type=int, default=2000)
    p.add_argument("--tier", default="thumb", choices=list(images.TIERS))
    p.add_argument("--label", default="both", choices=["pk", "filename", "both"])
    p.set_defaults(func=contact_sheets)
