"""Human tables and machine JSON."""

from __future__ import annotations

import json
import os
import sys
import time

STATE = {"json": False, "quiet": False}


def use_json() -> bool:
    return STATE["json"]


def emit(payload: dict) -> None:
    """The single JSON exit point. Every command's --json output goes through here."""
    json.dump(payload, sys.stdout, indent=2, ensure_ascii=False, default=str)
    sys.stdout.write("\n")


def say(*parts: str) -> None:
    # flush: progress from a long index build is usually piped, and block buffering
    # would hide it until the very end.
    if not STATE["quiet"] and not STATE["json"]:
        print(*parts, flush=True)


def warn(message: str) -> None:
    if not STATE["json"]:
        print(f"warning: {message}", file=sys.stderr)


def summary(lines: list[str]) -> None:
    if STATE["json"] or STATE["quiet"]:
        return
    print("--- Summary ---")
    for line in lines:
        print(line)


def terminal_width(default: int = 120) -> int:
    try:
        return max(60, os.get_terminal_size().columns)
    except OSError:
        return default


def table(headers: list[str], rows: list[list], aligns: str | None = None,
          max_width: int | None = None) -> None:
    """Print an aligned table, shrinking the widest column to fit the terminal."""
    if STATE["json"] or STATE["quiet"]:
        return
    if not rows:
        print("(nothing)")
        return
    cells = [[("" if c is None else str(c)) for c in row] for row in rows]
    widths = [max(len(h), *(len(r[i]) for r in cells)) for i, h in enumerate(headers)]
    aligns = aligns or "l" * len(headers)
    limit = max_width or terminal_width()

    total = sum(widths) + 2 * (len(widths) - 1)
    if total > limit:
        widest = max(range(len(widths)), key=lambda i: widths[i])
        widths[widest] = max(8, widths[widest] - (total - limit))

    def fmt(text: str, width: int, align: str) -> str:
        if len(text) > width:
            text = text[: width - 1] + "…"
        return text.rjust(width) if align == "r" else text.ljust(width)

    print("  ".join(fmt(h, widths[i], aligns[i]) for i, h in enumerate(headers)))
    print("  ".join("-" * w for w in widths))
    for row in cells:
        print("  ".join(fmt(c, widths[i], aligns[i]) for i, c in enumerate(row)))


def when(stamp: float | None, fmt: str = "%Y-%m-%d %H:%M") -> str:
    if not stamp:
        return ""
    return time.strftime(fmt, time.localtime(stamp))


def size(num_bytes: float | None) -> str:
    if not num_bytes:
        return "0 B"
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if abs(num_bytes) < 1024 or unit == "TB":
            return f"{num_bytes:.0f} {unit}" if unit == "B" else f"{num_bytes:.1f} {unit}"
        num_bytes /= 1024
    return ""


def variant_dict(row) -> dict:
    """The stable per-variant JSON shape."""
    keywords_raw = row["keywords_raw"]
    return {
        "variant_pk": row["variant_pk"],
        "variant_uuid": row["variant_uuid"],
        "variant_index": row["variant_index"],
        "image_pk": row["image_pk"],
        "image_uuid": row["image_uuid"],
        "filename": row["filename"],
        "display_name": row["display_name"],
        "path_win": row["path_win"],
        "path_wsl": row["path_wsl"],
        "thumb": row["thumb_path"],
        "preview": row["preview_path"],
        "capture_date": when(row["capture_ts"], "%Y-%m-%dT%H:%M:%S") or None,
        "import_date": when(row["import_ts"], "%Y-%m-%dT%H:%M:%S") or None,
        "camera": row["camera_model"],
        "lens": row["lens"],
        "iso": row["iso"],
        "aperture": row["aperture"],
        "shutter": row["shutter"],
        "focal": row["focal"],
        "width": row["width"],
        "height": row["height"],
        "rating": row["rating"],
        "color": row["color_name"],
        "keywords": [p for p, _ in _parse(keywords_raw)],
        "description": row["description"],
        "headline": row["headline"],
        "title": row["title"],
        "creator": row["creator"],
        "copyright": row["copyright"],
        "city": row["city"],
        "state": row["state"],
        "country": row["country"],
        "location": row["location"],
        "gps": ({"lat": row["lat"], "lon": row["lon"]} if row["lat"] is not None else None),
        "edited": bool(row["is_modified"]),
        "styles": (row["style_names"] or "").split(";") if row["style_names"] else [],
        "local_layers": row["n_local_layers"],
        "trashed": bool(row["is_trashed"]),
    }


def _parse(serialised):
    from .keywords import parse
    return parse(serialised)
