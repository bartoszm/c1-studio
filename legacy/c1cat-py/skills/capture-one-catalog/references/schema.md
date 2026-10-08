# The Capture One catalog schema

Verified against a Capture One 16.8.1 Pro Win catalog (`ZVERSION=160800`) holding
100 716 images, 101 332 variants, 414 collections and 559 keywords. Every claim here was
checked against real data, not inferred from table names.

## Shape

A plain SQLite 3 database, `journal_mode=delete`, no WAL. Tables use a Core Data–style
`Z*` naming, but it is **not** Core Data: there is no `Z_PRIMARYKEY` counter table, and
every `Z_PK` is declared `INTEGER PRIMARY KEY`, i.e. a rowid alias. **Omit `Z_PK` on
INSERT** and SQLite assigns the next value itself.

`ZENTITIES(Z_ENT, ZNAME)` maps entity ids to names:

| id | entity | id | entity |
|---|---|---|---|
| 2 | AlbumCollection | 20 | VariantCombinedLayer |
| 5 | SmartCollection | 23 | LocalAdjustmentsLayer |
| 7 | ProjectCollection | 36 | CatalogFolderCollection |
| 8 | VirtualFolderCollection (a group) | 38 | PathLocation |
| 11 | TrashCollection | 41 | ImageInCollectionProperties |
| 12 | Image | 44 | Keyword |
| 13 | VariantMetadata | 47 | ImageInCollection |
| 15 | Variant | 48 | VariantInCollection |
| 17 | VariantAdjustmentLayer | 51 | RetouchingLayer |
| 18 | VariantDefaultLayer | | |
| 19 | VariantStyleLayer | | |

## Image → variant → layer → metadata

```
ZIMAGE ──1:N──> ZVARIANT ──> ZADJUSTMENTLAYER  ┐
                          ──> ZDEFAULTLAYER     ├─> ZVARIANTLAYER ──ZMETADATA──> ZVARIANTMETADATA
                          ──> ZCOMBINEDSETTINGS ┘
```

- All `FLOAT` date columns are **plain Unix epoch seconds**, not the Core Data 2001 epoch.
- `ZVARIANTMETADATA.Z_PK == ZVARIANTMETADATA.ZLAYER` for every row, no exceptions.
- 100 128 images have one variant, 568 have two, 20 have three or more.
- Referential integrity is perfect for these relations — 0 orphans — so inner joins are safe.

### Where user metadata lives

**On the adjustment layer's metadata row, mirrored onto the combined layer's.** Counts of
non-empty fields by layer entity:

| layer | rating | keywords | description | colour tag |
|---|---|---|---|---|
| 17 adjustment | 27 149 | 31 065 | 2 530 | 3 257 |
| 18 default (from the file itself) | 1 029 | 215 | 0 | 88 |
| 20 combined | 28 075 | 31 227 | 2 530 | 3 333 |
| 19 style, 23 local adjustments | 0 | 0 | 0 | 0 |

A read must pick the adjustment layer. A write must update **both** rows and never the
default layer, which represents what was embedded in the file on disk.

```sql
UPDATE ZVARIANTMETADATA SET ZBASIC_RATING = :r
 WHERE ZLAYER IN (SELECT ZADJUSTMENTLAYER  FROM ZVARIANT WHERE Z_PK = :v
                  UNION ALL
                  SELECT ZCOMBINEDSETTINGS FROM ZVARIANT WHERE Z_PK = :v);
```

## Metadata fields

- `ZBASIC_RATING` 0–5, but **unrated is `NULL`** (74 065 rows). `0` appears on 118 rows
  and is legacy — treat `NULL` and `0` alike on read, write `NULL` to clear.
- `ZCOLOR_TAG_INDEX` 1–7 = red, orange, yellow, green, blue, pink, purple. `NULL` (91 618
  rows) or `0` means none. `ZBASIC_LABEL` is unused.
