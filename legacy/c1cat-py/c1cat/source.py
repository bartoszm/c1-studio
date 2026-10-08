"""Opening the live catalog and reading its shape."""

from __future__ import annotations

import sqlite3
from contextlib import contextmanager
from pathlib import Path

from . import config


def read_only(catalog: Path, immutable: bool = False) -> sqlite3.Connection:
    """Open the catalog read-only.

    ``immutable`` promises the file will not change while open, which lets SQLite
    skip locking entirely. Use it for a one-shot read, never for a long session
    that might straddle a Capture One save.
    """
    uri = f"file:{catalog}?mode=ro" + ("&immutable=1" if immutable else "")
    con = sqlite3.connect(uri, uri=True, timeout=30.0)
    con.row_factory = sqlite3.Row
    return con


@contextmanager
def reading(catalog: Path, immutable: bool = False):
    """read_only() that actually closes on exit.

    sqlite3's own context manager commits the transaction but leaves the handle open,
    and a stray handle on a DrvFs mount makes Windows report the catalog as locked.
    """
    con = read_only(catalog, immutable=immutable)
    try:
        yield con
    finally:
        con.close()


def read_write(catalog: Path) -> sqlite3.Connection:
    con = sqlite3.connect(str(catalog), isolation_level=None, timeout=30.0)
    con.row_factory = sqlite3.Row
    return con


def schema_info(con: sqlite3.Connection) -> dict:
    """Version, journal mode and headline counts."""
    row = con.execute(
        "SELECT ZVERSION, ZCOMPATIBLEVERSION, ZAUTHOR FROM ZVERSIONINFO ORDER BY Z_PK DESC LIMIT 1"
    ).fetchone()
    info = {
        "zversion": row["ZVERSION"] if row else None,
        "compatible_version": row["ZCOMPATIBLEVERSION"] if row else None,
        "author": row["ZAUTHOR"] if row else None,
        "journal_mode": con.execute("PRAGMA journal_mode").fetchone()[0],
    }
    info["version_supported"] = info["zversion"] == config.EXPECTED_ZVERSION
    return info


def counts(con: sqlite3.Connection) -> dict:
    def one(sql: str) -> int:
        return con.execute(sql).fetchone()[0]

    return {
        "images": one("SELECT COUNT(*) FROM ZIMAGE"),
        "variants": one("SELECT COUNT(*) FROM ZVARIANT"),
        "collections": one("SELECT COUNT(*) FROM ZCOLLECTION"),
        "albums": one(f"SELECT COUNT(*) FROM ZCOLLECTION WHERE Z_ENT = {config.ENT_ALBUM}"),
        "smart_albums": one(f"SELECT COUNT(*) FROM ZCOLLECTION WHERE Z_ENT = {config.ENT_SMART}"),
        "keywords": one("SELECT COUNT(*) FROM ZKEYWORD"),
    }


def metadata_rows(con: sqlite3.Connection, variant_pk: int) -> list[int]:
    """The two ZVARIANTMETADATA rows a metadata write must touch.

    User-visible metadata lives on the adjustment layer and is mirrored onto the
    combined layer. The default layer holds what was embedded in the file itself
    and must never be written.
    """
    rows = con.execute(
        "SELECT ZMETADATA FROM ZVARIANTLAYER WHERE Z_PK IN ("
        "  SELECT ZADJUSTMENTLAYER FROM ZVARIANT WHERE Z_PK = ?"
        "  UNION ALL"
        "  SELECT ZCOMBINEDSETTINGS FROM ZVARIANT WHERE Z_PK = ?)",
        (variant_pk, variant_pk),
    ).fetchall()
    return [r[0] for r in rows if r[0] is not None]


def layer_kinds(con: sqlite3.Connection, variant_pk: int) -> dict[int, int]:
    """Map ZVARIANTMETADATA.Z_PK -> the Z_ENT of the layer that owns it."""
    rows = con.execute(
        "SELECT l.ZMETADATA, l.Z_ENT FROM ZVARIANTLAYER l "
        "JOIN ZVARIANT v ON l.Z_PK IN (v.ZADJUSTMENTLAYER, v.ZCOMBINEDSETTINGS) "
        "WHERE v.Z_PK = ?", (variant_pk,)).fetchall()
    return {r[0]: r[1] for r in rows if r[0] is not None}
