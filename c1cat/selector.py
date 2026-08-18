"""The selector: one flag group, shared by every command that acts on a set of variants."""

from __future__ import annotations

import json
import re
import time
from dataclasses import dataclass
from pathlib import Path

from . import config

# Descending sorts are spelled "date-desc" rather than "-date": argparse would read a
# leading dash as the start of another flag, so "--sort -date" could never work.
SORTS = {
    "date": "capture_ts ASC", "date-desc": "capture_ts DESC",
    "rating": "rating ASC", "rating-desc": "rating DESC",
    "filename": "filename_lc ASC", "filename-desc": "filename_lc DESC",
    "imported": "import_ts ASC", "imported-desc": "import_ts DESC",
    "random": "RANDOM()",
    "id": "variant_pk ASC", "id-desc": "variant_pk DESC",
}
# Accepted with --sort=-date, kept because the shorthand is natural to type.
SORT_ALIASES = {f"-{name}": f"{name}-desc" for name in
                ("date", "rating", "filename", "imported", "id")}

_COMPARISON = re.compile(r"^(>=|<=|>|<|=)?\s*(\d+)$")
_RANGE = re.compile(r"^(\d+(?:\.\d+)?)\s*-\s*(\d+(?:\.\d+)?)$")


class SelectorError(Exception):
    pass


def add_arguments(parser) -> None:
    g = parser.add_argument_group("selection")
    g.add_argument("--id", type=int, action="append", metavar="PK", help="variant primary key (repeatable)")
    g.add_argument("--image-id", type=int, action="append", metavar="PK", help="image primary key (repeatable)")
    g.add_argument("--uuid", action="append", help="variant UUID (repeatable)")
    g.add_argument("--ids-file", help="file with one variant PK per line")
    g.add_argument("--from-selection", metavar="FILE", help="a selection saved by --save-selection")
    g.add_argument("--filename", help="filename glob, e.g. 'DSCF*.RAF'")
    g.add_argument("--ext", help="comma-separated extensions, e.g. raf,dng")
    g.add_argument("--folder", help="folder path prefix (WSL or Windows form)")
    g.add_argument("--rating", help="'>=4', '3', '2-4', 'none' or 'any'")
    g.add_argument("--color", "--colour", dest="color", help="colour tag name(s), 'none' or 'any'")
    g.add_argument("--keyword", action="append", help="keyword path; repeat to require all")
    g.add_argument("--keyword-any", help="comma-separated keywords, match any")
    g.add_argument("--keyword-under", action="append", metavar="PATH", help="keyword and everything below it")
    g.add_argument("--no-keyword", action="append", metavar="PATH", help="exclude this keyword")
    g.add_argument("--unkeyworded", action="store_true", help="only variants with no keywords")
    g.add_argument("--camera", help="camera model, substring match")
    g.add_argument("--lens", help="lens name, substring match")
    g.add_argument("--iso", help="e.g. '400', '100-1600', '>=800'")
    g.add_argument("--aperture", help="e.g. '<=2.8', '1.4-2.8'")
    g.add_argument("--focal", help="focal length in mm, e.g. '23', '16-55'")
    g.add_argument("--from", dest="date_from", metavar="DATE", help="capture date from (YYYY-MM-DD)")
    g.add_argument("--to", dest="date_to", metavar="DATE", help="capture date to (inclusive)")
    g.add_argument("--year", type=int, help="capture year")
    g.add_argument("--month", help="capture month, YYYY-MM")
    g.add_argument("--imported-from", metavar="DATE")
    g.add_argument("--imported-to", metavar="DATE")
    g.add_argument("--album", action="append", help="album name or #PK (repeatable)")
    g.add_argument("--not-in-album", action="append", metavar="NAME")
    g.add_argument("--gps", choices=["has", "none"])
    g.add_argument("--near", metavar="LAT,LON,KM", help="within KM of a point")
    g.add_argument("--city")
    g.add_argument("--country")
    g.add_argument("--state")
    g.add_argument("--text", help="free-text search over keywords, caption and IPTC")
    g.add_argument("--edited", action="store_true", help="develop settings were changed")
    g.add_argument("--unedited", action="store_true")
    g.add_argument("--style", help="applied style name, substring match")
    g.add_argument("--has-local-layers", action="store_true")
    g.add_argument("--include-trashed", action="store_true")
    g.add_argument("--only-trashed", action="store_true")
    g.add_argument("--variant-index", type=int, help="only this variant position (0 is the original)")
    g.add_argument("--sort", default="date-desc", choices=sorted(SORTS) + sorted(SORT_ALIASES),
                   metavar="ORDER", help="one of: " + ", ".join(sorted(SORTS)))
    g.add_argument("--limit", type=int)
    g.add_argument("--offset", type=int)
    g.add_argument("--save-selection", metavar="FILE")
    g.add_argument("--dump-sql", action="store_true", help="print the compiled query")


