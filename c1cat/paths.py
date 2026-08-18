"""Translating catalog path records into paths this machine can open."""

from __future__ import annotations

import re
from pathlib import Path

_DRIVE = re.compile(r"^([A-Za-z]):[\\/]*$")
_UNC = re.compile(r"^\\\\")


def win_to_wsl(win_root: str) -> str | None:
    """'H:\\' -> '/mnt/h/'. UNC roots have no mount point, so they return None."""
    if not win_root:
        return None
    m = _DRIVE.match(win_root.strip())
    if m:
        return f"/mnt/{m.group(1).lower()}/"
    if _UNC.match(win_root):
        return None
    return None


def wsl_to_win(path: Path | str) -> str | None:
    """'/mnt/c/foo/bar' -> 'C:\\foo\\bar'. Returns None for paths outside a drive mount."""
    text = str(path)
    m = re.match(r"^/mnt/([a-z])(/.*)?$", text)
    if not m:
        return None
    rest = (m.group(2) or "/").lstrip("/").replace("/", "\\")
    return f"{m.group(1).upper()}:\\{rest}"


def original_paths(win_root: str, relative_path: str, filename: str,
                   is_relative: bool, catalog_dir: Path) -> tuple[str | None, str | None]:
    """Return (windows_path, wsl_path) for an image's original file.

    ``is_relative`` images live inside the catalog bundle (under ``Originals/``);
    everything else is anchored at a Windows drive root.
    """
    rel = (relative_path or "").strip("\\/")
    if is_relative:
        wsl = catalog_dir.joinpath(*rel.split("\\")) / filename if rel else catalog_dir / filename
        return wsl_to_win(wsl), str(wsl)

    win = f"{win_root}{rel}\\{filename}" if rel else f"{win_root}{filename}"
    mount = win_to_wsl(win_root)
    if mount is None:
        return win, None
    wsl = f"{mount}{rel.replace(chr(92), '/')}/{filename}" if rel else f"{mount}{filename}"
    return win, wsl


def thumbnail_path(catalog_dir: Path, variant_pk: int) -> Path:
    """Capture One's browser thumbnail: a plain 450px JPEG, keyed by variant."""
    return catalog_dir / "Cache" / "Thumbnails" / f"{variant_pk:08d}.cot"


def preview_path_unchecked(catalog_dir: Path, sidecar_path: str | None) -> Path | None:
    """Where the JPEG-XL preview should be, derived purely from ZIMAGE.ZSIDECARPATH.

    No filesystem access: the catalog lives on a 9p mount where a stat costs
    milliseconds, and the index builds this for every variant.
    """
    if not sidecar_path:
        return None
    return catalog_dir / "Cache" / "Previews" / (sidecar_path.replace("\\", "/") + ".cop")


def preview_path(catalog_dir: Path, sidecar_path: str | None) -> Path | None:
    """As above, but confirmed to exist.

    ZSIDECARPATH is sometimes lower-cased relative to the real filename, so fall back
    to a case-insensitive scan of the one containing directory. Call this when you are
    about to open the file, never in a loop over the whole catalog.
    """
    candidate = preview_path_unchecked(catalog_dir, sidecar_path)
    if candidate is None:
        return None
    if candidate.exists():
        return candidate
    parent = candidate.parent
    if parent.is_dir():
        wanted = candidate.name.lower()
        for entry in parent.iterdir():
            if entry.name.lower() == wanted:
                return entry
    return None


def parse_gps(value: str | None) -> float | None:
    """'44.803944444445N' / '20.37835E' -> signed decimal degrees."""
    if not value:
        return None
    text = value.strip()
    if not text:
        return None
    sign = 1.0
    if text[-1] in "NSEWnsew":
        if text[-1] in "SWsw":
            sign = -1.0
        text = text[:-1]
    try:
        return sign * float(text)
    except ValueError:
        return None


def format_gps(value: float, is_latitude: bool) -> str:
    """Inverse of parse_gps, matching Capture One's own formatting.

    repr() gives the shortest representation that round-trips exactly; a fixed
    precision such as %.12g silently truncates the 14-significant-figure coordinates
    Capture One actually writes (44.803944444445 would lose its last two digits).
    """
    letter = ("N" if value >= 0 else "S") if is_latitude else ("E" if value >= 0 else "W")
    return f"{repr(abs(float(value)))}{letter}"
