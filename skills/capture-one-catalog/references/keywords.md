# Keywords

## How a photo's keywords are stored

`ZVARIANTMETADATA.ZCONTENT_KEYWORDS` is a single string: comma-separated
`<path>||<index>` tokens, where `path` uses `|` between hierarchy levels.

The same variant stores the same set **twice, in two different orders**:

```
adjustment layer (Z_ENT 17)   35mm||0,b&w||3,best||5,best|best2018||4,israel||1,street||2
combined layer   (Z_ENT 20)   35mm||0,israel||1,street||2,b&w||3,best|best2018||4,best||5
```

- The adjustment layer is sorted **by path**, as raw bytes. In the example `b&w` comes
  before `best` because `&` (0x26) sorts below `e` (0x65). Use plain byte comparison —
  not locale collation, not casefolding.
- The combined layer is sorted **by index**. The index is display order.
- Indices are always a dense `0..n-1`.
- Empty means `NULL`, not `''`.

Validated against all 62 292 non-empty values in the catalog: 62 287 round-trip
byte-exactly. The 5 that differ are adjustment-layer rows that happen to be
index-ordered, which shows Capture One accepts either order and treats it as cosmetic.

**No keyword name contains `,` or `|`**, which is what makes the format unambiguous.
`c1cat` refuses to create a keyword whose name contains either, because one such name
would corrupt every keyword string in the catalog.

Matching a keyword with `LIKE '%street%'` is wrong — it also hits `streetlight` and
`street art`. `c1cat` stores a `\x1f`-delimited copy so it can match whole tokens.

## The keyword library

`ZKEYWORD(Z_PK, Z_ENT=44, ZNAME, ZPARENT, ZLEFT, ZRIGHT, ZISEXPORTABLE, ZDESCRIPTION)`
is a **nested set**: `ZLEFT` and `ZRIGHT` are both `UNIQUE`, there is a row-level
`CHECK(ZLEFT < ZRIGHT)`, and `UNIQUE(ZNAME, ZPARENT)`. In a healthy catalog the union of
every `ZLEFT` and `ZRIGHT` is exactly `1..2n` — verified: 559 nodes covering 1..1118.

Roots have `ZPARENT IS NULL` (453 of 559). The tree is at most two levels deep. All rows
have `ZISEXPORTABLE=1`. 102 keywords exist but are unused, so removing a keyword from
photos does not remove it from the library.

`UNIQUE(ZNAME, ZPARENT)` is BINARY, so `Street` and `street` *can* coexist. `c1cat`
resolves case-insensitively and reuses the existing node rather than creating a variant.

### Adding a root keyword is free

The new interval sits past every existing one, so nothing needs to move:

```sql
INSERT INTO ZKEYWORD (Z_ENT, ZNAME, ZPARENT, ZLEFT, ZRIGHT, ZISEXPORTABLE, ZDESCRIPTION)
SELECT 44, :name, NULL, MAX(ZRIGHT) + 1, MAX(ZRIGHT) + 2, 1, :description FROM ZKEYWORD;
```

### Adding a child needs a shift, and the obvious ways all fail

Given the parent's `ZRIGHT = :lo`, `:top = MAX(ZRIGHT)`, and `:off = :top + 1000`:

```sql
UPDATE ZKEYWORD SET
    ZLEFT  = CASE WHEN ZLEFT  >= :lo THEN ZLEFT  + :off ELSE ZLEFT  END,
    ZRIGHT = CASE WHEN ZRIGHT >= :lo THEN ZRIGHT + :off ELSE ZRIGHT END
  WHERE ZRIGHT >= :lo;
UPDATE ZKEYWORD SET
    ZLEFT  = CASE WHEN ZLEFT  > :top THEN ZLEFT  - :off + 2 ELSE ZLEFT  END,
    ZRIGHT = CASE WHEN ZRIGHT > :top THEN ZRIGHT - :off + 2 ELSE ZRIGHT END
  WHERE ZRIGHT > :top;
INSERT INTO ZKEYWORD (Z_ENT, ZNAME, ZPARENT, ZLEFT, ZRIGHT, ZISEXPORTABLE)
  VALUES (44, :name, :parent, :lo, :lo + 1, 1);
```

Why it has to look like this:

- **Both endpoints of a row must move in the same statement.** Shifting `ZLEFT` in one
  `UPDATE` and `ZRIGHT` in another trips `CHECK(ZLEFT < ZRIGHT)` in between: a leaf whose
  `ZRIGHT = ZLEFT + 1` briefly gets `ZLEFT + 2 > ZRIGHT`.
- **Negate-then-restore fails too**, for the same reason: `-(L+2) < -(R+2)` requires
  `L > R`, which is false by construction.
- **The intermediate values must be disjoint from the untouched ones**, or the `UNIQUE`
  indexes fire mid-statement. Lifting everything above `MAX(ZRIGHT)` guarantees that, and
  bringing it back down to `[lo+2, top+2]` stays clear of the untouched `[1, lo-1]`.

For move, merge and delete, `c1cat` mutates `ZPARENT` and then rebuilds the whole nested
set by depth-first walk. With a few hundred nodes that is instant and makes the invariant
true by construction rather than by three different flavours of interval arithmetic.

Every keyword mutation re-checks the invariant inside the transaction and rolls back if
it fails.

## Renaming

A rename rewrites every photo that used the old path, and every path beneath it. `street`
alone appears on 20 350 photos, so this is the largest write the tool can make. `c1cat`
finds the affected variants through the local index and then updates only those rows by
primary key, rather than issuing a `LIKE`-driven `UPDATE` against the catalog.
