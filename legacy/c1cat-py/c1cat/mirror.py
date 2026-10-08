"""A local, fast, read-only copy of the catalog.

The catalog lives on /mnt/c, where a single scan costs ~30 s: the file is 715 MB,
it has almost no useful indexes, and every page read pays 9p latency. Copying it
to a WSL-native disk takes ~6 s and drops the same scan to ~0.13 s.

So the mirror *is* the catalog, byte for byte, plus a denormalised ``v`` table,
an FTS5 index and an R*Tree. Keeping the original tables means anything we did
not think to denormalise is still reachable with the real schema.
"""

from __future__ import annotations

import fcntl
import os
import shutil
import sqlite3
import time
from contextlib import contextmanager
from pathlib import Path

from . import config, keywords, paths, source

MIRROR_SCHEMA = 3

DDL = """
CREATE TABLE c1cat_meta (k TEXT PRIMARY KEY, v TEXT);

CREATE TABLE v (
  variant_pk INTEGER PRIMARY KEY,
  variant_uuid TEXT, variant_index INTEGER, is_modified INTEGER,
  adj_layer INTEGER, def_layer INTEGER, comb_layer INTEGER,
  image_pk INTEGER, image_uuid TEXT,
  filename TEXT, filename_lc TEXT, display_name TEXT, ext TEXT,
  is_trashed INTEGER, is_inside_catalog INTEGER,
  path_win TEXT, path_wsl TEXT, folder_wsl TEXT,
  sidecar_path TEXT, thumb_path TEXT, preview_path TEXT,
  capture_ts REAL, capture_year INTEGER, capture_month TEXT, import_ts REAL,
  camera_make TEXT, camera_model TEXT, lens TEXT, iso INTEGER,
  aperture REAL, shutter REAL, focal REAL,
  width INTEGER, height INTEGER, file_size INTEGER,
  lat REAL, lon REAL,
  rating INTEGER, color INTEGER, color_name TEXT,
  keywords_raw TEXT, keywords_norm TEXT, keyword_count INTEGER,
  description TEXT, headline TEXT, title TEXT, creator TEXT, copyright TEXT,
  city TEXT, state TEXT, country TEXT, location TEXT, iso_country TEXT, date_created TEXT,
  style_names TEXT, n_local_layers INTEGER, n_retouch_layers INTEGER
);
CREATE INDEX v_rating   ON v(rating);
CREATE INDEX v_color    ON v(color);
CREATE INDEX v_capture  ON v(capture_ts);
CREATE INDEX v_import   ON v(import_ts);
CREATE INDEX v_camera   ON v(camera_model);
CREATE INDEX v_lens     ON v(lens);
CREATE INDEX v_iso      ON v(iso);
CREATE INDEX v_folder   ON v(folder_wsl);
CREATE INDEX v_fname    ON v(filename_lc);
CREATE INDEX v_image    ON v(image_pk);
CREATE INDEX v_modified ON v(is_modified);
CREATE INDEX v_year     ON v(capture_year);

CREATE TABLE coll (
  pk INTEGER PRIMARY KEY, ent INTEGER, kind TEXT, uuid TEXT, name TEXT,
  parent INTEGER, project INTEGER, idx INTEGER, path TEXT,
  sort_order TEXT, smart_xml TEXT, n_variants INTEGER
);
CREATE INDEX coll_name ON coll(name);
CREATE INDEX coll_parent ON coll(parent);

CREATE TABLE member (coll_pk INTEGER, variant_pk INTEGER, image_pk INTEGER,
                     PRIMARY KEY (coll_pk, variant_pk)) WITHOUT ROWID;
CREATE INDEX member_variant ON member(variant_pk);

CREATE TABLE kw (pk INTEGER PRIMARY KEY, name TEXT, parent INTEGER,
                 lft INTEGER, rgt INTEGER, path TEXT, depth INTEGER, n_variants INTEGER);
CREATE UNIQUE INDEX kw_path ON kw(path);

CREATE VIRTUAL TABLE vfts USING fts5(
  filename, keywords, description, headline, title,
  city, state, country, location, creator,
  content='', tokenize='unicode61 remove_diacritics 2'
);

CREATE VIRTUAL TABLE vgeo USING rtree(variant_pk, minlat, maxlat, minlon, maxlon);
"""

