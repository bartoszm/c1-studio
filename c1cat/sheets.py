"""Contact sheets.

Adapted from /home/bartoszm/projects/photo_analyser/create_sheets.py - same white
canvas, same geometry, same DejaVu subtitle with binary-search truncation. Two
changes: cells come from a catalog selection rather than a directory scan, and the
subtitle carries the variant primary key.

The label is the only thing tying a model's answer back to a photo, and filenames
are not unique across a 100k catalog (nor across variants of one image), so the
default label leads with the variant PK.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path

from . import images

FONT_CANDIDATES = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
]


@dataclass
class Cell:
    variant_pk: int
    filename: str
    label: str
    tier: str = ""
    position: int = 0
    row: int = 0
    col: int = 0


@dataclass
class Sheet:
    path: Path
    cells: list[Cell] = field(default_factory=list)


def label_for(row, style: str) -> str:
    pk = row["variant_pk"]
    stem = Path(row["filename"]).stem
    if style == "pk":
        return f"v{pk}"
    if style == "filename":
        return row["filename"]
    return f"v{pk} {stem}"


def _font(size: int):
    from PIL import ImageFont
    for candidate in FONT_CANDIDATES:
        if Path(candidate).exists():
            try:
                return ImageFont.truetype(candidate, size)
            except OSError:
                continue
    return ImageFont.load_default()


def _fit_text(draw, text: str, font, max_width: int) -> str:
    """Truncate from the left with a leading ellipsis, as the original does."""
    if draw.textbbox((0, 0), text, font=font)[2] <= max_width:
        return text
    low, high = 0, len(text)
    while low < high:
        mid = (low + high + 1) // 2
        if draw.textbbox((0, 0), "..." + text[mid:], font=font)[2] <= max_width:
            high = mid - 1
        else:
            low = mid
    return "..." + text[low + 1:]


def build_sheet(entries: list[tuple[object, Cell]], output: Path, con=None, *,
                grid: int = 6, long_edge: int = 2000, spacing: int = 20,
                font_size: int = 24, tier: str = "thumb", quality: int = 88,
                on_error=None) -> Sheet:
    from PIL import Image, ImageDraw

    text_height = font_size + 20
    available = long_edge - (grid + 1) * spacing
    cell_size = available // grid
    canvas = Image.new("RGB", (long_edge, long_edge + text_height * grid), "white")
    draw = ImageDraw.Draw(canvas)
    font = _font(font_size)

    sheet = Sheet(path=output)
    for index, (row, cell) in enumerate(entries[: grid * grid]):
        r, c = divmod(index, grid)
        x = spacing + c * (cell_size + spacing)
        y = spacing + r * (cell_size + spacing + text_height)
        try:
            image, used = images.load(row, con=con, tier=tier)
        except images.ImageError as exc:
            if on_error:
                on_error(cell.variant_pk, str(exc))
            continue
        image.thumbnail((cell_size, cell_size), Image.Resampling.LANCZOS)
        canvas.paste(image, (x + (cell_size - image.width) // 2,
                             y + (cell_size - image.height) // 2))

        text = _fit_text(draw, cell.label, font, cell_size - 20)
        width = draw.textbbox((0, 0), text, font=font)[2]
        draw.text((x + (cell_size - width) // 2, y + cell_size + 5), text, fill="black", font=font)

        cell.tier = used
        cell.position, cell.row, cell.col = index, r, c
        sheet.cells.append(cell)

    output.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(output, quality=quality)
    return sheet


def build_all(rows, out_dir: Path, con=None, *, grid: int = 6, long_edge: int = 2000,
              tier: str = "thumb", label: str = "both", spacing: int = 20,
              font_size: int = 24, progress=None, on_error=None) -> list[Sheet]:
    out_dir.mkdir(parents=True, exist_ok=True)
    per_sheet = grid * grid
    sheets: list[Sheet] = []
    for number, start in enumerate(range(0, len(rows), per_sheet), start=1):
        chunk = rows[start:start + per_sheet]
        entries = [(row, Cell(row["variant_pk"], row["filename"], label_for(row, label)))
                   for row in chunk]
        path = out_dir / f"sheet_{number:03d}.jpg"
        sheet = build_sheet(entries, path, con=con, grid=grid, long_edge=long_edge,
                            spacing=spacing, font_size=font_size, tier=tier, on_error=on_error)
        sheets.append(sheet)
        if progress:
            progress(f"  {path.name}: {len(sheet.cells)} photo(s)")
    return sheets


def write_manifest(sheets: list[Sheet], path: Path, extra: dict | None = None) -> dict:
    payload = {
        "sheets": [
            {
                "file": sheet.path.name,
                "cells": [
                    {"position": c.position, "row": c.row, "col": c.col, "label": c.label,
                     "variant_pk": c.variant_pk, "filename": c.filename, "tier": c.tier}
                    for c in sheet.cells
                ],
            }
            for sheet in sheets
        ],
    }
    if extra:
        payload.update(extra)
    path.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n")
    return payload


def labels_to_variants(manifest: dict) -> dict[str, int]:
    """The lookup that maps a model's answer back onto the catalog."""
    out: dict[str, int] = {}
    for sheet in manifest.get("sheets", []):
        for cell in sheet.get("cells", []):
            out[cell["label"]] = cell["variant_pk"]
            out[f"v{cell['variant_pk']}"] = cell["variant_pk"]
            out[str(cell["variant_pk"])] = cell["variant_pk"]
            out[cell["filename"]] = cell["variant_pk"]
    return out
