"""Building ZCOLLECTION rows that Capture One accepts.

The default criteria blobs are copied byte for byte from a collection Capture One
itself created (CRLF line endings included).
"""

from __future__ import annotations

import time
import uuid
import xml.etree.ElementTree as ET

from . import config

DEFAULT_SEARCH = (
    '<?xml version="1.0" encoding="utf-8"?>\r\n'
    '<MatchOperator Kind="AND">\r\n'
    '  <MatchOperator Kind="OR">\r\n'
    '    <Condition Enabled="NO">\r\n'
    '      <Key>ANY</Key>\r\n'
    '      <Operator>6</Operator>\r\n'
    '      <Criterion></Criterion>\r\n'
    '    </Condition>\r\n'
    '  </MatchOperator>\r\n'
    '</MatchOperator>'
)

DEFAULT_FILTER = '<?xml version="1.0" encoding="utf-8"?>\r\n<MatchOperator Kind="AND" />'

# Operator codes, inferred from smart albums in this catalog whose names state their
# intent ("street 4+" uses op 2 with criterion 4; "street4+ 2025" uses op 5 with 3).
OP_EQUALS = 0
OP_AT_LEAST = 2
OP_GREATER = 5
OP_CONTAINS = 6

KEY_RATING = "IB_I_BASIC_RATING"
KEY_KEYWORDS = "IB_S_CONTENT_KEYWORDS"
KEY_DATE = "IB_S_EXP_DATE"

OPERATOR_NAMES = {
    OP_EQUALS: "is", OP_AT_LEAST: "is at least", OP_GREATER: "is greater than",
    OP_CONTAINS: "contains",
}


class CollectionError(Exception):
    pass


def _condition(key: str, operator: int, criterion: str, enabled: bool = True) -> str:
    state = "YES" if enabled else "NO"
    return (
        '  <MatchOperator Kind="OR">\r\n'
        f'    <Condition Enabled="{state}">\r\n'
        f'      <Key>{key}</Key>\r\n'
        f'      <Operator>{operator}</Operator>\r\n'
        f'      <Criterion>{_escape(criterion)}</Criterion>\r\n'
        '    </Condition>\r\n'
        '  </MatchOperator>\r\n'
    )