EXTRACT = """
SELECT v.Z_PK AS variant_pk, v.ZVARIANTUUID, v.ZINDEX, v.ZISMODIFIED,
       v.ZADJUSTMENTLAYER, v.ZDEFAULTLAYER, v.ZCOMBINEDSETTINGS,
       i.Z_PK AS image_pk, i.ZIMAGEUUID, i.ZIMAGEFILENAME, i.ZDISPLAYNAME, i.ZSIDECARPATH,
       i.ZISTRASHED, i.ZISINSIDECATALOG, i.ZEXP_DATE, i.ZIMPORTDATE,
       i.ZCAMERA_MAKE, i.ZCAMERA_MODEL, i.ZCAMERA_LENS, i.ZISO,
       i.ZEXP_APERTURE, i.ZEXP_SHUTTERSPEED, i.ZEXP_FOCALLENGTH,
       i.ZWIDTH, i.ZHEIGHT, i.ZFILE_SIZE, i.ZGPSLATITUDE, i.ZGPSLONGITUDE,
       p.ZWINROOT, p.ZRELATIVEPATH, p.ZISRELATIVE,
       m.ZBASIC_RATING, m.ZCOLOR_TAG_INDEX, m.ZCONTENT_KEYWORDS,
       m.ZCONTENT_DESCRIPTION, m.ZCONTENT_HEADLINE, m.ZSTATUS_TITLE,
       m.ZCONTACT_CREATOR, m.ZSTATUS_COPYRIGHTNOTICE,
       m.ZIMAGE_CITY, m.ZIMAGE_STATE, m.ZIMAGE_COUNTRY, m.ZIMAGE_LOCATION,
       m.ZIMAGE_ISOCOUNTRYCODE, m.ZIMAGE_DATECREATED
  FROM ZVARIANT v
  JOIN ZIMAGE i           ON i.Z_PK   = v.ZIMAGE
  JOIN ZPATHLOCATION p    ON p.Z_PK   = i.ZIMAGELOCATION
  JOIN ZVARIANTMETADATA m ON m.ZLAYER = v.ZADJUSTMENTLAYER
"""

KIND_BY_ENT = {
    config.ENT_ALBUM: "album",
    config.ENT_SMART: "smart",
    config.ENT_PROJECT: "project",
    config.ENT_GROUP: "group",
    config.ENT_TRASH: "trash",
    36: "folder",
    39: "in-catalog",
    40: "all-images",
}


class MirrorError(Exception):
    pass


@contextmanager
def _lock(catalog: Path):
    directory = config.cache_dir(catalog)
    directory.mkdir(parents=True, exist_ok=True)
    handle = open(directory / ".lock", "w")
    try:
        fcntl.flock(handle, fcntl.LOCK_EX)
        yield
    finally:
        fcntl.flock(handle, fcntl.LOCK_UN)
        handle.close()


def open_mirror(catalog: Path) -> sqlite3.Connection:
    path = config.mirror_path(catalog)
    if not path.exists():
        raise MirrorError(f"no local index yet - run 'c1cat sync' (expected {path})")
    con = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    con.row_factory = sqlite3.Row
    return con


def state(catalog: Path) -> dict:
    """Compare the mirror's recorded source fingerprint against the catalog."""
    path = config.mirror_path(catalog)
    stat = catalog.stat()
    result = {
        "path": str(path), "exists": path.exists(), "fresh": False,
        "source_mtime": stat.st_mtime, "source_size": stat.st_size,
        "built_at": None, "schema": None, "size": 0,
    }
    if not path.exists():
        return result
    result["size"] = path.stat().st_size
    try:
        con = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
        meta = dict(con.execute("SELECT k, v FROM c1cat_meta").fetchall())
        con.close()
    except sqlite3.Error:
        return result
    result["built_at"] = meta.get("built_at")
    result["schema"] = int(meta.get("mirror_schema", 0))
    result["fresh"] = (
        meta.get("source_path") == str(catalog)
        and meta.get("source_mtime") == repr(stat.st_mtime)
        and meta.get("source_size") == str(stat.st_size)
        and result["schema"] == MIRROR_SCHEMA
    )
    return result


