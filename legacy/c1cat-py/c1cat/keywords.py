"""Keyword serialisation and ZKEYWORD nested-set maintenance.

ZVARIANTMETADATA.ZCONTENT_KEYWORDS is a single comma-separated string of
``<path>||<index>`` tokens, where ``path`` uses ``|`` to separate hierarchy
levels.  The same variant stores the same set twice, in two different orders:

    adjustment layer (Z_ENT 17)  sorted by path
    combined layer   (Z_ENT 20)  sorted by index

ZKEYWORD is a nested set: ZLEFT/ZRIGHT are both UNIQUE, there is a row-level
CHECK(ZLEFT < ZRIGHT), and in a healthy catalog the union of all ZLEFT and
ZRIGHT values is exactly 1..2n.
"""

from __future__ import annotations

from dataclasses import dataclass

SEP = "|"      # separates hierarchy levels inside a path
TOK = "||"     # separates the path from its index


class KeywordError(Exception):
    """Raised for keyword names or paths that would corrupt the catalog."""


@dataclass(frozen=True)
class Node:
    pk: int
    name: str
    parent: int | None
    left: int
    right: int
    path: str


def parse(serialised: str | None) -> list[tuple[str, int]]:
    """'street||0,best|best2018||1' -> [('street', 0), ('best|best2018', 1)]."""
    if not serialised:
        return []
    out: list[tuple[str, int]] = []
    for token in serialised.split(","):
        if not token:
            continue
        path, sep, index = token.rpartition(TOK)
        if not sep:
            raise KeywordError(f"malformed keyword token: {token!r}")
        out.append((path, int(index)))
    return out


def renumber(pairs: list[tuple[str, int]]) -> list[tuple[str, int]]:
    """Force indices to a dense 0..n-1 while preserving the existing order."""
    ordered = sorted(pairs, key=lambda p: p[1])
    return [(path, i) for i, (path, _) in enumerate(ordered)]


def serialise(pairs: list[tuple[str, int]], order: str) -> str | None:
    """Render for one layer. ``order`` is 'path' (layer 17) or 'index' (layer 20).

    Path ordering is a plain byte comparison, which is what Capture One does:
    'b&w' sorts before 'best' because '&' (0x26) < 'e' (0x65).
    """
    if not pairs:
        return None
    pairs = renumber(pairs)
    if order == "path":
        pairs = sorted(pairs, key=lambda p: p[0].encode())
    elif order == "index":
        pairs = sorted(pairs, key=lambda p: p[1])
    else:
        raise ValueError(f"unknown order {order!r}")
    return ",".join(f"{path}{TOK}{index}" for path, index in pairs)


def validate_name(name: str) -> str:
    """A keyword name containing ',' or '|' would corrupt every serialised string."""
    name = name.strip()
    if not name:
        raise KeywordError("keyword name is empty")
    if "," in name or SEP in name:
        raise KeywordError(f"keyword name may not contain ',' or '|': {name!r}")
    return name


def split_path(path: str) -> list[str]:
    segments = [validate_name(s) for s in path.split(SEP)]
    if not segments:
        raise KeywordError(f"empty keyword path: {path!r}")
    return segments


def add_paths(pairs: list[tuple[str, int]], paths: list[str]) -> list[tuple[str, int]]:
    """Append paths that are not already present, then renumber."""
    have = {p for p, _ in pairs}
    result = list(pairs)
    nxt = max((i for _, i in pairs), default=-1) + 1
    for path in paths:
        if path not in have:
            result.append((path, nxt))
            have.add(path)
            nxt += 1
    return renumber(result)


def remove_paths(pairs: list[tuple[str, int]], paths: list[str],
                 recursive: bool = False) -> list[tuple[str, int]]:
    """Drop paths; with ``recursive`` also drop everything below them."""
    targets = set(paths)
    prefixes = tuple(p + SEP for p in paths) if recursive else ()
    kept = [
        (path, index) for path, index in pairs
        if path not in targets and not (prefixes and path.startswith(prefixes))
    ]
    return renumber(kept)


# --------------------------------------------------------------------------
# ZKEYWORD tree
# --------------------------------------------------------------------------

