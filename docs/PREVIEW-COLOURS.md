# How previews get Capture One's colours

C1 Studio shows Capture One's photos without Capture One and without converting raw
files itself. It renders the large preview from Capture One's own caches. It takes the
look (colours, film simulation, black and white, tone curve, grading) from Capture
One's small finished thumbnail of the same photo. This page covers how that works, where
it falls short, and how to measure it. The code is in `crates/c1image`.

## What Capture One leaves on disk

For each variant, the catalog folder holds two renders:

| file | what it is | size | edits |
|---|---|---|---|
| `Cache/Thumbnails/<variant>.cot` | JPEG, sRGB, finished | 450 px on the long edge | all of them, including turn, straightening, perspective and lens corrections, but **not** the crop |
| `Cache/Previews/<sidecar>.cop` | JPEG-XL, **linear camera RGB before white balance** | about 2560 px | none: the whole sensor image, as shot |

The `.cop` is the sharp one, but it is raw-like. Shown as it is, it is green, dark and
flat, because a Fuji sensor's green channel is about twice as sensitive as its red. The
`.cot` has the look but is too small to view. So C1 Studio renders the `.cop` and borrows
the `.cot`'s look.

## The pipeline

`c1image::render_preview` (`jxl.rs`), for every preview:

1. **Decode** (`decode_linear`). Capture One declares a 2000-nit intensity target.
   Decoded to SDR, jxl-oxide would tone-map that and bend the linear data. So the file
   is decoded as PQ, which keeps absolute luminance, and turned back into linear values.
2. **White balance**: per-channel gains from the catalog's `ZWHITEBALANCE`. The combined
   settings layer is used first, then the adjustment layer, then the default layer
   (`source-c1`, `catalog::preview_source`).
3. **Highlight scale**: the 99.5th percentile of all channel values becomes white, and
   the brightest half percent clips. The `.cop` has no film curve, so its own maximum is
   not white.
4. **sRGB curve**, to 8 bits through a 65,536-step table, so shadows do not band.
5. **Framing**: the quarter turn, then straightening (`ZROTATION`) on a canvas grown to
   hold the turn. This frames the render the way the `.cot` is framed.
6. **The look**, from the `.cot`:
   1. **Shrink** the render to the thumbnail's size (box filter).
   2. **Line the two up** (`align.rs`, see below).
   3. **Fit a colour table** (`lut.rs`, see below) to the pixel pairs.
   4. **Apply the table** to the full-size render. A black-and-white look is greyed
      afterwards; the table learns the black-and-white channel mix too.
7. **Crop** (`ZCROP`), by `c1image::frame`.

The table is applied to the unwarped full-size render. Lining up only decides which
pixels are paired; it does not move anything in the picture that is shown.

### Lining up (`align.rs`)

The `.cot` has corrections the `.cop` lacks, such as lens distortion and perspective
(keystone). On real photos, parts of the two frames sit 3–28 thumbnail pixels apart, and
the gap varies across the frame.

A colour table fitted to pairs from slightly different spots is wrong in a predictable
way. Least squares predicts the *average* target colour for each source colour. When a
dark pixel is paired with what is partly its brighter neighbour, that average is lighter
than the true colour. So shadows came out too light, highlights too dark, and the whole
preview looked flat and hazy (see the before and after numbers below).

So the render is warped onto the thumbnail before pairing:

- The thumbnail is cut into 32 px blocks every 28 px. Flat blocks (grey levels varying
  by less than 3) are skipped, because they cannot be placed.
- Each block is found in the render by normalised cross-correlation, which ignores
  brightness and contrast differences. The search is ±28 px at half size, then ±2 px at
  full size, refined to a fraction of a pixel with a parabola.
- Matches correlating below 0.6 are dropped. A displacement field, quadratic in x and y
  (12 numbers), is fitted to the rest by weighted least squares. This covers shift,
  zoom, rotation, perspective and mild lens distortion. Blocks further than 3 px, then
  1.5 px, from the field are dropped as mismatches (repeating patterns, local edits),
  and the field is fitted again.
- With fewer than 8 good blocks (a flat or very dark frame), the two are paired as they
  are.

Lined up, the two may also differ in shape by up to 6% instead of 2%. That covers a
perspective fix that changes the thumbnail's shape.

### The colour table (`lut.rs`)

- A 17×17×17 grid over 8-bit sRGB, read with trilinear interpolation.
- The nodes are a least-squares fit to the pixel pairs, solved by conjugate gradients.
- A smoothness term asks neighbouring nodes to differ as the best single affine colour
  matrix would have them differ. Colours the photo lacks then follow that matrix
  instead of staying flat.
- A second fit weights each pair by 1 / (1 + (distance / 12)²), so edges and leftover
  mismatches count less. Pairs where the thumbnail is black but the render is not (the
  wedges a perspective fix leaves) are left out.
- `residual` is the median distance of the pairs from the table, out of 255. Above 25,
  the two are probably not the same frame. The table is dropped then.