def ensure_fresh(catalog: Path, *, allow_build: bool = True, progress=None) -> dict:
    current = state(catalog)
    if current["fresh"]:
        return current
    if not allow_build:
        if not current["exists"]:
            raise MirrorError("no local index and --no-sync was given - run 'c1cat sync'")
        current["stale"] = True
        return current
    return build(catalog, progress=progress)


def build(catalog: Path, progress=None) -> dict:
    """Copy the catalog locally, then add our own tables inside the copy."""
    def say(message: str) -> None:
        if progress:
            progress(message)

    started = time.time()
    with _lock(catalog):
        target = config.mirror_path(catalog)
        tmp = target.with_suffix(".tmp")
        tmp.unlink(missing_ok=True)

        stat = catalog.stat()
        say(f"copying catalog ({stat.st_size / 2**20:.0f} MB) to local disk")
        shutil.copyfile(catalog, tmp)
        copied = time.time()

        con = sqlite3.connect(str(tmp), isolation_level=None)
        con.row_factory = sqlite3.Row
        try:
            con.execute("PRAGMA journal_mode = OFF")
            con.execute("PRAGMA synchronous = OFF")
            con.executescript(DDL)
            con.execute("BEGIN")
            rows = _load_variants(con, catalog, say)
            _load_collections(con)
            _load_keywords(con)
            con.execute("COMMIT")
            say("analysing")
            con.execute("ANALYZE")
            con.executemany(
                "INSERT INTO c1cat_meta (k, v) VALUES (?, ?)",
                [
                    ("source_path", str(catalog)),
                    ("source_mtime", repr(stat.st_mtime)),
                    ("source_size", str(stat.st_size)),
                    ("built_at", time.strftime("%Y-%m-%dT%H:%M:%S")),
                    ("mirror_schema", str(MIRROR_SCHEMA)),
                    ("c1cat_version", config.VERSION),
                    ("variants", str(rows)),
                ],
            )
        finally:
            con.close()

        os.replace(tmp, target)

    say(f"indexed {rows} variants in {time.time() - started:.1f}s "
        f"(copy {copied - started:.1f}s)")
    return state(catalog)


