"""Metadata writes, collections and path handling."""

from __future__ import annotations

from pathlib import Path

import pytest

from c1cat import cmd_meta, collections as coll, keywords, paths, source
from c1cat.cmd_keywords import _write_variant_keywords


# --------------------------------------------------------------------------
# metadata goes to both layers, never the default one
# --------------------------------------------------------------------------

def test_metadata_rows_are_adjustment_and_combined(catalog):
    rows = source.metadata_rows(catalog, 1)
    assert len(rows) == 2
    kinds = source.layer_kinds(catalog, 1)
    assert sorted(kinds.values()) == [17, 20]


def test_rating_write_touches_both_and_spares_the_default(catalog):
    cmd_meta.write_fields(catalog, [1], {"rating": 5})
    values = catalog.execute(
        "SELECT l.Z_ENT, m.ZBASIC_RATING FROM ZVARIANTLAYER l"
        " JOIN ZVARIANTMETADATA m ON m.Z_PK = l.ZMETADATA WHERE l.ZVARIANT = 1"
        " ORDER BY l.Z_ENT").fetchall()
    assert [(r[0], r[1]) for r in values] == [(17, 5), (18, None), (20, 5)]


def test_writing_several_fields_at_once(catalog):
    cmd_meta.write_fields(catalog, [1], {"rating": 4, "color": 5, "description": "hello"})
    row = catalog.execute(
        "SELECT m.ZBASIC_RATING, m.ZCOLOR_TAG_INDEX, m.ZCONTENT_DESCRIPTION"
        " FROM ZVARIANT v JOIN ZVARIANTLAYER l ON l.Z_PK = v.ZADJUSTMENTLAYER"
        " JOIN ZVARIANTMETADATA m ON m.Z_PK = l.ZMETADATA WHERE v.Z_PK = 1").fetchone()
    assert tuple(row) == (4, 5, "hello")


def test_keywords_are_written_in_each_layer_own_order(catalog):
    keywords.ensure_path(catalog, "best|best2018")
    pairs = keywords.add_paths([], ["street", "b&w", "best|best2018"])
    _write_variant_keywords(catalog, 1, pairs)
    values = dict(catalog.execute(
        "SELECT l.Z_ENT, m.ZCONTENT_KEYWORDS FROM ZVARIANTLAYER l"
        " JOIN ZVARIANTMETADATA m ON m.Z_PK = l.ZMETADATA"
        " WHERE l.ZVARIANT = 1 AND l.Z_ENT IN (17, 20)").fetchall())
    assert values[17] == "b&w||1,best|best2018||2,street||0"      # by path
    assert values[20] == "street||0,b&w||1,best|best2018||2"      # by index


# --------------------------------------------------------------------------
# collections
# --------------------------------------------------------------------------

def test_album_membership_writes_both_tables(catalog):
    album = coll.create(catalog, "test", 2, 1)
    images, variants = coll.add_members(catalog, album, [1, 2, 3])
    assert (images, variants) == (3, 3)
    assert coll.dangling_links(catalog, album) == (0, 0)


def test_adding_twice_is_a_no_op(catalog):
    album = coll.create(catalog, "test", 2, 1)
    coll.add_members(catalog, album, [1, 2])
    images, variants = coll.add_members(catalog, album, [1, 2])
    assert (images, variants) == (0, 0)


def test_image_link_survives_while_a_sibling_variant_remains(catalog):
    """Two variants of one image: removing one must not drop the shared image link."""
    catalog.execute("UPDATE ZVARIANT SET ZIMAGE = 1 WHERE Z_PK = 2")
    album = coll.create(catalog, "test", 2, 1)
    coll.add_members(catalog, album, [1, 2])
    assert catalog.execute("SELECT COUNT(*) FROM ZIMAGEINCOLLECTION WHERE ZCOLLECTION = ?",
                           (album,)).fetchone()[0] == 1

    images, variants = coll.remove_members(catalog, album, [1])
    assert (images, variants) == (0, 1)      # variant link gone, image link stays
    images, variants = coll.remove_members(catalog, album, [2])
    assert (images, variants) == (1, 1)      # last variant leaves, image link follows