- `ZCONTENT_KEYWORDS` is `NULL`, not `''`, when empty.
- IPTC in use: `ZIMAGE_COUNTRY` 27 262, `ZIMAGE_CITY` 25 043, `ZIMAGE_STATE` 12 590,
  `ZIMAGE_LOCATION` 5 051, `ZSTATUS_COPYRIGHTNOTICE` 60 751, `ZCONTACT_CREATOR` 1 590,
  `ZCONTENT_DESCRIPTION` 2 530. `ZIMAGE_DATECREATED` is `YYYYMMDD`.
- Unused and free for new data: `ZCONTENT_HEADLINE`, `ZSTATUS_TITLE`,
  `ZSTATUS_INSTRUCTIONS`, `ZSTATUS_JOBIDENTIFIER`, `ZIMAGE_SCENE`,
  `ZCONTENT_DESCRIPTIONWRITER`, `ZSTATUS_SOURCE`.

### GPS is on the image

`ZIMAGE.ZGPSLATITUDE` / `ZGPSLONGITUDE`, decimal degrees with a hemisphere letter:
`44.803944444445N`, `20.37835E`. Because it is per *image*, writing it affects every
variant of that image. Only 645 images have coordinates. `ZGPSALTITUDE` is unused.

## Timestamps

`ZVARIANT.ZLASTMETADATAMODIFICATIONDATE`, `ZXMPMETADATAMODIFICATIONDATE` and
`ZRAWMETADATAMODIFICATIONDATE` all hold the sentinel `-62135596800.0` (year 0001, .NET
`DateTime.MinValue`) on all 101 332 rows. They are unused — writing a real value would be
the anomalous act. Do bump `ZDOCUMENTCONTENT.ZDATEMODIFIED` on a commit, and
`ZCOLLECTION.ZDATEMODIFIED` / `ZCONTENTMODIFIEDDATE` when membership changes.

`ZSIDECAR` holds only 2 rows, so XMP sidecar sync is effectively off and there is little
risk of Capture One overwriting database metadata from files on disk.

## Files and cached pixels

`ZPATHLOCATION(ZWINROOT, ZRELATIVEPATH, ZISRELATIVE)`. The full Windows path is
`ZWINROOT || ZRELATIVEPATH || '\' || ZIMAGEFILENAME`; when `ZISRELATIVE=1` the path is
relative to the catalog folder (images imported *into* the catalog, under `Originals\`).

Three tiers of pixels, both cache paths derived from a column with no searching:

| tier | path | size |
|---|---|---|
| thumb | `Cache/Thumbnails/<ZVARIANT.Z_PK:08d>.cot` | 450 px, **a plain JPEG** |
| preview | `Cache/Previews/<ZIMAGE.ZSIDECARPATH>.cop` | 1920 or 2560 px, JPEG-XL |
| original | resolved from `ZPATHLOCATION` | full size |

- `.cot` thumbnails are ordinary JFIF JPEGs keyed by **variant**, and they are Capture
  One's own render — the crop, the black-and-white conversion and any style are baked in.
- `ZIMAGE.ZSIDECARPATH` (populated for all 100 716 images) *is* the preview cache relative
  path, e.g. `2026/8/9/548/R0012450.DNG`. It is sometimes lower-cased relative to the real
  filename, so match the basename case-insensitively.
- `.cop` files are JPEG-XL holding **linear, camera-space RGB before white balance**. See
  `develop-settings.md` for how to render one.
- Sibling `.cof` files are focus masks. Ignore them.

## Performance

The catalog sits on a Windows drive (9p/DrvFs). A `LIKE` scan over `ZVARIANTMETADATA`
joined to `ZIMAGE` takes ~30 s cold; the same query against a copy on WSL-native disk
takes 0.13 s, and copying the 715 MB file takes ~6 s. `ZIMAGE` has exactly one index, on
`ZISINSIDECATALOG`, and nothing indexes rating, keywords or dates.

This is why `c1cat` keeps a local index: a byte copy of the catalog plus a denormalised
`v` table, an FTS5 index and an R*Tree, rebuilt when the source mtime or size changes.
Because the copy retains the original tables, anything not denormalised is still
reachable there with the real schema.