def load_tree(con) -> dict[int, Node]:
    rows = con.execute("SELECT Z_PK, ZNAME, ZPARENT, ZLEFT, ZRIGHT FROM ZKEYWORD").fetchall()
    raw = {r[0]: (r[1], r[2], r[3], r[4]) for r in rows}

    def path_of(pk: int) -> str:
        name, parent, _, _ = raw[pk]
        return f"{path_of(parent)}{SEP}{name}" if parent else name

    return {
        pk: Node(pk, name, parent, left, right, path_of(pk))
        for pk, (name, parent, left, right) in raw.items()
    }


def by_path(tree: dict[int, Node]) -> dict[str, Node]:
    return {node.path: node for node in tree.values()}


def resolve(con, path: str, case_insensitive: bool = True) -> Node | None:
    """Find an existing node for a path, walking one segment at a time."""
    parent: int | None = None
    node: Node | None = None
    for segment in split_path(path):
        if parent is None:
            row = con.execute(
                "SELECT Z_PK, ZNAME, ZPARENT, ZLEFT, ZRIGHT FROM ZKEYWORD "
                "WHERE ZPARENT IS NULL AND ZNAME = ?", (segment,)).fetchone()
            if row is None and case_insensitive:
                row = con.execute(
                    "SELECT Z_PK, ZNAME, ZPARENT, ZLEFT, ZRIGHT FROM ZKEYWORD "
                    "WHERE ZPARENT IS NULL AND ZNAME = ? COLLATE NOCASE", (segment,)).fetchone()
        else:
            row = con.execute(
                "SELECT Z_PK, ZNAME, ZPARENT, ZLEFT, ZRIGHT FROM ZKEYWORD "
                "WHERE ZPARENT = ? AND ZNAME = ?", (parent, segment)).fetchone()
            if row is None and case_insensitive:
                row = con.execute(
                    "SELECT Z_PK, ZNAME, ZPARENT, ZLEFT, ZRIGHT FROM ZKEYWORD "
                    "WHERE ZPARENT = ? AND ZNAME = ? COLLATE NOCASE", (parent, segment)).fetchone()
        if row is None:
            return None
        node = Node(row[0], row[1], row[2], row[3], row[4], "")
        parent = row[0]
    return node


def _max_right(con) -> int:
    return con.execute("SELECT COALESCE(MAX(ZRIGHT), 0) FROM ZKEYWORD").fetchone()[0]


def insert_node(con, name: str, parent_pk: int | None, description: str | None = None) -> int:
    """Insert one ZKEYWORD row, preserving the nested set. Returns the new Z_PK.

    Z_PK is omitted: it is an INTEGER PRIMARY KEY (a rowid alias) and there is no
    Core Data counter table to keep in sync, so SQLite assigns max+1 itself.
    """
    name = validate_name(name)

    if parent_pk is None:
        # Appending a root is free: the new interval sits past every existing one.
        con.execute(
            "INSERT INTO ZKEYWORD (Z_ENT, ZNAME, ZPARENT, ZLEFT, ZRIGHT, ZISEXPORTABLE, ZDESCRIPTION) "
            "SELECT 44, ?, NULL, COALESCE(MAX(ZRIGHT), 0) + 1, COALESCE(MAX(ZRIGHT), 0) + 2, 1, ? "
            "FROM ZKEYWORD", (name, description))
        return con.execute("SELECT last_insert_rowid()").fetchone()[0]

    row = con.execute("SELECT ZRIGHT FROM ZKEYWORD WHERE Z_PK = ?", (parent_pk,)).fetchone()
    if row is None:
        raise KeywordError(f"parent keyword {parent_pk} does not exist")
    lo = row[0]
    top = _max_right(con)
    off = top + 1000

    # Both endpoints of a row must move in the SAME statement, or CHECK(ZLEFT < ZRIGHT)
    # fires midway; and the intermediate values must not collide with the untouched
    # ones, or the UNIQUE indexes fire. Lifting into a disjoint high range satisfies both.
    con.execute(
        "UPDATE ZKEYWORD SET "
        "  ZLEFT  = CASE WHEN ZLEFT  >= ? THEN ZLEFT  + ? ELSE ZLEFT  END, "
        "  ZRIGHT = CASE WHEN ZRIGHT >= ? THEN ZRIGHT + ? ELSE ZRIGHT END "
        "WHERE ZRIGHT >= ?", (lo, off, lo, off, lo))
    con.execute(
        "UPDATE ZKEYWORD SET "
        "  ZLEFT  = CASE WHEN ZLEFT  > ? THEN ZLEFT  - ? + 2 ELSE ZLEFT  END, "
        "  ZRIGHT = CASE WHEN ZRIGHT > ? THEN ZRIGHT - ? + 2 ELSE ZRIGHT END "
        "WHERE ZRIGHT > ?", (top, off, top, off, top))
    con.execute(
        "INSERT INTO ZKEYWORD (Z_ENT, ZNAME, ZPARENT, ZLEFT, ZRIGHT, ZISEXPORTABLE, ZDESCRIPTION) "
        "VALUES (44, ?, ?, ?, ?, 1, ?)", (name, parent_pk, lo, lo + 1, description))
    return con.execute("SELECT last_insert_rowid()").fetchone()[0]


