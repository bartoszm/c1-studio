"""c1cat - work with a Capture One catalog from the command line."""

from __future__ import annotations

import argparse
import shutil
import sqlite3
import sys
import time
from dataclasses import dataclass
from pathlib import Path

from . import config, mirror, render, safety, selector, source
from .render import say, summary


@dataclass
class Context:
    catalog: Path
    args: argparse.Namespace
    _mirror: sqlite3.Connection | None = None

    @property
    def mirror(self) -> sqlite3.Connection:
        if self._mirror is None:
            state = mirror.ensure_fresh(
                self.catalog, allow_build=not self.args.no_sync, progress=say)
            if state.get("stale"):
                render.warn("local index is out of date and --no-sync was given")
            self._mirror = mirror.open_mirror(self.catalog)
        return self._mirror


# --------------------------------------------------------------------------
# status / doctor / sync
# --------------------------------------------------------------------------

def cmd_status(args, ctx) -> int:
    catalog = ctx.catalog
    stat = catalog.stat()
    with source.reading(catalog) as con:
        info = source.schema_info(con)
        totals = source.counts(con)
    state = mirror.state(catalog)
    process = safety.detect_capture_one()
    backups = safety.list_backups(catalog)

    if render.use_json():
        render.emit({
            "command": "status",
            "catalog": {"path": str(catalog), "size": stat.st_size, "modified": render.when(stat.st_mtime)},
            "schema": info,
            "counts": totals,
            "mirror": state,
            "capture_one": process,
            "backups": {"count": len(backups), "newest": backups[0]["created"] if backups else None,
                        "dir": str(config.BACKUP_ROOT)},
        })
        return 0

    say(f"Catalog   {catalog}")
    say(f"          {render.size(stat.st_size)}   modified {render.when(stat.st_mtime, '%Y-%m-%d %H:%M:%S')}")
    version_note = "supported" if info["version_supported"] else f"UNTESTED (expected {config.EXPECTED_ZVERSION})"
    say(f"Version   ZVERSION={info['zversion']} ({info['author']})   journal={info['journal_mode']}   {version_note}")
    say(f"C1        {process['status']} - {process['detail']}")
    freshness = "fresh" if state["fresh"] else ("stale" if state["exists"] else "not built")
    say(f"Index     {freshness}"
        + (f"   built {state['built_at']}   {render.size(state['size'])}" if state["exists"] else "")
        + f"\n          {state['path']}")
    say(f"Content   {totals['images']:,} images   {totals['variants']:,} variants   "
        f"{totals['albums']:,} albums   {totals['smart_albums']:,} smart albums   "
        f"{totals['keywords']:,} keywords")
    if backups:
        total = sum(b["size"] for b in backups)
        say(f"Backups   {len(backups)} in {config.BACKUP_ROOT} ({render.size(total)}), newest {backups[0]['created']}")
    else:
        say(f"Backups   none yet in {config.BACKUP_ROOT}")
    return 0


def _probe_import(name: str) -> tuple[bool, str]:
    try:
        module = __import__(name)
        return True, getattr(module, "__version__", "ok")
    except Exception as exc:  # noqa: BLE001 - report whatever went wrong
        return False, str(exc)[:60]


