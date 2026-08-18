"""A small synthetic catalog built from the real DDL.

The schema is dumped from the live catalog once into tests/fixtures/schema.sql so the
tests exercise the actual constraints - the UNIQUE indexes on ZKEYWORD.ZLEFT/ZRIGHT and
the CHECK(ZLEFT < ZRIGHT) are the whole reason the nested-set code looks the way it does.
"""

from __future__ import annotations

import sqlite3
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

FIXTURE_SCHEMA = Path(__file__).parent / "fixtures" / "schema.sql"
LIVE_CATALOG = Path("/mnt/c/c1_catalogs/fuji/fiuji.cocatalogdb")


def _schema_sql() -> str:
    if FIXTURE_SCHEMA.exists():
        return FIXTURE_SCHEMA.read_text()
    if not LIVE_CATALOG.exists():
        pytest.skip("no schema fixture and no live catalog to dump one from")
    out = subprocess.run(
        ["sqlite3", f"file:{LIVE_CATALOG}?mode=ro&immutable=1", ".schema"],
        capture_output=True, text=True, check=True).stdout
    FIXTURE_SCHEMA.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE_SCHEMA.write_text(out)
    return out


@pytest.fixture
def con() -> sqlite3.Connection:
    """An empty catalog with the real schema."""
    connection = sqlite3.connect(":memory:")
    connection.row_factory = sqlite3.Row
    for statement in _schema_sql().split(";\n"):
        if statement.strip() and "sqlite_stat" not in statement:
            try:
                connection.execute(statement)
            except sqlite3.OperationalError:
                pass
    connection.execute("INSERT INTO ZVERSIONINFO (Z_ENT, ZVERSION, ZCOMPATIBLEVERSION, ZAUTHOR)"
                       " VALUES (32, 160800, 160800, 'test')")
    yield connection
    connection.close()


@pytest.fixture
def catalog(con) -> sqlite3.Connection:
    """A catalog with a handful of images, variants, layers and keywords."""
    con.execute("INSERT INTO ZCOLLECTION (Z_ENT, Z_PK, ZUUID, ZNAME, ZPARENT, ZPROJECT,"
                " ZCOLLECTIONINDEX, ZDOCUMENT) VALUES (7, 1, 'root', '[Collection]', -1, -1, 0, 1)")
    con.execute("INSERT INTO ZPATHLOCATION (Z_ENT, Z_PK, ZWINROOT, ZRELATIVEPATH, ZISRELATIVE)"
                " VALUES (38, 1, 'H:\\', 'fotos', 0)")

    for image_pk in range(1, 6):
        con.execute(
            "INSERT INTO ZIMAGE (Z_ENT, Z_PK, ZIMAGEUUID, ZIMAGEFILENAME, ZDISPLAYNAME,"
            " ZIMAGELOCATION, ZISTRASHED, ZISINSIDECATALOG, ZEXP_DATE, ZIMPORTDATE,"
            " ZCAMERA_MODEL, ZCAMERA_LENS, ZISO, ZWIDTH, ZHEIGHT, ZSIDECARPATH)"
            " VALUES (12, ?, ?, ?, ?, 1, 0, 0, ?, ?, 'X-Pro2', '23mm', 400, 6000, 4000, ?)",
            (image_pk, f"uuid-img-{image_pk}", f"DSCF{image_pk:04d}.RAF", f"DSCF{image_pk:04d}",
             1700000000 + image_pk * 86400, 1700100000, f"2023/1/1/1/DSCF{image_pk:04d}.RAF"))

    # Each variant gets a default (18), adjustment (17) and combined (20) layer, each with
    # its own metadata row, exactly as Capture One lays them out.
    layer_pk = 0
    for variant_pk in range(1, 6):
        layers = {}
        for index, ent in enumerate((18, 17, 20)):
            layer_pk += 1
            con.execute("INSERT INTO ZVARIANTMETADATA (Z_ENT, Z_PK, ZLAYER) VALUES (13, ?, ?)",
                        (layer_pk, layer_pk))
            con.execute("INSERT INTO ZVARIANTLAYER (Z_ENT, Z_PK, ZVARIANT, ZINDEX, ZMETADATA)"
                        " VALUES (?, ?, ?, ?, ?)", (ent, layer_pk, variant_pk, index, layer_pk))
            layers[ent] = layer_pk
        con.execute(
            "INSERT INTO ZVARIANT (Z_ENT, Z_PK, ZVARIANTUUID, ZINDEX, ZIMAGE, ZISMODIFIED,"
            " ZADJUSTMENTLAYER, ZDEFAULTLAYER, ZCOMBINEDSETTINGS)"
            " VALUES (15, ?, ?, 0, ?, 0, ?, ?, ?)",
            (variant_pk, f"uuid-var-{variant_pk}", variant_pk,
             layers[17], layers[18], layers[20]))
    con.commit()
    return con