SELECTOR_FLAGS = (
    "id", "image_id", "uuid", "ids_file", "from_selection", "filename", "ext", "folder",
    "rating", "color", "keyword", "keyword_any", "keyword_under", "no_keyword", "unkeyworded",
    "camera", "lens", "iso", "aperture", "focal", "date_from", "date_to", "year", "month",
    "imported_from", "imported_to", "album", "not_in_album", "gps", "near", "city", "country",
    "state", "text", "edited", "unedited", "style", "has_local_layers", "only_trashed",
    "variant_index",
)


def is_empty(args) -> bool:
    """True when nothing narrows the selection - mutating commands refuse this."""
    return not any(getattr(args, name, None) for name in SELECTOR_FLAGS)


def _numeric(clause: str, expr: str, params: list) -> str:
    """Compile '>=4' / '2-4' / '3' against a column."""
    clause = clause.strip()
    m = _RANGE.match(clause)
    if m:
        params.extend([float(m.group(1)), float(m.group(2))])
        return f"{expr} BETWEEN ? AND ?"
    m = _COMPARISON.match(clause)
    if m:
        op = m.group(1) or "="
        params.append(float(m.group(2)))
        return f"{expr} {op} ?"
    try:
        params.append(float(clause))
    except ValueError as exc:
        raise SelectorError(f"cannot parse numeric filter {clause!r}") from exc
    return f"{expr} = ?"


def _epoch(text: str, end_of_day: bool = False) -> float:
    for fmt in ("%Y-%m-%d", "%Y-%m", "%Y"):
        try:
            tm = time.strptime(text, fmt)
        except ValueError:
            continue
        stamp = time.mktime(tm)
        if end_of_day:
            if fmt == "%Y-%m-%d":
                stamp += 86400 - 1
            elif fmt == "%Y-%m":
                stamp = time.mktime((tm.tm_year + (tm.tm_mon == 12), tm.tm_mon % 12 + 1, 1,
                                     0, 0, 0, 0, 1, -1)) - 1
            else:
                stamp = time.mktime((tm.tm_year + 1, 1, 1, 0, 0, 0, 0, 1, -1)) - 1
        return stamp
    raise SelectorError(f"cannot parse date {text!r}, use YYYY-MM-DD")


@dataclass
class Query:
    sql: str
    params: list
    described: dict


