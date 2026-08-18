"""Locating the catalog, the caches and the backup directory."""

from __future__ import annotations

import hashlib
import json
import os
import re
from pathlib import Path

VERSION = "0.1.0"

# The catalog schema this tool was written against (ZVERSIONINFO.ZVERSION).
EXPECTED_ZVERSION = 160800

# Capture One's colour tag order. Index 0 means "no tag".
COLOR_NAMES = ["none", "red", "orange", "yellow", "green", "blue", "pink", "purple"]
COLOR_INDEX = {name: i for i, name in enumerate(COLOR_NAMES)}

# ZENTITIES ids we depend on.
ENT_ALBUM = 2
ENT_SMART = 5
ENT_PROJECT = 7
ENT_GROUP = 8
ENT_TRASH = 11
ENT_ADJUSTMENT_LAYER = 17
ENT_DEFAULT_LAYER = 18
ENT_STYLE_LAYER = 19
ENT_COMBINED_LAYER = 20
ENT_LOCAL_LAYER = 23
ENT_KEYWORD = 44
ENT_IMAGE_IN_COLLECTION = 47
ENT_VARIANT_IN_COLLECTION = 48
ENT_RETOUCHING_LAYER = 51

ROOT_COLLECTION_PK = 1

CONFIG_PATH = Path.home() / ".config" / "c1cat" / "config.json"
CACHE_ROOT = Path(os.environ.get("C1CAT_CACHE", Path.home() / ".cache" / "c1cat"))
BACKUP_ROOT = Path(os.environ.get("C1CAT_BACKUP_DIR", Path.home() / ".local" / "share" / "c1cat" / "backups"))

WINDOWS_TOOLS = {
    "tasklist": "/mnt/c/Windows/System32/tasklist.exe",
    "powershell": "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe",
}

# Capture One processes that hold the catalog open.
C1_PROCESSES = ["CaptureOne.exe", "ImgCoreProcess.exe"]


class ConfigError(Exception):
    """Raised when the catalog cannot be located or is not usable."""


def _read_config() -> dict:
    try:
        return json.loads(CONFIG_PATH.read_text())
    except (OSError, ValueError):
        return {}


def find_catalog(explicit: str | None = None) -> Path:
    """Resolve the catalog path: --catalog, then $C1CAT_CATALOG, then config, then a search."""
    for candidate in (explicit, os.environ.get("C1CAT_CATALOG"), _read_config().get("catalog")):
        if candidate:
            path = Path(candidate).expanduser()
            if not path.is_file():
                raise ConfigError(f"catalog not found: {path}")
            return path.resolve()

    found = sorted(Path("/mnt/c/c1_catalogs").glob("*/*.cocatalogdb")) if Path("/mnt/c/c1_catalogs").is_dir() else []
    # Capture One's own rolling backups sit next to the catalog as "name.cocatalogdb.backup N".
    found = [p for p in found if p.suffix == ".cocatalogdb"]
    if len(found) == 1:
        return found[0].resolve()
    if not found:
        raise ConfigError(
            "no catalog found. Pass --catalog PATH, set $C1CAT_CATALOG, "
            f"or write {{\"catalog\": \"...\"}} to {CONFIG_PATH}"
        )
    listing = "\n  ".join(str(p) for p in found)
    raise ConfigError(f"several catalogs found, pass --catalog to choose one:\n  {listing}")


def catalog_id(catalog: Path) -> str:
    """A stable per-catalog cache key: readable stem plus a hash of the full path."""
    digest = hashlib.sha1(str(catalog).encode()).hexdigest()[:8]
    stem = re.sub(r"[^a-z0-9]+", "-", catalog.stem.lower()).strip("-") or "catalog"
    return f"{stem}-{digest}"


def cache_dir(catalog: Path) -> Path:
    return CACHE_ROOT / catalog_id(catalog)


def mirror_path(catalog: Path) -> Path:
    return cache_dir(catalog) / "mirror.sqlite"


def catalog_dir(catalog: Path) -> Path:
    return catalog.parent


def thumbnails_dir(catalog: Path) -> Path:
    return catalog.parent / "Cache" / "Thumbnails"


def previews_dir(catalog: Path) -> Path:
    return catalog.parent / "Cache" / "Previews"
