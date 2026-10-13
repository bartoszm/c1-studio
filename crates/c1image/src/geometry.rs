//! Straightening and cropping, as Capture One frames a picture after its quarter turn.

use c1model::Crop;

use crate::RgbImage;

/// Straightening smaller than this (degrees) is left alone: it moves no pixel visibly.
const NEGLIGIBLE: f32 = 0.01;

/// Turned clockwise by `degrees` about the centre, on a canvas grown to hold the whole
/// picture; the corners it uncovers are black. Bilinear.
pub fn straighten(image: RgbImage, degrees: f32) -> RgbImage {
    if degrees.abs() < NEGLIGIBLE || !degrees.is_finite() {
        return image;
    }
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (w, h) = (image.width() as f32, image.height() as f32);
    let out_w = (w * cos.abs() + h * sin.abs()).round().max(1.0) as u32;
    let out_h = (w * sin.abs() + h * cos.abs()).round().max(1.0) as u32;
    let mut out = RgbImage::new(out_w, out_h);
    let src = image.as_raw();
    let (iw, ih) = (image.width() as usize, image.height() as usize);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let row_bytes = out_w as usize * 3;
    let rows_per = (out_h as usize).div_ceil(threads).max(1);
    let raw: &mut [u8] = &mut out;
    std::thread::scope(|scope| {
        for (part, rows) in raw.chunks_mut(rows_per * row_bytes).enumerate() {
            scope.spawn(move || {
                for (k, row) in rows.chunks_exact_mut(row_bytes).enumerate() {
                    let v = (part * rows_per + k) as f32 + 0.5 - out_h as f32 / 2.0;
                    for (x, px) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                        let u = x as f32 + 0.5 - out_w as f32 / 2.0;
                        // The output turned back anticlockwise lands on the source.
                        let sx = u * cos + v * sin + w / 2.0 - 0.5;
                        let sy = -u * sin + v * cos + h / 2.0 - 0.5;
                        *px = bilinear(src, iw, ih, sx, sy);
                    }
                }
            });
        }
    });
    out
}

/// The pixel at (`x`, `y`), blended from its four neighbours; black off the image.
fn bilinear(src: &[u8], w: usize, h: usize, x: f32, y: f32) -> [u8; 3] {
    if x <= -1.0 || y <= -1.0 || x >= w as f32 || y >= h as f32 {
        return [0; 3];
    }
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xi: f32, yi: f32| -> [f32; 3] {
        if xi < 0.0 || yi < 0.0 || xi >= w as f32 || yi >= h as f32 {
            return [0.0; 3];
        }
        let i = (yi as usize * w + xi as usize) * 3;
        [f32::from(src[i]), f32::from(src[i + 1]), f32::from(src[i + 2])]
    };
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bottom = c[k] + (d[k] - c[k]) * fx;
        (top + (bottom - top) * fy + 0.5).clamp(0.0, 255.0) as u8
    })
}

/// The part of `image` that `crop` keeps. `image` is the picture after
/// [`straighten`]ing by `degrees`; the crop is measured against the picture before it.
pub fn crop(image: &RgbImage, crop: &Crop, degrees: f32) -> RgbImage {
    let (cw, ch) = (f64::from(image.width()), f64::from(image.height()));
    // The picture's own size, from the canvas it was turned onto.
    let (w, h) = if degrees.abs() < NEGLIGIBLE {
        (cw, ch)
    } else {
        let (sin, cos) = f64::from(degrees).to_radians().sin_cos();
        let (s, c) = (sin.abs(), cos.abs());
        let det = c * c - s * s;
        ((cw * c - ch * s) / det, (ch * c - cw * s) / det)
    };
    let width = (f64::from(crop.width) * w).round().clamp(1.0, cw);
    let height = (f64::from(crop.height) * h).round().clamp(1.0, ch);
    let left = (cw / 2.0 + f64::from(crop.dx) * w - width / 2.0).round().clamp(0.0, cw - width);
    let top = (ch / 2.0 + f64::from(crop.dy) * h - height / 2.0).round().clamp(0.0, ch - height);
    image::imageops::crop_imm(image, left as u32, top as u32, width as u32, height as u32).to_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_turns_change_nothing() {
        let image = RgbImage::from_fn(30, 20, |x, y| image::Rgb([x as u8, y as u8, 7]));
        assert_eq!(straighten(image.clone(), 0.001), image);
    }

    #[test]
    fn the_canvas_grows_to_hold_the_turn() {
        // A 3:2 frame turned 1.67° (DSCF thumbnails: 450×307 for a straightened 450×300).
        let out = straighten(RgbImage::new(7728, 5152), 1.67);
        let expected = 7728.0 * 1.67f32.to_radians().cos() + 5152.0 * 1.67f32.to_radians().sin();
        assert!((out.width() as f32 - expected).abs() <= 1.0, "{}", out.width());
        assert!((450.0 * out.height() as f32 / out.width() as f32 - 307.0).abs() < 1.0);
    }

    #[test]
    fn turning_is_clockwise() {
        // A bright spot right of centre moves down when turned clockwise.
        let mut image = RgbImage::new(101, 101);
        image.put_pixel(90, 50, image::Rgb([255; 3]));
        let out = straighten(image, 20.0);
        let (cx, cy) = (out.width() as f32 / 2.0, out.height() as f32 / 2.0);
        let (x, y, _) = out.enumerate_pixels().max_by_key(|(_, _, p)| p.0[0]).unwrap();
        assert!(x as f32 > cx + 30.0 && y as f32 > cy + 10.0, "({x}, {y})");
    }

    #[test]
    fn crop_measures_against_the_picture_before_the_turn() {
        let picture = RgbImage::new(600, 400);
        let half = Crop { dx: 0.25, dy: 0.0, width: 0.5, height: 1.0 };
        let out = crop(&picture, &half, 0.0);
        assert_eq!(out.dimensions(), (300, 400));
        // Turned, the canvas is larger but the crop still takes half the picture's width.
        let turned = straighten(picture, 3.0);
        let out = crop(&turned, &Crop { dx: 0.0, dy: 0.0, width: 0.5, height: 0.5 }, 3.0);
        assert!(
            (i64::from(out.width()) - 300).abs() <= 1 && (i64::from(out.height()) - 200).abs() <= 1,
            "{:?}",
            out.dimensions()
        );
    }
}