def ensure_path(con, path: str, created: list[str] | None = None) -> int:
    """Resolve a path, creating any missing nodes along the way. Returns the leaf Z_PK."""
    parent: int | None = None
    walked: list[str] = []
    for segment in split_path(path):
        walked.append(segment)
        if parent is None:
            row = con.execute(
                "SELECT Z_PK FROM ZKEYWORD WHERE ZPARENT IS NULL AND ZNAME = ?", (segment,)).fetchone()
        else:
            row = con.execute(
                "SELECT Z_PK FROM ZKEYWORD WHERE ZPARENT = ? AND ZNAME = ?", (parent, segment)).fetchone()
        if row is None:
            parent = insert_node(con, segment, parent)
            if created is not None:
                created.append(SEP.join(walked))
        else:
            parent = row[0]
    assert parent is not None
    return parent


def rebuild_tree(con) -> None:
    """Recompute every ZLEFT/ZRIGHT by depth-first walk over ZPARENT.

    Used after move/merge/delete, where hand-rolled interval arithmetic would need
    a different shift for each case. With a few hundred nodes this is trivial, and
    it makes the invariant true by construction.
    """
    rows = con.execute("SELECT Z_PK, ZNAME, ZPARENT FROM ZKEYWORD").fetchall()
    children: dict[int | None, list[tuple[int, str]]] = {}
    for pk, name, parent in rows:
        children.setdefault(parent, []).append((pk, name))
    for group in children.values():
        group.sort(key=lambda t: (t[1].encode(), t[0]))

    assignments: list[tuple[int, int, int]] = []
    counter = 1

    def walk(parent: int | None) -> None:
        nonlocal counter
        for pk, _ in children.get(parent, []):
            left = counter
            counter += 1
            walk(pk)
            right = counter
            counter += 1
            assignments.append((left, right, pk))

    walk(None)
    if len(assignments) != len(rows):
        raise KeywordError("keyword tree has a cycle or an orphaned parent reference")

    # Vacate 1..2n first, so no assignment collides with a value still in place.
    offset = 2 * len(rows) + 1000
    con.execute("UPDATE ZKEYWORD SET ZLEFT = ZLEFT + ?, ZRIGHT = ZRIGHT + ?", (offset, offset))
    con.executemany("UPDATE ZKEYWORD SET ZLEFT = ?, ZRIGHT = ? WHERE Z_PK = ?", assignments)


def check_tree(con) -> list[str]:
    """Return a list of nested-set violations; empty means healthy."""
    problems: list[str] = []
    rows = con.execute("SELECT Z_PK, ZNAME, ZPARENT, ZLEFT, ZRIGHT FROM ZKEYWORD").fetchall()
    n = len(rows)
    values = sorted([r[3] for r in rows] + [r[4] for r in rows])
    if values != list(range(1, 2 * n + 1)):
        problems.append(f"ZLEFT/ZRIGHT are not a contiguous 1..{2 * n}")
    intervals = {r[0]: (r[3], r[4]) for r in rows}
    for pk, name, parent, left, right in rows:
        if left >= right:
            problems.append(f"keyword {name!r} ({pk}) has ZLEFT >= ZRIGHT")
        if parent is not None:
            if parent not in intervals:
                problems.append(f"keyword {name!r} ({pk}) points at missing parent {parent}")
            else:
                pl, pr = intervals[parent]
                if not (pl < left and right < pr):
                    problems.append(f"keyword {name!r} ({pk}) sits outside its parent's interval")
    return problems
