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
}
