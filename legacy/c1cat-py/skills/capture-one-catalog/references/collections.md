# Albums, groups, projects and smart albums

## The collection tree

Everything lives in `ZCOLLECTION`, distinguished by `Z_ENT`:

| `Z_ENT` | kind |
|---|---|
| 2 | album |
| 5 | smart album |
| 7 | project |
| 8 | group (a folder in the sidebar) |
| 11 | trash |
| 36 | catalog folder (mirrors the folders on disk) |

The root is `Z_PK = 1` (`[Collection]`, whose own `ZPARENT` is `-1`). `ZPARENT` is the
containing node, `ZPROJECT` the enclosing project (`1` when there is none), and
`ZCOLLECTIONINDEX` orders siblings.

## Creating one

```sql
INSERT INTO ZCOLLECTION
  (Z_ENT, ZUUID, ZNAME, ZPARENT, ZPROJECT, ZDOCUMENT, ZCOLLECTIONINDEX, ZCOLLAPSED,
   ZSORTORDER, ZDATECREATED, ZDATEMODIFIED, ZCONTENTMODIFIEDDATE,
   ZSERVEDBYCAPTUREPILOT, ZCAPTURECOLLECTIONOPTIONS, ZPROCESSCOLLECTIONOPTIONS,
   ZSEARCHCRITERIA, ZFILTERCRITERIA, ZSMARTCRITERIA)
VALUES (2, :uuid, :name, :parent, :project, 1, :index, 1,
        'date', :now, :now, :now, 0, 0, 0, :DEFAULT_SEARCH, :DEFAULT_FILTER, NULL);
```

`ZUUID` is a lowercase `uuid4`. `ZCOLLECTIONINDEX` is `MAX(...)+1` among siblings. Dates
are plain Unix epoch seconds. Even a plain album carries non-null criteria blobs, copied
byte for byte from one Capture One made — CRLF line endings included:

```
DEFAULT_SEARCH = '<?xml version="1.0" encoding="utf-8"?>\r\n<MatchOperator Kind="AND">\r\n
  <MatchOperator Kind="OR">\r\n    <Condition Enabled="NO">\r\n      <Key>ANY</Key>\r\n
      <Operator>6</Operator>\r\n      <Criterion></Criterion>\r\n    </Condition>\r\n
  </MatchOperator>\r\n</MatchOperator>'
DEFAULT_FILTER = '<?xml version="1.0" encoding="utf-8"?>\r\n<MatchOperator Kind="AND" />'
```

## Membership is two tables

Adding a photo to an album writes both:

```sql
INSERT OR IGNORE INTO ZIMAGEINCOLLECTION   (Z_ENT, ZCOLLECTION, ZIMAGE)   VALUES (47, :c, :img);
INSERT OR IGNORE INTO ZVARIANTINCOLLECTION (Z_ENT, ZCOLLECTION, ZVARIANT) VALUES (48, :c, :var);
```

`Z_PK` is omitted (rowid alias) and `OR IGNORE` leans on the unique indexes, so re-adding
is a no-op.

Removing is not symmetric: drop the variant link, but only drop the image link once **no
other variant of that image** remains in the collection. 568 images have two variants and
20 have three or more, so this matters.

`ZIMAGEINCOLLECTIONPROPERTIES` carries `ZMANUALSORTINDEX` and is optional — 13 487 rows
against 13 686 memberships. Only write it for manually ordered albums.

### The two tables are not required to agree

This catalog contains 739 variant links with no image link (all in **projects**, which
hold variant links only) and 2033 image links with no variant link (in albums, left
behind when a variant was removed). Capture One tolerates both. So "every link has a
partner" is not an invariant to enforce — the check that matters is whether an operation
*increased* the count for the collection it touched.

## Smart albums

`Z_ENT = 5` plus a `ZSMARTCRITERIA` XML blob. `ZSEARCHCRITERIA` stays the default no-op.

The top-level `Kind` is `AND` for match-all or `OR` for match-any. The **first inner
block is always a disabled `ANY` placeholder** — every smart album in the catalog has
one. Each real condition sits in its own `<MatchOperator Kind="OR">`:

```xml
<?xml version="1.0" encoding="utf-8"?>
<MatchOperator Kind="AND">
  <MatchOperator Kind="OR">
    <Condition Enabled="NO"><Key>ANY</Key><Operator>6</Operator><Criterion></Criterion></Condition>
  </MatchOperator>
  <MatchOperator Kind="OR">
    <Condition Enabled="YES"><Key>IB_I_BASIC_RATING</Key><Operator>2</Operator><Criterion>4</Criterion></Condition>
  </MatchOperator>
</MatchOperator>
```

### Operators

Decoded from smart albums in this catalog whose names state their intent — "street 4+"
uses operator 2 with criterion 4, and "street4+ 2025" uses operator 5 with criterion 3:

| operator | meaning |
|---|---|
| 0 | equals |
| 2 | is at least (≥) |
| 5 | is greater than (>) |
| 6 | contains |

### Keys

| key | type | seen with |
|---|---|---|
| `IB_I_BASIC_RATING` | integer | operators 0, 2, 5 |
| `IB_S_CONTENT_KEYWORDS` | string | operator 6 |
| `IB_S_EXP_DATE` | string | operator 0, criterion `2026` |
| `ANY` | — | the disabled placeholder |

That is the whole decoded vocabulary; Capture One supports many more keys (colour tag,
camera, lens, …) that have not been observed here. **Do not guess one.** Have the user
create the smart album once in Capture One, read the blob back with
`c1cat smart show NAME --xml`, and reuse the condition through
`--raw-condition KEY:OPERATOR:VALUE`.
