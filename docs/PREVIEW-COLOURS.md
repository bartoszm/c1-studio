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
7. **Local contrast** (`clarity.rs`): Capture One's Clarity and Structure, read from the
   combined layer (`ZCLARITY`, `ZCLARITYSTRUCTURE`). See below.
8. **Crop** (`ZCROP`), by `c1image::frame`.

The table is applied to the unwarped full-size render. Lining up only decides which
pixels are paired; it does not move anything in the picture that is shown.

### Lining up (`align.rs`)

The `.cot` has corrections the `.cop` lacks, such as lens distortion and perspective
(keystone). On real photos, parts of the two frames sit 3–27 thumbnail pixels apart, and
the gap varies across the frame.

A colour table fitted to pairs from slightly different spots is wrong in a predictable
way. Least squares predicts the *average* target colour for each source colour. When a
dark pixel is paired with what is partly its brighter neighbour, that average is lighter
than the true colour. So shadows came out too light, highlights too dark, and the whole
preview looked flat and hazy (see the before and after numbers below).

So the render is warped onto the thumbnail before pairing:

All matching uses normalised cross-correlation, which ignores brightness and contrast
differences. Distances below are in thumbnail pixels.

- First, the shift of the frame as a whole: the middle of the thumbnail is searched for
  at quarter size, ±48 px.
- Then the thumbnail is cut into 32 px blocks every 28 px. Flat blocks (grey levels
  varying by less than 3) are skipped, because they cannot be placed.
- Each block is searched for ±20 px around the frame's shift at half size, then ±2 px
  at full size, refined to a fraction of a pixel with a parabola. A block whose best
  half-size match lies at the edge of its ±20 px may really lie beyond it, so it is
  dropped. So are matches correlating below 0.6. The blocks are searched on several
  threads.
- A displacement field, quadratic in x and y (12 numbers), is fitted to the remaining
  blocks by weighted least squares. This covers shift, zoom, rotation, perspective and
  mild lens distortion. Blocks further than 3 px, then 1.5 px, from the field are
  dropped as mismatches (repeating patterns, local edits), and the field is fitted
  again.
- The two are paired as they are instead when fewer than 8 good blocks are left (a flat
  or very dark frame), or when the field would move any part of the frame more than
  60 px. The second case means it has swung out where no block holds it.

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

If the lined-up pairs give no table (fewer than 1,000 usable pairs, or a residual above
25), or the two could not be lined up, the pairs are tried as they are. That is only
done when the two have the same shape (within 2%), as before lining up existed. So
lining up never loses a table the old way would have found.

When neither gives a table, the preview falls back to `look.rs::apply_tones`. That
matches the distribution of brightness to the thumbnail's, keeping each pixel's colour
ratios: the tone curve, not the colours. This also happens when the shapes differ by
more than 6%. Without a `.cot` at all, the preview gets white balance and the sRGB curve
only.

The decision for one photo is `c1image::colour_fit(render, thumbnail)`. It returns a
`ColourFit`: `Table { table, shift }`, `ShapesDiffer` or `NoFit { residual }`.

### Local contrast (`clarity.rs`)

Clarity and Structure raise the contrast between neighbouring areas, not across the
tonal range. Capture One applies them at full size; the 450 px thumbnail hardly shows
them, and a colour table, which maps each colour to one colour everywhere, cannot carry
them. Without this step a preview of a photo with strong Structure looks flat beside
Capture One's export: blacks lifted, highlights dull, fine texture soft.

For each setting the step takes the brightness minus its Gaussian-blurred neighbourhood
(sigma 2% of the long side for Clarity, 0.3% for Structure), scales it by the setting,
and adds it to all three channels, so greys stay grey. A change towards white or black
eases off over the last 15% of the range, so highlights do not clip.

The gains (0.35 for Clarity, 0.6 for Structure at a setting of 100) were fitted to the
colour exports below by the *detail* measure of `exportcheck`. A black-and-white photo
takes half of that: Capture One's black-and-white exports show about half the local
contrast the same settings give a colour photo. The method (`ZCLARITYMETHOD`: Natural,
Punch, Neutral, Classic) is not told apart; every photo measured has the same one.