def build(args, con) -> Query:
    where: list[str] = []
    params: list = []
    described: dict = {}

    def note(key, value):
        if value not in (None, False, [], ""):
            described[key] = value

    pks: set[int] = set()
    if args.id:
        pks.update(args.id)
    if args.ids_file:
        pks.update(int(line) for line in Path(args.ids_file).read_text().split() if line.strip())
    if args.from_selection:
        payload = json.loads(Path(args.from_selection).read_text())
        pks.update(payload["variant_pks"])
    if pks:
        where.append(f"variant_pk IN ({','.join('?' * len(pks))})")
        params.extend(sorted(pks))
        note("ids", len(pks))
    if args.image_id:
        where.append(f"image_pk IN ({','.join('?' * len(args.image_id))})")
        params.extend(args.image_id)
        note("image_ids", args.image_id)
    if args.uuid:
        where.append(f"variant_uuid IN ({','.join('?' * len(args.uuid))})")
        params.extend(args.uuid)
        note("uuid", args.uuid)

    if args.filename:
        where.append("filename_lc GLOB ?")
        params.append(args.filename.lower())
        note("filename", args.filename)
    if args.ext:
        exts = [e.strip().lower().lstrip(".") for e in args.ext.split(",") if e.strip()]
        where.append(f"ext IN ({','.join('?' * len(exts))})")
        params.extend(exts)
        note("ext", exts)
    if args.folder:
        folder = args.folder.replace("\\", "/").rstrip("/")
        if re.match(r"^[A-Za-z]:", folder):
            folder = f"/mnt/{folder[0].lower()}/{folder[3:]}"
        where.append("(folder_wsl = ? OR folder_wsl LIKE ?)")
        params.extend([folder, folder + "/%"])
        note("folder", args.folder)

    if args.rating:
        value = args.rating.strip().lower()
        if value == "none":
            where.append("rating IS NULL")
        elif value == "any":
            where.append("rating IS NOT NULL")
        else:
            where.append(_numeric(value, "rating", params))
        note("rating", args.rating)
    if args.color:
        value = args.color.strip().lower()
        if value == "none":
            where.append("color IS NULL")
        elif value == "any":
            where.append("color IS NOT NULL")
        else:
            names = [n.strip() for n in value.split(",") if n.strip()]
            unknown = [n for n in names if n not in config.COLOR_INDEX]
            if unknown:
                raise SelectorError(f"unknown colour tag(s): {', '.join(unknown)}; "
                                    f"choose from {', '.join(config.COLOR_NAMES[1:])}")
            where.append(f"color IN ({','.join('?' * len(names))})")
            params.extend(config.COLOR_INDEX[n] for n in names)
        note("color", args.color)

    for path in args.keyword or []:
        where.append("keywords_norm LIKE ?")
        params.append(f"%\x1f{path}\x1f%")
    note("keyword", args.keyword)
    if args.keyword_any:
        wanted = [k.strip() for k in args.keyword_any.split(",") if k.strip()]
        where.append("(" + " OR ".join("keywords_norm LIKE ?" for _ in wanted) + ")")
        params.extend(f"%\x1f{k}\x1f%" for k in wanted)
        note("keyword_any", wanted)
    for path in args.keyword_under or []:
        where.append("(keywords_norm LIKE ? OR keywords_norm LIKE ?)")
        params.extend([f"%\x1f{path}\x1f%", f"%\x1f{path}|%"])
    note("keyword_under", args.keyword_under)
    for path in args.no_keyword or []:
        where.append("(keywords_norm IS NULL OR keywords_norm NOT LIKE ?)")
        params.append(f"%\x1f{path}\x1f%")
    note("no_keyword", args.no_keyword)
    if args.unkeyworded:
        where.append("keyword_count = 0")
        note("unkeyworded", True)

    if args.camera:
        where.append("camera_model LIKE ?")
        params.append(f"%{args.camera}%")
        note("camera", args.camera)
    if args.lens:
        where.append("lens LIKE ?")
        params.append(f"%{args.lens}%")
        note("lens", args.lens)
    if args.iso:
        where.append(_numeric(args.iso, "iso", params))
        note("iso", args.iso)
    if args.aperture:
        where.append(_numeric(args.aperture, "aperture", params))
        note("aperture", args.aperture)
    if args.focal:
        where.append(_numeric(args.focal, "focal", params))
        note("focal", args.focal)

    if args.date_from:
        where.append("capture_ts >= ?")
        params.append(_epoch(args.date_from))
        note("from", args.date_from)
    if args.date_to:
        where.append("capture_ts <= ?")
        params.append(_epoch(args.date_to, end_of_day=True))
        note("to", args.date_to)
    if args.year:
        where.append("capture_year = ?")
        params.append(args.year)
        note("year", args.year)
    if args.month:
        where.append("capture_month = ?")
        params.append(args.month)
        note("month", args.month)
    if args.imported_from:
        where.append("import_ts >= ?")
        params.append(_epoch(args.imported_from))
        note("imported_from", args.imported_from)
    if args.imported_to:
        where.append("import_ts <= ?")
        params.append(_epoch(args.imported_to, end_of_day=True))
        note("imported_to", args.imported_to)

    for name in args.album or []:
        pk = resolve_collection(con, name)
        where.append("variant_pk IN (SELECT variant_pk FROM member WHERE coll_pk = ?)")
        params.append(pk)
    note("album", args.album)
    for name in args.not_in_album or []:
        pk = resolve_collection(con, name)
        where.append("variant_pk NOT IN (SELECT variant_pk FROM member WHERE coll_pk = ?)")
        params.append(pk)
    note("not_in_album", args.not_in_album)

    if args.gps == "has":
        where.append("lat IS NOT NULL")
        note("gps", "has")
    elif args.gps == "none":
        where.append("lat IS NULL")
        note("gps", "none")
    if args.near:
        try:
            lat, lon, km = (float(x) for x in args.near.split(","))
        except ValueError as exc:
            raise SelectorError("--near expects LAT,LON,KM") from exc
        dlat = km / 111.0
        dlon = km / max(1e-6, 111.0 * abs(__import__("math").cos(__import__("math").radians(lat))))
        where.append("variant_pk IN (SELECT variant_pk FROM vgeo WHERE maxlat >= ? AND minlat <= ?"
                     " AND maxlon >= ? AND minlon <= ?)")
        params.extend([lat - dlat, lat + dlat, lon - dlon, lon + dlon])
        note("near", args.near)
    for field, value in (("city", args.city), ("country", args.country), ("state", args.state)):
        if value:
            where.append(f"{field} LIKE ?")
            params.append(f"%{value}%")
            note(field, value)

    if args.text:
        where.append("variant_pk IN (SELECT rowid FROM vfts WHERE vfts MATCH ?)")
        params.append(args.text)
        note("text", args.text)

    if args.edited:
        where.append("is_modified = 1")
        note("edited", True)
    if args.unedited:
        where.append("is_modified = 0")
        note("unedited", True)
    if args.style:
        where.append("style_names LIKE ?")
        params.append(f"%{args.style}%")
        note("style", args.style)
    if args.has_local_layers:
        where.append("n_local_layers > 0")
        note("has_local_layers", True)

    if args.only_trashed:
        where.append("is_trashed = 1")
        note("trashed", "only")
    elif not args.include_trashed:
        where.append("is_trashed = 0")
    else:
        note("trashed", "included")

    if args.variant_index is not None:
        where.append("variant_index = ?")
        params.append(args.variant_index)
        note("variant_index", args.variant_index)

    sql = "SELECT * FROM v"
    if where:
        sql += "\n WHERE " + "\n   AND ".join(where)
    sql += f"\n ORDER BY {SORTS[SORT_ALIASES.get(args.sort, args.sort)]}"
    if args.limit:
        sql += f"\n LIMIT {int(args.limit)}"
        if args.offset:
            sql += f" OFFSET {int(args.offset)}"
    elif args.offset:
        sql += f"\n LIMIT -1 OFFSET {int(args.offset)}"

    return Query(sql, params, described)


