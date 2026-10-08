"""Keyword serialisation and nested-set maintenance."""

from __future__ import annotations

import random

import pytest

from c1cat import keywords


# --------------------------------------------------------------------------
# serialisation
# --------------------------------------------------------------------------

def test_parse_round_trip():
    text = "35mm||0,b&w||3,best||5,best|best2018||4,israel||1,street||2"
    pairs = keywords.parse(text)
    assert len(pairs) == 6
    assert ("best|best2018", 4) in pairs
    assert keywords.serialise(pairs, "path") == text


def test_combined_layer_is_index_ordered():
    text = "35mm||0,b&w||3,best||5,best|best2018||4,israel||1,street||2"
    pairs = keywords.parse(text)
    assert keywords.serialise(pairs, "index") == (
        "35mm||0,israel||1,street||2,b&w||3,best|best2018||4,best||5")


def test_path_order_is_bytewise_not_locale():
    """'b&w' precedes 'best' because '&' (0x26) sorts below 'e' (0x65)."""
    pairs = [("best", 0), ("b&w", 1)]
    assert keywords.serialise(pairs, "path").startswith("b&w")


def test_empty_serialises_to_none():
    assert keywords.serialise([], "path") is None
    assert keywords.parse(None) == []
    assert keywords.parse("") == []


def test_indices_are_renumbered_dense():
    pairs = [("a", 7), ("b", 3), ("c", 90)]
    assert [i for _, i in keywords.renumber(pairs)] == [0, 1, 2]
    assert [p for p, _ in keywords.renumber(pairs)] == ["b", "a", "c"]


def test_add_is_idempotent():
    pairs = keywords.parse("street||0")
    once = keywords.add_paths(pairs, ["best"])
    twice = keywords.add_paths(once, ["best"])
    assert once == twice
    assert [p for p, _ in once] == ["street", "best"]


def test_remove_recursive_takes_children():
    pairs = keywords.parse("best||0,best|best2018||1,street||2")
    kept = keywords.remove_paths(pairs, ["best"], recursive=True)
    assert [p for p, _ in kept] == ["street"]
    kept = keywords.remove_paths(pairs, ["best"], recursive=False)
    assert [p for p, _ in kept] == ["best|best2018", "street"]


@pytest.mark.parametrize("name", ["a,b", "a|b", "", "   "])
def test_names_that_would_corrupt_the_format_are_rejected(name):
    with pytest.raises(keywords.KeywordError):
        keywords.validate_name(name)


# --------------------------------------------------------------------------
# the nested set
# --------------------------------------------------------------------------

def assert_healthy(con):
    problems = keywords.check_tree(con)
    assert problems == [], problems


def test_root_insert_keeps_the_set_contiguous(con):
    for name in ("street", "transport", "best"):
        keywords.insert_node(con, name, None)
    assert_healthy(con)


def test_child_insert_keeps_the_set_contiguous(con):
    transport = keywords.insert_node(con, "transport", None)
    keywords.insert_node(con, "street", None)
    for name in ("taxi", "tram", "metro"):
        keywords.insert_node(con, name, transport)
        assert_healthy(con)

    row = con.execute("SELECT ZLEFT, ZRIGHT FROM ZKEYWORD WHERE Z_PK = ?", (transport,)).fetchone()
    inside = con.execute("SELECT COUNT(*) FROM ZKEYWORD WHERE ZLEFT > ? AND ZRIGHT < ?",
                         (row[0], row[1])).fetchone()[0]
    assert inside == 3


def test_grandchild_nests_inside_both_ancestors(con):
    best = keywords.insert_node(con, "best", None)
    year = keywords.insert_node(con, "best2018", best)
    keywords.insert_node(con, "january", year)
    assert_healthy(con)

    tree = keywords.load_tree(con)
    paths = {node.path for node in tree.values()}
    assert paths == {"best", "best|best2018", "best|best2018|january"}


def test_ensure_path_creates_intermediate_parents(con):
    created: list[str] = []
    keywords.ensure_path(con, "a|b|c", created=created)
    assert created == ["a", "a|b", "a|b|c"]
    assert_healthy(con)
    # Second call creates nothing.
    created.clear()
    keywords.ensure_path(con, "a|b|c", created=created)
    assert created == []


def test_random_inserts_never_break_the_invariant(con):
    """The UNIQUE indexes and CHECK(ZLEFT < ZRIGHT) make ordering here load-bearing."""
    rng = random.Random(1234)
    pks = [keywords.insert_node(con, f"root{i}", None) for i in range(5)]
    for i in range(60):
        parent = rng.choice([None] + pks)
        pks.append(keywords.insert_node(con, f"kw{i}", parent))
        assert_healthy(con)
    assert con.execute("SELECT COUNT(*) FROM ZKEYWORD").fetchone()[0] == 65


def test_rebuild_after_reparenting(con):
    a = keywords.insert_node(con, "a", None)
    b = keywords.insert_node(con, "b", None)
    child = keywords.insert_node(con, "child", a)
    con.execute("UPDATE ZKEYWORD SET ZPARENT = ? WHERE Z_PK = ?", (b, child))
    keywords.rebuild_tree(con)
    assert_healthy(con)
    assert keywords.load_tree(con)[child].path == "b|child"


def test_delete_subtree_then_rebuild(con):
    keywords.insert_node(con, "keep", None)
    doomed = keywords.insert_node(con, "doomed", None)
    keywords.insert_node(con, "kid", doomed)
    node = keywords.resolve(con, "doomed")
    con.execute("DELETE FROM ZKEYWORD WHERE ZLEFT >= ? AND ZRIGHT <= ?", (node.left, node.right))
    keywords.rebuild_tree(con)
    assert_healthy(con)
    assert set(n.path for n in keywords.load_tree(con).values()) == {"keep"}


def test_resolve_is_case_insensitive_by_default(con):
    keywords.insert_node(con, "street", None)
    assert keywords.resolve(con, "Street") is not None
    assert keywords.resolve(con, "Street", case_insensitive=False) is None


def test_check_tree_detects_a_broken_set(con):
    keywords.insert_node(con, "a", None)
    con.execute("UPDATE ZKEYWORD SET ZRIGHT = 99 WHERE ZNAME = 'a'")
    assert keywords.check_tree(con) != []