def cmd_doctor(args, ctx) -> int:
    checks = []

    catalog = ctx.catalog
    checks.append({"check": "catalog", "status": "ok", "detail": str(catalog)})
    with source.reading(catalog) as con:
        info = source.schema_info(con)
    checks.append({"check": "catalog version", "status": "ok" if info["version_supported"] else "warn",
                   "detail": f"ZVERSION={info['zversion']}"})
    checks.append({"check": "journal mode", "status": "ok" if info["journal_mode"] == "delete" else "fail",
                   "detail": info["journal_mode"]})

    for label, path in config.WINDOWS_TOOLS.items():
        exists = Path(path).exists()
        checks.append({"check": f"{label}.exe", "status": "ok" if exists else "warn", "detail": path})

    process = safety.detect_capture_one()
    checks.append({"check": "Capture One", "status": {"stopped": "ok", "running": "warn"}.get(process["status"], "warn"),
                   "detail": process["detail"]})
    lock = safety.probe_windows_lock(catalog)
    checks.append({"check": "windows lock probe", "status": "ok" if lock["status"] == "free" else "warn",
                   "detail": lock["detail"]})

    thumbs = config.thumbnails_dir(catalog)
    checks.append({"check": "thumbnail cache", "status": "ok" if thumbs.is_dir() else "fail",
                   "detail": str(thumbs)})
    previews = config.previews_dir(catalog)
    checks.append({"check": "preview cache", "status": "ok" if previews.is_dir() else "warn",
                   "detail": str(previews)})

    # Originals commonly live on an external drive that is not always plugged in.
    try:
        from . import images
        volumes = {}
        for row in ctx.mirror.execute(
                "SELECT folder_wsl, COUNT(*) FROM v WHERE folder_wsl IS NOT NULL"
                " GROUP BY folder_wsl ORDER BY COUNT(*) DESC LIMIT 200"):
            volume = images.volume_of(row[0])
            if volume:
                volumes[str(volume)] = volumes.get(str(volume), 0) + row[1]
        for volume in sorted(volumes, key=lambda v: -volumes[v]):
            try:
                # is_dir() raises ENODEV rather than returning False for an absent volume.
                mounted = Path(volume).is_dir()
            except OSError:
                mounted = False
            checks.append({
                "check": f"originals on {volume}",
                "status": "ok" if mounted else "warn",
                "detail": f"mounted, {volumes[volume]:,} photos" if mounted
                          else "NOT MOUNTED - the thumb and preview tiers still work",
            })
    except Exception as exc:  # noqa: BLE001 - doctor must never be the thing that breaks
        checks.append({"check": "originals volume", "status": "warn", "detail": str(exc)[:60]})

    ok, detail = _probe_import("PIL")
    checks.append({"check": "Pillow", "status": "ok" if ok else "fail", "detail": detail})
    ok, detail = _probe_import("imagecodecs")
    checks.append({"check": "imagecodecs (JPEG-XL previews)", "status": "ok" if ok else "warn",
                   "detail": detail if ok else "pip install imagecodecs - without it, use --tier thumb"})
    ok, detail = _probe_import("numpy")
    checks.append({"check": "numpy", "status": "ok" if ok else "warn", "detail": detail})

    agy = shutil.which("agy") or str(Path.home() / ".local/bin/agy")
    checks.append({"check": "agy (Gemini CLI)", "status": "ok" if Path(agy).exists() else "warn",
                   "detail": agy})

    free = safety.free_space(config.BACKUP_ROOT.parent if config.BACKUP_ROOT.exists()
                             else Path.home())
    checks.append({"check": "backup space", "status": "ok" if free > 5 * 2**30 else "warn",
                   "detail": f"{render.size(free)} free at {config.BACKUP_ROOT}"})

    state = mirror.state(catalog)
    checks.append({"check": "local index", "status": "ok" if state["fresh"] else "warn",
                   "detail": "fresh" if state["fresh"] else ("stale - run c1cat sync" if state["exists"]
                                                             else "not built - run c1cat sync")})

    if render.use_json():
        render.emit({"command": "doctor", "checks": checks})
        return 0
    render.table(["CHECK", "STATUS", "DETAIL"],
                 [[c["check"], c["status"], c["detail"]] for c in checks], "lll")
    failures = [c for c in checks if c["status"] == "fail"]
    summary([f"{len(checks)} checks, {len(failures)} failing"])
    return 1 if failures else 0


def cmd_sync(args, ctx) -> int:
    state = mirror.state(ctx.catalog)
    if state["fresh"] and not args.force:
        say(f"index is already current (built {state['built_at']})")
        if render.use_json():
            render.emit({"command": "sync", "rebuilt": False, "mirror": state})
        return 0
    state = mirror.build(ctx.catalog, progress=say)
    if render.use_json():
        render.emit({"command": "sync", "rebuilt": True, "mirror": state})
    return 0


def cmd_backup(args, ctx) -> int:
    if args.list:
        records = safety.list_backups(ctx.catalog)
        if render.use_json():
            render.emit({"command": "backup", "backups": records})
            return 0
        render.table(["CREATED", "SIZE", "FILE"],
                     [[r["created"], render.size(r["size"]), Path(r["backup"]).name] for r in records], "llr")
        summary([f"{len(records)} backup(s) in {config.BACKUP_ROOT}"])
        return 0

    if args.restore:
        return _restore(args, ctx)

    safety.preflight(ctx.catalog, write=False)
    say(f"backing up {ctx.catalog.name} ({render.size(ctx.catalog.stat().st_size)}) ...")
    record = safety.make_backup(ctx.catalog, argv=sys.argv[1:], keep=args.keep)
    if render.use_json():
        render.emit({"command": "backup", "backup": record})
    else:
        if record.get("reused"):
            say(f"catalog unchanged since {record['created']}, reusing {Path(record['backup']).name}")
        else:
            say(f"wrote {record['backup']} in {record['duration_s']}s")
    return 0