def resolve_collection(con, name: str) -> int:
    """Accept '#875', an exact name, a path suffix, or a unique case-insensitive match."""
    if name.startswith("#"):
        return int(name[1:])
    rows = con.execute("SELECT pk, name, path, kind FROM coll WHERE name = ?", (name,)).fetchall()
    if not rows:
        rows = con.execute(
            "SELECT pk, name, path, kind FROM coll WHERE name = ? COLLATE NOCASE", (name,)).fetchall()
    if not rows:
        rows = con.execute(
            "SELECT pk, name, path, kind FROM coll WHERE path LIKE ? COLLATE NOCASE",
            (f"%{name}",)).fetchall()
    if not rows:
        raise SelectorError(f"no collection named {name!r}")
    if len(rows) > 1:
        listing = "\n  ".join(f"#{r['pk']}  {r['path']} ({r['kind']})" for r in rows)
        raise SelectorError(f"{name!r} matches several collections, use #PK:\n  {listing}")
    return rows[0]["pk"]


def run(args, con) -> tuple[list, Query]:
    query = build(args, con)
    rows = con.execute(query.sql, query.params).fetchall()
    if args.save_selection:
        Path(args.save_selection).write_text(json.dumps({
            "built_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "query": query.described,
            "count": len(rows),
            "variant_pks": [r["variant_pk"] for r in rows],
        }, indent=2, ensure_ascii=False) + "\n")
    return rows, query


def count(args, con) -> int:
    query = build(args, con)
    inner = query.sql.split("\n ORDER BY")[0]
    return con.execute(f"SELECT COUNT(*) FROM ({inner})", query.params).fetchone()[0]
