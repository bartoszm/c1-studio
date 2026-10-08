# Develop settings

`c1cat` reads develop settings and never writes them. Rendering a raw file is Capture
One's job, and the values interact in ways the database does not make explicit.

## Where the values live

Each variant has three layers in `ZVARIANTLAYER`, plus any style and local-adjustment
layers:

| `Z_ENT` | layer | what it holds |
|---|---|---|
| 18 | default | the neutral baseline for *this* file and camera |
| 17 | adjustment | **only what the user overrode** — `NULL` means "inherit" |
| 20 | combined | Capture One's own computed cache |
| 19 | style | one row per applied style (`ZNAME`, `ZSTYLEUUID`, `ZSTYLESOURCE`) |
| 23 | local adjustments | one row per local layer |

`ZVARIANT.ZISMODIFIED` is populated and meaningful — 57 460 modified against 43 872
untouched — so use it rather than inferring "edited" from the values.

## Reading a change correctly

Two mistakes are easy here.

**The combined layer is not default + adjustment.** It is a cache that carries
style-derived values. One sampled variant has adjustment exposure `0.3525` but combined
`0.0`, and combined contrast `12.31` which came from its style, not from the user. Never
use it as a baseline.

**A `NULL` on the adjustment layer is "inherit", not "cleared".** Diffing naively reports
every column the default layer sets and the adjustment layer does not — dozens of
spurious "changes" per photo.

So a real user change is: the adjustment layer holds a non-null value that differs from
the same column on that variant's own default layer.

```sql
SELECT * FROM ZVARIANTLAYER WHERE Z_PK IN (:adjustment_layer, :default_layer);
```

Diffing against the variant's own default layer means there is no table of per-camera
defaults to maintain, and a Capture One update that adds a slider degrades to an unnamed
column rather than being silently dropped.

## Column names

`ZVARIANTLAYER` has roughly 150 columns. The ones worth naming are mapped in
`c1cat/develop_names.py` — exposure, contrast, clarity, HDR, levels, curves, crop,
rotation, black & white, colour balance, sharpening, noise reduction, film grain, lens
corrections, keystone, retouching. Anything unmapped is reported under its raw column
name. Long packed values (curves, the colour editor table, mask blobs) are summarised as
`(set, N chars)` rather than printed.

Notable formats: `ZCROP` is `centreX;centreY;width;height`; `ZWHITEBALANCE` is
`R;G;B` multipliers with G normalised to 1; `ZFILMCURVE` and `ZICCPROFILE` are filenames
such as `FujiXPro2-Fujifilm CLASSIC CHROME.fcrv` and `FujiX100VI-Generic.icm`.

## Rendering a `.cop` preview

Relevant because the preview tier depends on it. A `.cop` is a JPEG-XL file holding
**linear, camera-space RGB before white balance**. Decoding it as-is gives a heavy green
cast, because a Fuji sensor's green channel is about twice as sensitive as its red.

To get something viewable:

1. Decode with `imagecodecs.jpegxl_decode`, which returns float32. Capture One writes
   **two frames**; take the last. (Pillow's JXL plugin returns clipped 8-bit — use
   `imagecodecs` instead.)
2. Multiply the channels by `ZWHITEBALANCE` from the variant's default or combined layer.
3. Normalise on a high percentile (the preview has no film curve, so 1.0 is not white).
4. Apply the sRGB transfer function.

Measured on one frame: raw channel means R 0.10 / G 0.21 / B 0.14 become R 0.198 /
G 0.215 / B 0.247 after white balance — neutral, and matching the slightly cool balance
of Capture One's own thumbnail for the same variant.

What this does **not** reproduce is the film curve, contrast, crop or black-and-white
conversion, so a rendered `.cop` looks flatter than Capture One's own output and shows
the uncropped frame. The `.cot` thumbnail, by contrast, is Capture One's finished render
at 450 px. For judging content, prefer `thumb`; for detail, use `preview` and accept that
it is the unedited rendering.
