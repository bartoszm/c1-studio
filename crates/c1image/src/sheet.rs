//! Contact sheets: many photos on one image, each numbered, for a model that describes
//! them in one call (the Python `sheets.py`, without file names on the sheet).

use image::{Rgb, RgbImage, imageops};

/// Photos per row and per column.
pub const GRID: u32 = 6;
/// The most photos on one sheet.
pub const PER_SHEET: usize = (GRID * GRID) as usize;
/// The sheet's width and height.
pub const SHEET_EDGE: u32 = 2000;
const GAP: u32 = 20;
/// The long edge a photo is fitted into: what to load it at.
pub const CELL: u32 = (SHEET_EDGE - GAP * (GRID + 1)) / GRID;

/// Where cell `i` (0-based, row by row) starts on the sheet.
pub fn cell_origin(i: usize) -> (u32, u32) {
    let (col, row) = (i as u32 % GRID, i as u32 / GRID);
    (GAP + col * (CELL + GAP), GAP + row * (CELL + GAP))
}

/// A white sheet with `photos` (at most [`PER_SHEET`]) fitted into its cells row by row,
/// each centred in its cell and numbered from 1 in its top left corner.
pub fn contact_sheet(photos: &[RgbImage]) -> RgbImage {
    let mut sheet = RgbImage::from_pixel(SHEET_EDGE, SHEET_EDGE, Rgb([255, 255, 255]));
    for (i, photo) in photos.iter().take(PER_SHEET).enumerate() {
        let photo = crate::fit(photo.clone(), CELL);
        let (x, y) = cell_origin(i);
        let (w, h) = photo.dimensions();
        imageops::replace(&mut sheet, &photo, i64::from(x + (CELL - w) / 2), i64::from(y + (CELL - h) / 2));
        label(&mut sheet, x, y, i + 1);
    }
    sheet
}

/// Columns a scene's sheet has at most, and its width: photos of one place, sent to a
/// model together, as large as one image of [`SCENE_EDGE`] allows.
pub const SCENE_COLUMNS: usize = 3;
pub const SCENE_EDGE: u32 = 1568;

/// A white sheet of a few photos (at most [`SCENE_COLUMNS`] squared), as many columns as
/// make it squarest, each photo numbered from 1 in its cell's top left corner.
pub fn scene_sheet(photos: &[RgbImage]) -> RgbImage {
    sheet_of(photos, SCENE_COLUMNS, SCENE_EDGE)
}

/// Columns a sheet of frames to compare has at most, and its width: the frames of a burst,
/// sent to a model together, need more of each than a scene's photos do.
pub const FRAMES_COLUMNS: usize = 4;
pub const FRAMES_EDGE: u32 = 2000;

/// A white sheet of the frames of a group (at most [`FRAMES_COLUMNS`] squared), laid out
/// as [`scene_sheet`] lays out a scene.
pub fn frames_sheet(photos: &[RgbImage]) -> RgbImage {
    sheet_of(photos, FRAMES_COLUMNS, FRAMES_EDGE)
}

/// A white sheet `edge` wide of at most `columns` squared photos, as many columns as make
/// it squarest, each photo numbered from 1 in its cell's top left corner.
fn sheet_of(photos: &[RgbImage], columns: usize, edge: u32) -> RgbImage {
    let n = photos.len().clamp(1, columns * columns);
    let cols = (1..=columns).find(|c| c * c >= n).unwrap_or(columns) as u32;
    let rows = (n as u32).div_ceil(cols);
    let cell = (edge - GAP * (cols + 1)) / cols;
    let height = GAP + rows * (cell + GAP);
    let mut sheet = RgbImage::from_pixel(edge, height, Rgb([255, 255, 255]));
    for (i, photo) in photos.iter().take(n).enumerate() {
        let photo = crate::fit(photo.clone(), cell);
        let (col, row) = (i as u32 % cols, i as u32 / cols);
        let (x, y) = (GAP + col * (cell + GAP), GAP + row * (cell + GAP));
        let (w, h) = photo.dimensions();
        imageops::replace(&mut sheet, &photo, i64::from(x + (cell - w) / 2), i64::from(y + (cell - h) / 2));
        label(&mut sheet, x, y, i + 1);
    }
    sheet
}