def test_sibling_index_increments(catalog):
    first = coll.create(catalog, "a", 2, 1)
    second = coll.create(catalog, "b", 2, 1)
    indices = [catalog.execute("SELECT ZCOLLECTIONINDEX FROM ZCOLLECTION WHERE Z_PK = ?",
                               (pk,)).fetchone()[0] for pk in (first, second)]
    assert indices[1] == indices[0] + 1


def test_delete_clears_membership_but_not_photos(catalog):
    album = coll.create(catalog, "test", 2, 1)
    coll.add_members(catalog, album, [1, 2])
    coll.delete(catalog, album)
    assert catalog.execute("SELECT COUNT(*) FROM ZVARIANTINCOLLECTION").fetchone()[0] == 0
    assert catalog.execute("SELECT COUNT(*) FROM ZVARIANT").fetchone()[0] == 5


# --------------------------------------------------------------------------
# smart criteria
# --------------------------------------------------------------------------

def test_smart_criteria_shape_matches_capture_one():
    xml = coll.smart_criteria([coll.rating_condition(">=4"),
                               (coll.KEY_KEYWORDS, coll.OP_CONTAINS, "street")])
    assert xml.startswith('<?xml version="1.0" encoding="utf-8"?>\r\n<MatchOperator Kind="AND">')
    assert xml.count('<MatchOperator Kind="OR">') == 3          # placeholder + two conditions
    assert '<Condition Enabled="NO">' in xml                     # the ANY placeholder
    assert "<Key>IB_I_BASIC_RATING</Key>\r\n      <Operator>2</Operator>" in xml
    assert xml.endswith("</MatchOperator>")


@pytest.mark.parametrize("expression,operator", [(">=4", 2), (">3", 5), ("=5", 0), ("5", 0)])
def test_rating_operators(expression, operator):
    assert coll.rating_condition(expression)[1] == operator


def test_criteria_round_trip():
    xml = coll.smart_criteria([(coll.KEY_KEYWORDS, coll.OP_CONTAINS, "street")])
    active = [c for c in coll.parse_criteria(xml) if c["enabled"]]
    assert len(active) == 1
    assert active[0]["key"] == coll.KEY_KEYWORDS
    assert active[0]["criterion"] == "street"


def test_criteria_escapes_the_criterion():
    xml = coll.smart_criteria([(coll.KEY_KEYWORDS, coll.OP_CONTAINS, "b&w")])
    assert "b&amp;w" in xml
    assert coll.parse_criteria(xml)[1]["criterion"] == "b&w"


# --------------------------------------------------------------------------
# paths
# --------------------------------------------------------------------------

def test_windows_path_translation():
    assert paths.win_to_wsl("H:\\") == "/mnt/h/"
    assert paths.wsl_to_win("/mnt/c/c1_catalogs/fuji/x.db") == "C:\\c1_catalogs\\fuji\\x.db"
    assert paths.win_to_wsl("\\\\NAS\\home") is None


def test_original_path_resolution():
    win, wsl = paths.original_paths("H:\\", "fotos\\2024", "a.RAF", False, Path("/cat"))
    assert win == "H:\\fotos\\2024\\a.RAF"
    assert wsl == "/mnt/h/fotos/2024/a.RAF"


def test_in_catalog_originals_are_relative_to_the_catalog():
    win, wsl = paths.original_paths("", "Originals\\2024\\3", "a.RAF", True, Path("/cat"))
    assert wsl == "/cat/Originals/2024/3/a.RAF"


@pytest.mark.parametrize("text,expected", [
    ("44.803944444445N", 44.803944444445), ("20.37835E", 20.37835),
    ("33.5S", -33.5), ("70.1W", -70.1), ("", None), (None, None),
])
def test_gps_parsing(text, expected):
    assert paths.parse_gps(text) == expected


def test_gps_formatting_matches_capture_one():
    assert paths.format_gps(44.803944444445, True) == "44.803944444445N"
    assert paths.format_gps(-21.9426, False) == "21.9426W"


def test_gps_round_trip():
    for value, is_lat in ((52.2297, True), (-33.8688, True), (21.0122, False), (-70.6693, False)):
        assert paths.parse_gps(paths.format_gps(value, is_lat)) == pytest.approx(value)


def test_thumbnail_path_is_zero_padded_by_variant():
    assert paths.thumbnail_path(Path("/cat"), 267113).name == "00267113.cot"