The step runs before straightening when the photo is straightened, so that the black
corners straightening leaves do not count as picture (they would leave a bright band
inside the crop). The colour table is still fitted on a straightened copy, which is
framed like the thumbnail, and applied to the unstraightened render: it works pixel by
pixel, so the order does not change its colours.

## What it cannot do

- **Local adjustments**: masks, gradients, radial filters and vignetting. A table maps
  each colour to one colour across the whole frame, so it learns an average of these.
  Clarity and Structure are added back (see above), but only as the global settings;
  ones painted on a layer are not.
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
- the same for the thumbnail, as the bar to reach;
- *brightness*: the 1st to 99th percentiles of brightness and the mean chroma of the
  export, the thumbnail and the preview, which blur and framing do not move;
- *detail*: the local contrast, fine and broad, at 1024 px wide.

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
| DSCF3468 | 17.9 | 10.0 | 5.0 | 15 px |
| DSCF7077_1 | 3.5 | 1.4 | 1.7 | 5 px |
| R0015727 | 11.8 | 11.8 | 7.3 | 1 px |
| untitled-0066-2 (black and white) | 9.0 | 5.4 | 6.2 | 6 px |
| untitled-0219-2 | 11.3 | 7.6 | 3.8 | 27 px |
| untitled-0268-2 | 9.1 | 4.8 | 4.5 | 9 px |
| untitled-0403 | 20.0 | 9.7 | 10.0 | 15 px |

The median went from 11.3 to 7.6. Three exports are now closer to Capture One's export
than its own thumbnail is (DSCF7077_1, untitled-0066-2, untitled-0403), and untitled-0268-2
is within 0.3 of it. *Colour only* lines the preview up with the export using the same
`align`, so these numbers move a little when `align` changes. The search was widened
after review, which moved DSCF3468 from 8.8 to 10.0 and
untitled-0219-2 from 8.5 to 7.6.

Before lining up, every export showed the same signs: shadows 6–30 levels too light and
highlights 15–115 too dark. After, the bias is within about ±6 on all but two:

- R0015727 is the only one already lined up, so there was nothing to gain. What is left
  looks local (shadows +6); C1's own thumbnail is +5 there too.
- untitled-0219-2 has deep shadows +15–18 in green and blue, on 5% of the frame (the
  thumbnail +7).

#### With local contrast (2026-11-27)

The bias left above was mostly not colour. Brightness percentiles showed the preview
following Capture One's thumbnail closely, and both flatter than the export on the
photos with strong Structure: R0015727 (Clarity 20, Structure 51) and untitled-0066-2
(31, 57). The others have 28 and 15. With local contrast added (*detail*, standard
deviation of brightness around its neighbourhood, out of 255):

| export | fine: export / before / after | broad: export / before / after | colour only before → after |
|---|---|---|---|
| DSCF3468 | 16.2 / 13.2 / 15.2 | 33.0 / 30.5 / 33.5 | 10.0 → 10.2 |
| DSCF7077_1 | 7.6 / 7.1 / 8.0 | 18.1 / 18.1 / 19.6 | 1.4 → 1.5 |
| R0015727 | 21.4 / 17.3 / 21.5 | 38.5 / 34.6 / 39.0 | 11.8 → 11.5 |
| untitled-0066-2 | 17.5 / 14.3 / 19.5 | 27.5 / 26.3 / 32.1 | 5.4 → 5.7 |
| untitled-0219-2 | 11.7 / 9.1 / 10.5 | 28.6 / 23.7 / 26.4 | 7.6 → 7.6 |
| untitled-0268-2 | 15.0 / 13.1 / 14.9 | 35.1 / 33.0 / 36.1 | 4.8 → 4.7 |
| untitled-0403 | 26.3 / 22.0 / 25.4 | 47.2 / 43.5 / 48.3 | 9.7 → 10.0 |

The signed bias shrank with it: highlights that were 3–6 levels too dark are within ±3.5
on all colour exports, and R0015727's within ±1.5. *Colour only* barely moves: it pairs
pixels, which favours the softer image, so it is no measure of local contrast.