def _restore(args, ctx) -> int:
    records = safety.list_backups(ctx.catalog)
    target = Path(args.restore) if "/" in args.restore else None
    if target is None:
        try:
            target = Path(records[int(args.restore)]["backup"])
        except (ValueError, IndexError):
            say(f"no such backup: {args.restore}. Use 'c1cat backup --list'.")
            return safety.EXIT_USAGE
    if not target.is_file():
        say(f"backup file missing: {target}")
        return safety.EXIT_USAGE

    safety.preflight(ctx.catalog, write=True, force=args.force)
    if not args.apply:
        say(f"DRY RUN - would restore {target} over {ctx.catalog}")
        say("Re-run with --apply to do it.")
        return 0
    safety.make_backup(ctx.catalog, argv=["pre-restore"], keep=args.keep)
    shutil.copyfile(target, ctx.catalog)
    say(f"restored {target} -> {ctx.catalog}")
    say("Reopen Capture One to see the restored state.")
    return 0


# --------------------------------------------------------------------------
# parser
# --------------------------------------------------------------------------

def build_parser() -> argparse.ArgumentParser:
    from . import cmd_collections, cmd_develop, cmd_images, cmd_keywords, cmd_meta, cmd_query, cmd_vision

    parser = argparse.ArgumentParser(
        prog="c1cat", description="Work with a Capture One catalog from the command line.")
    parser.add_argument("--catalog", help="path to the .cocatalogdb")
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    parser.add_argument("--quiet", action="store_true")
    parser.add_argument("--no-sync", action="store_true", help="never rebuild the local index")
    parser.add_argument("--version", action="version", version=f"c1cat {config.VERSION}")
    sub = parser.add_subparsers(dest="command", required=True)

    def write_flags(p):
        p.add_argument("--apply", action="store_true", help="commit (default is a dry run)")
        p.add_argument("--force", action="store_true", help="bypass the Capture One guard")
        p.add_argument("--no-backup", action="store_true")
        p.add_argument("--keep", type=int, default=5, help="backups to retain")

    p = sub.add_parser("status", help="where things stand")
    p.set_defaults(func=cmd_status)

    p = sub.add_parser("doctor", help="check the environment")
    p.set_defaults(func=cmd_doctor)

    p = sub.add_parser("sync", help="rebuild the local index")
    p.add_argument("--force", action="store_true")
    p.set_defaults(func=cmd_sync)

    p = sub.add_parser("backup", help="back up or restore the catalog")
    p.add_argument("--list", action="store_true")
    p.add_argument("--restore", metavar="N|PATH")
    write_flags(p)
    p.set_defaults(func=cmd_backup)

    p = sub.add_parser("find", help="search the catalog")
    selector.add_arguments(p)
    p.add_argument("--fields", help=f"columns to show (default: {','.join(cmd_query.DEFAULT_FIELDS)})")
    p.add_argument("--count", action="store_true", help="print only the number of matches")
    p.set_defaults(func=cmd_query.find)

    p = sub.add_parser("count", help="count matches")
    selector.add_arguments(p)
    p.set_defaults(func=cmd_query.count)

    p = sub.add_parser("show", help="everything about the matching variants")
    selector.add_arguments(p)
    p.set_defaults(func=cmd_query.show)

    p = sub.add_parser("stats", help="group and tally")
    selector.add_arguments(p)
    p.add_argument("--by", required=True,
                   choices=sorted(set(cmd_query.GROUPINGS) | {"keyword", "album"}))
    p.add_argument("--limit-groups", type=int, default=40)
    p.set_defaults(func=cmd_query.stats)

    p = sub.add_parser("paths", help="print file paths")
    selector.add_arguments(p)
    p.add_argument("--tier", default="original",
                   choices=["original", "windows", "thumb", "preview"])
    p.set_defaults(func=cmd_query.paths_cmd)

    cmd_meta.register(sub, write_flags)
    cmd_keywords.register(sub, write_flags)
    cmd_collections.register(sub, write_flags)
    cmd_develop.register(sub)
    cmd_images.register(sub)
    cmd_vision.register(sub, write_flags)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    render.STATE["json"] = bool(getattr(args, "json", False))
    render.STATE["quiet"] = bool(getattr(args, "quiet", False))

    try:
        catalog = config.find_catalog(args.catalog)
    except config.ConfigError as exc:
        print(f"c1cat: {exc}", file=sys.stderr)
        return safety.EXIT_USAGE

    ctx = Context(catalog=catalog, args=args)
    try:
        return args.func(args, ctx) or 0
    except safety.Refused as exc:
        print(f"c1cat: refused: {exc}", file=sys.stderr)
        if exc.hint:
            print(f"        {exc.hint}", file=sys.stderr)
        return exc.exit_code
    except (selector.SelectorError, mirror.MirrorError, config.ConfigError) as exc:
        print(f"c1cat: {exc}", file=sys.stderr)
        return safety.EXIT_USAGE
    except BrokenPipeError:
        return 0
    finally:
        if ctx._mirror is not None:
            ctx._mirror.close()


if __name__ == "__main__":
    sys.exit(main())