def _load_variants(con: sqlite3.Connection, catalog: Path, say) -> int:
    directory = config.catalog_dir(catalog)

    styles: dict[int, list[str]] = {}
    local_counts: dict[int, int] = {}
    retouch_counts: dict[int, int] = {}
    for variant, ent, name in con.execute(
            "SELECT ZVARIANT, Z_ENT, ZNAME FROM ZVARIANTLAYER WHERE Z_ENT IN (?, ?)",
            (config.ENT_STYLE_LAYER, config.ENT_LOCAL_LAYER)):
        if ent == config.ENT_STYLE_LAYER:
            if name:
                styles.setdefault(variant, []).append(name)
        else:
            local_counts[variant] = local_counts.get(variant, 0) + 1
    for (variant,) in con.execute("SELECT ZVARIANT FROM ZRETOUCHINGLAYER"):
        retouch_counts[variant] = retouch_counts.get(variant, 0) + 1

    insert = (
        "INSERT INTO v (variant_pk, variant_uuid, variant_index, is_modified, adj_layer, def_layer,"
        " comb_layer, image_pk, image_uuid, filename, filename_lc, display_name, ext, is_trashed,"
        " is_inside_catalog, path_win, path_wsl, folder_wsl, sidecar_path, thumb_path, preview_path,"
        " capture_ts, capture_year, capture_month, import_ts, camera_make, camera_model, lens, iso,"
        " aperture, shutter, focal, width, height, file_size, lat, lon, rating, color, color_name,"
        " keywords_raw, keywords_norm, keyword_count, description, headline, title, creator,"
        " copyright, city, state, country, location, iso_country, date_created, style_names,"
        " n_local_layers, n_retouch_layers)"
        " VALUES (" + ",".join("?" * 57) + ")"
    )

    batch: list[tuple] = []
    fts: list[tuple] = []
    geo: list[tuple] = []
    total = 0

    cursor = con.execute(EXTRACT)
    while True:
        chunk = cursor.fetchmany(5000)
        if not chunk:
            break
        for r in chunk:
            filename = r["ZIMAGEFILENAME"] or ""
            ext = filename.rsplit(".", 1)[-1].lower() if "." in filename else ""
            win, wsl = paths.original_paths(
                r["ZWINROOT"] or "", r["ZRELATIVEPATH"] or "", filename,
                bool(r["ZISRELATIVE"]), directory)
            folder = wsl.rsplit("/", 1)[0] if wsl else None

            capture_ts = r["ZEXP_DATE"]
            year = month = None
            if capture_ts:
                tm = time.localtime(capture_ts)
                year, month = tm.tm_year, time.strftime("%Y-%m", tm)

            pairs = keywords.parse(r["ZCONTENT_KEYWORDS"])
            kw_paths = [p for p, _ in pairs]
            # \x1f-delimited so LIKE '%\x1fstreet\x1f%' matches whole tokens only.
            kw_norm = "\x1f" + "\x1f".join(sorted(kw_paths)) + "\x1f" if kw_paths else None

            rating = r["ZBASIC_RATING"]
            rating = None if rating in (None, "", 0) else int(rating)
            color = r["ZCOLOR_TAG_INDEX"]
            color = None if color in (None, "", 0) else int(color)

            lat = paths.parse_gps(r["ZGPSLATITUDE"])
            lon = paths.parse_gps(r["ZGPSLONGITUDE"])
            # Derived, not stat'd: existence is checked lazily when an image is opened.
            preview = paths.preview_path_unchecked(directory, r["ZSIDECARPATH"])
            vpk = r["variant_pk"]

            def blank(value):
                return value if value not in ("", None) else None

            batch.append((
                vpk, r["ZVARIANTUUID"], r["ZINDEX"], 1 if r["ZISMODIFIED"] else 0,
                r["ZADJUSTMENTLAYER"], r["ZDEFAULTLAYER"], r["ZCOMBINEDSETTINGS"],
                r["image_pk"], r["ZIMAGEUUID"], filename, filename.lower(),
                r["ZDISPLAYNAME"], ext, 1 if r["ZISTRASHED"] else 0,
                1 if r["ZISINSIDECATALOG"] else 0, win, wsl, folder,
                r["ZSIDECARPATH"], str(paths.thumbnail_path(directory, vpk)),
                str(preview) if preview else None,
                capture_ts, year, month, r["ZIMPORTDATE"],
                blank(r["ZCAMERA_MAKE"]), blank(r["ZCAMERA_MODEL"]), blank(r["ZCAMERA_LENS"]),
                r["ZISO"], r["ZEXP_APERTURE"], r["ZEXP_SHUTTERSPEED"], r["ZEXP_FOCALLENGTH"],
                r["ZWIDTH"], r["ZHEIGHT"], r["ZFILE_SIZE"], lat, lon,
                rating, color, config.COLOR_NAMES[color] if color else None,
                r["ZCONTENT_KEYWORDS"], kw_norm, len(kw_paths),
                blank(r["ZCONTENT_DESCRIPTION"]), blank(r["ZCONTENT_HEADLINE"]),
                blank(r["ZSTATUS_TITLE"]), blank(r["ZCONTACT_CREATOR"]),
                blank(r["ZSTATUS_COPYRIGHTNOTICE"]), blank(r["ZIMAGE_CITY"]),
                blank(r["ZIMAGE_STATE"]), blank(r["ZIMAGE_COUNTRY"]), blank(r["ZIMAGE_LOCATION"]),
                blank(r["ZIMAGE_ISOCOUNTRYCODE"]), blank(r["ZIMAGE_DATECREATED"]),
                ";".join(styles.get(vpk, [])) or None,
                local_counts.get(vpk, 0), retouch_counts.get(vpk, 0),
            ))
            fts.append((
                vpk, filename, " ".join(p.replace("|", " ") for p in kw_paths),
                r["ZCONTENT_DESCRIPTION"] or "", r["ZCONTENT_HEADLINE"] or "",
                r["ZSTATUS_TITLE"] or "", r["ZIMAGE_CITY"] or "", r["ZIMAGE_STATE"] or "",
                r["ZIMAGE_COUNTRY"] or "", r["ZIMAGE_LOCATION"] or "", r["ZCONTACT_CREATOR"] or "",
            ))
            if lat is not None and lon is not None:
                geo.append((vpk, lat, lat, lon, lon))

        con.executemany(insert, batch)
        con.executemany(
            "INSERT INTO vfts (rowid, filename, keywords, description, headline, title,"
            " city, state, country, location, creator) VALUES (?,?,?,?,?,?,?,?,?,?,?)", fts)
        if geo:
            con.executemany("INSERT INTO vgeo VALUES (?,?,?,?,?)", geo)
        total += len(batch)
        batch, fts, geo = [], [], []
        say(f"  indexed {total} variants")

    return total