def _escape(text: str) -> str:
    return (text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;"))


def smart_criteria(conditions: list[tuple[str, int, str]], match_all: bool = True) -> str:
    """Serialise smart-album criteria in Capture One's own layout.

    The first inner block is always the disabled ANY placeholder - every smart album
    in the catalog has it, so we reproduce it rather than guess that it is optional.
    """
    kind = "AND" if match_all else "OR"
    body = _condition("ANY", OP_CONTAINS, "", enabled=False)
    for key, operator, criterion in conditions:
        body += _condition(key, operator, criterion)
    return (
        '<?xml version="1.0" encoding="utf-8"?>\r\n'
        f'<MatchOperator Kind="{kind}">\r\n'
        f'{body}'
        '</MatchOperator>'
    )


def parse_criteria(xml: str) -> list[dict]:
    """Read a ZSMARTCRITERIA blob back into readable conditions."""
    if not xml:
        return []
    root = ET.fromstring(xml)
    out = []
    for condition in root.iter("Condition"):
        key = condition.findtext("Key") or ""
        operator = int(condition.findtext("Operator") or 0)
        out.append({
            "key": key,
            "operator": operator,
            "operator_name": OPERATOR_NAMES.get(operator, f"op{operator}"),
            "criterion": condition.findtext("Criterion") or "",
            "enabled": condition.get("Enabled") == "YES",
        })
    return out


def rating_condition(expression: str) -> tuple[str, int, str]:
    """'>=4' -> at-least 4; '>4' -> greater-than 4; '4' or '=4' -> exactly 4."""
    text = expression.strip()
    if text.startswith(">="):
        return KEY_RATING, OP_AT_LEAST, text[2:].strip()
    if text.startswith(">"):
        return KEY_RATING, OP_GREATER, text[1:].strip()
    if text.startswith("="):
        return KEY_RATING, OP_EQUALS, text[1:].strip()
    if text.isdigit():
        return KEY_RATING, OP_EQUALS, text
    raise CollectionError(f"cannot express rating {expression!r} as smart-album criteria; "
                          "use '>=4', '>3' or '4'")


def create(con, name: str, ent: int, parent_pk: int, *, sort_order: str = "date",
           smart_xml: str | None = None) -> int:
    """Insert a collection row. Z_PK is a rowid alias, so SQLite assigns it."""
    parent = con.execute("SELECT Z_PK, ZPROJECT, Z_ENT FROM ZCOLLECTION WHERE Z_PK = ?",
                         (parent_pk,)).fetchone()
    if parent is None:
        raise CollectionError(f"parent collection {parent_pk} does not exist")
    project = parent["ZPROJECT"] if parent["ZPROJECT"] not in (None, -1) else config.ROOT_COLLECTION_PK
    if parent["Z_ENT"] == config.ENT_PROJECT:
        project = parent_pk

    index = con.execute(
        "SELECT COALESCE(MAX(ZCOLLECTIONINDEX), -1) + 1 FROM ZCOLLECTION WHERE ZPARENT = ?",
        (parent_pk,)).fetchone()[0]
    now = time.time()
    con.execute(
        "INSERT INTO ZCOLLECTION (Z_ENT, ZUUID, ZNAME, ZPARENT, ZPROJECT, ZDOCUMENT,"
        " ZCOLLECTIONINDEX, ZCOLLAPSED, ZSORTORDER, ZDATECREATED, ZDATEMODIFIED,"
        " ZCONTENTMODIFIEDDATE, ZSERVEDBYCAPTUREPILOT, ZCAPTURECOLLECTIONOPTIONS,"
        " ZPROCESSCOLLECTIONOPTIONS, ZSEARCHCRITERIA, ZFILTERCRITERIA, ZSMARTCRITERIA)"
        " VALUES (?, ?, ?, ?, ?, 1, ?, 1, ?, ?, ?, ?, 0, 0, 0, ?, ?, ?)",
        (ent, str(uuid.uuid4()), name, parent_pk, project, index, sort_order,
         now, now, now, DEFAULT_SEARCH, DEFAULT_FILTER, smart_xml))
    return con.execute("SELECT last_insert_rowid()").fetchone()[0]


def touch(con, collection_pk: int) -> None:
    now = time.time()
    con.execute(
        "UPDATE ZCOLLECTION SET ZDATEMODIFIED = ?, ZCONTENTMODIFIEDDATE = ? WHERE Z_PK = ?",
        (now, now, collection_pk))


def add_members(con, collection_pk: int, variant_pks: list[int]) -> tuple[int, int]:
    """Link variants (and their images) into a collection. Returns (images, variants) added."""
    before_i = con.execute("SELECT COUNT(*) FROM ZIMAGEINCOLLECTION WHERE ZCOLLECTION = ?",
                           (collection_pk,)).fetchone()[0]
    before_v = con.execute("SELECT COUNT(*) FROM ZVARIANTINCOLLECTION WHERE ZCOLLECTION = ?",
                           (collection_pk,)).fetchone()[0]
    for pk in variant_pks:
        row = con.execute("SELECT ZIMAGE FROM ZVARIANT WHERE Z_PK = ?", (pk,)).fetchone()
        if row is None:
            continue
        con.execute("INSERT OR IGNORE INTO ZIMAGEINCOLLECTION (Z_ENT, ZCOLLECTION, ZIMAGE)"
                    " VALUES (?, ?, ?)", (config.ENT_IMAGE_IN_COLLECTION, collection_pk, row[0]))
        con.execute("INSERT OR IGNORE INTO ZVARIANTINCOLLECTION (Z_ENT, ZCOLLECTION, ZVARIANT)"
                    " VALUES (?, ?, ?)", (config.ENT_VARIANT_IN_COLLECTION, collection_pk, pk))
    touch(con, collection_pk)
    after_i = con.execute("SELECT COUNT(*) FROM ZIMAGEINCOLLECTION WHERE ZCOLLECTION = ?",
                          (collection_pk,)).fetchone()[0]
    after_v = con.execute("SELECT COUNT(*) FROM ZVARIANTINCOLLECTION WHERE ZCOLLECTION = ?",
                          (collection_pk,)).fetchone()[0]
    return after_i - before_i, after_v - before_v


def remove_members(con, collection_pk: int, variant_pks: list[int]) -> tuple[int, int]:
    """Unlink variants. An image link only goes when its last variant leaves."""
    images = variants = 0
    for pk in variant_pks:
        row = con.execute("SELECT ZIMAGE FROM ZVARIANT WHERE Z_PK = ?", (pk,)).fetchone()
        if row is None:
            continue
        image_pk = row[0]
        cur = con.execute("DELETE FROM ZVARIANTINCOLLECTION WHERE ZCOLLECTION = ? AND ZVARIANT = ?",
                          (collection_pk, pk))
        variants += cur.rowcount
        remaining = con.execute(
            "SELECT COUNT(*) FROM ZVARIANTINCOLLECTION vc JOIN ZVARIANT v ON v.Z_PK = vc.ZVARIANT"
            " WHERE vc.ZCOLLECTION = ? AND v.ZIMAGE = ?", (collection_pk, image_pk)).fetchone()[0]
        if remaining == 0:
            cur = con.execute("DELETE FROM ZIMAGEINCOLLECTION WHERE ZCOLLECTION = ? AND ZIMAGE = ?",
                              (collection_pk, image_pk))
            images += cur.rowcount
            con.execute("DELETE FROM ZIMAGEINCOLLECTIONPROPERTIES WHERE ZCOLLECTION = ? AND ZIMAGE = ?",
                        (collection_pk, image_pk))
    touch(con, collection_pk)
    return images, variants


def delete(con, collection_pk: int) -> None:
    for table in ("ZIMAGEINCOLLECTION", "ZVARIANTINCOLLECTION", "ZIMAGEINCOLLECTIONPROPERTIES"):
        con.execute(f"DELETE FROM {table} WHERE ZCOLLECTION = ?", (collection_pk,))
    con.execute("UPDATE ZCOLLECTION SET ZPRIMARYVARIANT = NULL WHERE ZPRIMARYVARIANT = ?",
                (collection_pk,))
    con.execute("DELETE FROM ZCOLLECTION WHERE Z_PK = ?", (collection_pk,))


def dangling_links(con, collection_pk: int | None = None) -> tuple[int, int]:
    """Count (variant links with no image link, image links with no variant link).

    This is deliberately a *measurement*, not an invariant. Capture One's own catalogs
    carry plenty of both: projects hold variant links with no image links at all, and
    albums keep an image link after its last variant link is removed. So the check that
    matters is whether an operation increased these counts, not whether they are zero.
    Pass ``collection_pk`` to scope it to the collection being modified.
    """
    scope = "" if collection_pk is None else " AND vc.ZCOLLECTION = :pk"
    orphan_variants = con.execute(
        "SELECT COUNT(*) FROM ZVARIANTINCOLLECTION vc JOIN ZVARIANT v ON v.Z_PK = vc.ZVARIANT"
        " WHERE NOT EXISTS (SELECT 1 FROM ZIMAGEINCOLLECTION ic"
        "                   WHERE ic.ZCOLLECTION = vc.ZCOLLECTION AND ic.ZIMAGE = v.ZIMAGE)"
        + scope, {"pk": collection_pk}).fetchone()[0]
    scope = "" if collection_pk is None else " AND ic.ZCOLLECTION = :pk"
    orphan_images = con.execute(
        "SELECT COUNT(*) FROM ZIMAGEINCOLLECTION ic"
        " WHERE NOT EXISTS (SELECT 1 FROM ZVARIANTINCOLLECTION vc JOIN ZVARIANT v ON v.Z_PK = vc.ZVARIANT"
        "                   WHERE vc.ZCOLLECTION = ic.ZCOLLECTION AND v.ZIMAGE = ic.ZIMAGE)"
        + scope, {"pk": collection_pk}).fetchone()[0]
    return orphan_variants, orphan_images


def check_no_worse(before: tuple[int, int], after: tuple[int, int]) -> list[str]:
    problems = []
    if after[0] > before[0]:
        problems.append(f"{after[0] - before[0]} new variant link(s) without a matching image link")
    if after[1] > before[1]:
        problems.append(f"{after[1] - before[1]} new image link(s) without a matching variant link")
    return problems