The black-and-white untitled-0066-2 gets too much: its blacks reach 8 (export 0, before
11), but its brightest 1% sit 20 levels above the export's. One black-and-white
export is too few to fit it apart from the colour ones.

#### Black and white at half strength (2026-11-28)

Fourteen more black-and-white exports (Fujifilm RAF and Ricoh DNG; eleven with Clarity
and Structure, three without) settled it. At full strength the black-and-white previews
came out crisper than the export at both sizes on 9 of the 12 with the settings; the
colour ones did not. Fitting the two gains to each group by *detail* (root mean square of
the log ratio to the export, fine and broad together): colour 0.40 / 0.68 (error 0.053,
against 0.056 for the gains above), black and white 0 / 0.38 (0.058, against 0.100).
Half the colour gains, 0.18 / 0.30, gives 0.059, about as good, so a black-and-white
photo takes half. Detail, fine and broad, export / none / full / half:

| export | Clarity, Structure | fine | broad |
|---|---|---|---|
| DSCF0953_1 | 19, 40 | 12.6 / 11.8 / 14.4 / 13.1 | 31.7 / 30.7 / 33.8 / 32.2 |
| DSCF3017_1 | 19, 40 | 10.0 / 9.3 / 11.4 / 10.3 | 29.7 / 28.1 / 30.7 / 29.4 |
| DSCF3991_1 | 19, 40 | 21.8 / 19.9 / 24.8 / 22.3 | 40.1 / 38.3 / 43.6 / 40.9 |
| DSCF8261 | 19, 40 | 16.2 / 15.4 / 19.0 / 17.2 | 35.2 / 33.9 / 38.0 / 35.9 |
| DSCF8274 | 19, 40 | 15.4 / 14.8 / 18.2 / 16.4 | 37.0 / 36.6 / 40.4 / 38.5 |
| R0009111 | 10, 30 | 25.9 / 22.8 / 26.0 / 24.4 | 48.7 / 46.5 / 49.7 / 48.1 |
| R0014781 | 19, 40 | 14.9 / 13.6 / 16.4 / 15.0 | 33.5 / 29.6 / 32.6 / 31.1 |
| R0014921 | 19, 40 | 9.1 / 9.7 / 11.6 / 10.7 | 28.3 / 27.8 / 29.9 / 28.8 |
| R0015630 | 10, 30 | 14.0 / 11.7 / 13.4 / 12.5 | 28.1 / 25.7 / 27.3 / 26.5 |
| R0016303 | 10, 30 | 16.1 / 15.1 / 16.9 / 16.0 | 37.2 / 36.1 / 37.8 / 36.9 |
| R0016335 | 10, 30 | 18.5 / 15.3 / 17.6 / 16.4 | 36.6 / 33.9 / 36.3 / 35.0 |
| untitled-0066-2 | 31, 57 | 17.5 / 14.2 / 19.3 / 16.8 | 27.5 / 26.2 / 31.8 / 29.0 |

The three without the settings (DSCF3925_1, DSCF5952, DSCF9374) match the export within
0.2 either way, so the rest of the pipeline leaves the local contrast alone.
untitled-0066-2's brightest 1% now sit 8 levels above the export's, not 20. The
Ricoh ones at 10 / 30 are now a little soft (1–2 levels); the Fujifilm ones, which are
most of the catalog, were the most over. Several Ricoh DNGs keep a tone error the
thumbnail does not have (R0014781: shadows 16 levels too bright, highlights 11 too
dark): they have a second layer with its own Clarity and Structure, likely local
adjustments, which a table cannot carry (see below).

On 120 random photos from the fuji catalog, `lookcheck` (held-out halves, median, out of
255):

| | plain | + b&w | + b&w + tones | table | table, lined up |
|---|---|---|---|---|---|
| black and white (51) | 28.3 | 27.6 | 7.0 | 4.3 | 4.3 |
| colour (69) | 33.7 | 33.7 | 6.8 | 4.9 | 4.9 |

All 120 lined up. Most photos are already in line: the median of each photo's largest
shift is 0.1 px. One in ten is more than 4.7 px off, the most 13.6 px. So the median does
not move; the gain is in that tail, which is where the exports above come from. A
colour render takes about 45 ms more than the tone-curve one (fit, lining up and table
together).