def _load_collections(con: sqlite3.Connection) -> None:
    rows = con.execute(
        "SELECT Z_PK, Z_ENT, ZUUID, ZNAME, ZPARENT, ZPROJECT, ZCOLLECTIONINDEX,"
        " ZSORTORDER, ZSMARTCRITERIA FROM ZCOLLECTION").fetchall()
    by_pk = {r["Z_PK"]: r for r in rows}

    def path_of(pk: int, seen: frozenset = frozenset()) -> str:
        row = by_pk.get(pk)
        if row is None or pk in seen:
            return ""
        name = row["ZNAME"] or ""
        parent = row["ZPARENT"]
        if parent in (None, -1, config.ROOT_COLLECTION_PK) or parent not in by_pk:
            return name
        prefix = path_of(parent, seen | {pk})
        return f"{prefix} / {name}" if prefix else name

    counts = dict(con.execute(
        "SELECT ZCOLLECTION, COUNT(*) FROM ZVARIANTINCOLLECTION GROUP BY ZCOLLECTION").fetchall())

    con.executemany(
        "INSERT INTO coll (pk, ent, kind, uuid, name, parent, project, idx, path,"
        " sort_order, smart_xml, n_variants) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        [(r["Z_PK"], r["Z_ENT"], KIND_BY_ENT.get(r["Z_ENT"], f"ent{r['Z_ENT']}"),
          r["ZUUID"], r["ZNAME"], r["ZPARENT"], r["ZPROJECT"], r["ZCOLLECTIONINDEX"],
          path_of(r["Z_PK"]), r["ZSORTORDER"], r["ZSMARTCRITERIA"],
          counts.get(r["Z_PK"], 0)) for r in rows])

    con.execute(
        "INSERT OR IGNORE INTO member (coll_pk, variant_pk, image_pk) "
        "SELECT vc.ZCOLLECTION, vc.ZVARIANT, v.ZIMAGE FROM ZVARIANTINCOLLECTION vc "
        "JOIN ZVARIANT v ON v.Z_PK = vc.ZVARIANT")


def _load_keywords(con: sqlite3.Connection) -> None:
    tree = keywords.load_tree(con)
    usage: dict[str, int] = {}
    for (serialised,) in con.execute(
            "SELECT keywords_raw FROM v WHERE keywords_raw IS NOT NULL AND keywords_raw <> ''"):
        for path, _ in keywords.parse(serialised):
            usage[path] = usage.get(path, 0) + 1
    con.executemany(
        "INSERT INTO kw (pk, name, parent, lft, rgt, path, depth, n_variants) VALUES (?,?,?,?,?,?,?,?)",
        [(n.pk, n.name, n.parent, n.left, n.right, n.path, n.path.count(keywords.SEP),
          usage.get(n.path, 0)) for n in tree.values()])