/// Digits 0-9, 3 wide and 5 high, one row of bits each (the top bit on the left).
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];
/// Pixels per bit of a digit.
const SCALE: u32 = 6;

/// `n` in white on a black box at the cell's corner: plain enough to read at any size.
fn label(sheet: &mut RgbImage, x: u32, y: u32, n: usize) {
    let text = n.to_string();
    let (w, h) = ((text.len() as u32 * 4 + 1) * SCALE, 7 * SCALE);
    for py in y..y + h {
        for px in x..x + w {
            sheet.put_pixel(px, py, Rgb([0, 0, 0]));
        }
    }
    for (k, digit) in text.bytes().map(|b| (b - b'0') as usize).enumerate() {
        let left = x + (1 + k as u32 * 4) * SCALE;
        for (row, bits) in DIGITS[digit].iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            let (px, py) = (left + col * SCALE + dx, y + (1 + row as u32) * SCALE + dy);
                            sheet.put_pixel(px, py, Rgb([255, 255, 255]));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photos_go_into_numbered_cells_row_by_row() {
        let red = RgbImage::from_pixel(600, 400, Rgb([200, 0, 0]));
        let photos = vec![red; 8];
        let sheet = contact_sheet(&photos);
        assert_eq!(sheet.dimensions(), (SHEET_EDGE, SHEET_EDGE));
        // The 7th photo starts the second row; the 9th cell is empty.
        assert_eq!(cell_origin(6), (GAP, GAP + CELL + GAP));
        let middle = |i: usize| {
            let (x, y) = cell_origin(i);
            *sheet.get_pixel(x + CELL / 2, y + CELL / 2)
        };
        assert_eq!(middle(7), Rgb([200, 0, 0]));
        assert_eq!(middle(8), Rgb([255, 255, 255]));
        // Each photo's number sits on a black box in its corner.
        let (x, y) = cell_origin(0);
        assert_eq!(*sheet.get_pixel(x + 1, y + 1), Rgb([0, 0, 0]));
        assert_eq!(CELL, 310);
    }

    #[test]
    fn a_scene_sheet_is_as_square_as_its_photos_allow() {
        let photo = RgbImage::from_pixel(300, 200, Rgb([10, 10, 10]));
        let one = scene_sheet(std::slice::from_ref(&photo));
        assert_eq!(one.width(), SCENE_EDGE);
        assert_eq!(one.height(), SCENE_EDGE, "one cell, square");
        let four = scene_sheet(&vec![photo.clone(); 4]);
        assert_eq!(four.height(), SCENE_EDGE, "two by two");
        let five = scene_sheet(&vec![photo.clone(); 5]);
        assert!(five.height() < SCENE_EDGE, "three across, two rows");
        let many = scene_sheet(&vec![photo; 12]);
        assert_eq!(many.height(), SCENE_EDGE, "nine at most");
    }

    #[test]
    fn a_sheet_of_frames_holds_sixteen_and_shows_two_large() {
        let frame = RgbImage::from_pixel(300, 200, Rgb([10, 10, 10]));
        let two = frames_sheet(&vec![frame.clone(); 2]);
        assert_eq!(
            (two.width(), two.height()),
            (FRAMES_EDGE, GAP + (FRAMES_EDGE - 3 * GAP) / 2 + GAP),
            "two across, one row"
        );
        let five = frames_sheet(&vec![frame.clone(); 5]);
        assert!(five.height() < FRAMES_EDGE, "three across, two rows");
        let many = frames_sheet(&vec![frame; 20]);
        assert_eq!(many.height(), FRAMES_EDGE, "four by four at most");
        // The sixteenth cell holds a frame; its number is on a black box.
        let cell = (FRAMES_EDGE - GAP * 5) / 4;
        let (x, y) = (GAP + 3 * (cell + GAP), GAP + 3 * (cell + GAP));
        assert_eq!(*many.get_pixel(x + 1, y + 1), Rgb([0, 0, 0]));
        assert_eq!(*many.get_pixel(x + cell / 2, y + cell / 2), Rgb([10, 10, 10]));
    }
}