### Fallbacks

When no table can be fitted, the preview falls back to `look.rs::apply_tones`. That
matches the distribution of brightness to the thumbnail's, keeping each pixel's colour
ratios: the tone curve, not the colours. This happens when the shapes differ by more
than 6% (more than 2% if the two could not be lined up), when there are fewer than 1,000
usable pairs, or when the residual is above 25. Without a `.cot` at all, the preview gets
white balance and the sRGB curve only.

The decision for one photo is `c1image::colour_fit(render, thumbnail)`. It returns a
`ColourFit`: `Table { table, shift }`, `ShapesDiffer` or `NoFit { residual }`.

## What it cannot do

- **Local adjustments**: masks, gradients, radial filters, vignetting, and clarity or
  structure (local contrast). A table maps each colour to one colour across the whole
  frame, so it learns an average of these. Detail finer than the 450 px thumbnail is
  not carried over either.
- **A stale thumbnail.** If Capture One has not re-rendered the `.cot` since the last
  edit, the preview takes the older look. Browsing the photo in Capture One refreshes it.
- **Framing**: the shown preview is not warped. Perspective (`ZGEOMETRYKEYSTONE*`) and
  lens distortion corrections that Capture One applies are not applied to the preview.
  The colours are right, but such photos sit a few percent differently in the frame than
  in Capture One.
- **Clipped highlights.** Anything above the 99.5th percentile is white before the table
  sees it, so highlights that Capture One recovers stay clipped.

## Measuring it

Two examples in `crates/c1cat` measure the match. Build them in release mode: the debug
build of c1cat overflows its stack.

**Against Capture One's exports**: the real target.

1. In Capture One, export some photos as sRGB JPEGs, about 2000 px. Name each after its
   image (`DSCF1234.jpg`; a second variant may be `DSCF1234_1.jpg` or
   `untitled-0066-2.jpg`).
2. Run:

   ```
   cargo run --release -p c1cat --example exportcheck -- <folder> [--dump <dir>]
   ```

Each export is matched to its photo. The output is the mean absolute difference per
channel, out of 255, for the thumbnail, the tone-curve match, the colour table, and the
preview *as shown* (framed as Capture One frames it).

With `--dump` it also prints, for each export:

- how the colours were fitted (the residual, and how far the lining up moved things);
- the shift per block between preview and thumbnail;
- *colour only*: the preview lined up with the export, so that framing does not count,
  with the mean signed error per channel in the export's shadows, midtones and
  highlights;
- the same for the thumbnail, as the bar to reach.

It also writes a 2×2 sheet per export: the export, the thumbnail, the preview, and their
difference ×4.

**Against the thumbnails**: for many photos at once, no exports needed.

```
cargo run --release -p c1cat --example lookcheck [-- <n>]
```

This takes `n` random photos (120 by default). Each method fits on half of the thumbnail
(a 16 px checkerboard) and is scored on the other half, lined up where possible. It also
reports render times and how often lining up succeeds.

### Results

On the 7 exports in `C:\tmp\fix_colors` (2026-11-20), *colour only*, mean absolute
difference out of 255:

| export | paired as they are | lined up | C1's thumbnail | largest shift |
|---|---|---|---|---|
| DSCF3468 | 17.7 | 8.8 | 5.0 | 13 px |
| DSCF7077_1 | 3.5 | 1.3 | 1.7 | 6 px |
| R0015727 | 11.8 | 11.8 | 7.3 | 1 px |
| untitled-0066-2 (black and white) | 9.0 | 5.4 | 6.2 | 6 px |
| untitled-0219-2 | 12.6 | 8.5 | 3.8 | 28 px |
| untitled-0268-2 | 9.1 | 4.8 | 4.5 | 9 px |
| untitled-0403 | 20.0 | 9.7 | 10.0 | 13 px |

Before lining up, every export showed the same signs: shadows 6–30 levels too light and
highlights 15–115 too dark. After, the bias is within about ±5 on all but two:

- R0015727 is the only one already lined up, so there was nothing to gain. What is left
  looks local (shadows +6); C1's own thumbnail is +5 there too.
- untitled-0219-2 has deep shadows +20 in green and blue, on 5% of the frame.

On 120 random photos from the fuji catalog, `lookcheck` (held-out halves, median, out of
255):

| | plain | + b&w | + b&w + tones | table | table, lined up |
|---|---|---|---|---|---|
| black and white (50) | 29.1 | 28.8 | 7.3 | 4.7 | 4.7 |
| colour (70) | 35.8 | 35.8 | 6.7 | 4.3 | 4.3 |

All 120 lined up. Most photos are already in line: the median of each photo's largest
shift is 0.1 px. One in ten is more than 4 px off, the most 8.6 px. So the median does
not move; the gain is in that tail, which is where the exports above come from. A
colour render takes about 90 ms more than the tone-curve one (fit, lining up and table
together).
