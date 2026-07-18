//! Raster row encoding.
//!
//! Each raster row (one "column" of the printed image, since the print
//! head lays down data perpendicular to the tape feed direction) is
//! encoded as:
//!
//! ```text
//! 0x1B 0x2E 0x00 0x0D 0x0D 0x01 <width_byte> <bitmap bytes>
//! ```
//!
//! The 5-byte header `00 0D 0D 01 <width_byte>` was observed unchanged
//! across every capture except for the final `width_byte`, which encodes
//! the tape width (and therefore the bitmap length): 18mm tape uses
//! `0xA8` with a 22-byte (176-bit) bitmap, 6mm tape uses `0x38` with an
//! 8-byte (64-bit) bitmap. The meaning of the other 4 header bytes (`00 0D
//! 0D 01`) is unknown; they are constant in every sample and are kept
//! as-is.
//!
//! There is no per-row checksum.

use crate::tepra::TapeWidth;

/// Fixed prefix of every raster row, before the tape-width byte.
const ROW_HEADER_PREFIX: [u8; 4] = [0x00, 0x0D, 0x0D, 0x01];

/// Build a single raster row from a bitmap.
///
/// `bitmap` must be exactly [`TapeWidth::bitmap_bytes`] bytes long, most
/// significant bit first, representing one column of dots across the tape
/// width.
pub fn build_row(tape: TapeWidth, bitmap: &[u8]) -> Vec<u8> {
    assert_eq!(
        bitmap.len(),
        tape.bitmap_bytes(),
        "bitmap length must match tape width's bit depth"
    );

    let mut row = Vec::with_capacity(2 + 4 + 1 + bitmap.len());
    row.push(0x1B);
    row.push(b'.');
    row.extend_from_slice(&ROW_HEADER_PREFIX);
    row.push(tape.width_byte());
    row.extend_from_slice(bitmap);
    row
}

/// Build an empty (all-zero) raster row for the given tape width.
pub fn build_blank_row(tape: TapeWidth) -> Vec<u8> {
    build_row(tape, &vec![0u8; tape.bitmap_bytes()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_row_matches_capture() {
        // First raster row of a.prn is an all-zero 18mm row.
        let expected = include_bytes!("../../tests/fixtures/a.prn");
        let row = build_blank_row(TapeWidth::Mm18);
        assert_eq!(row.len(), 29);
        assert_eq!(&row, &expected[2220..2220 + 29]);
    }

    #[test]
    fn nonblank_row_matches_capture() {
        // Row 45 (0-indexed) of a.prn's raster data.
        let expected = include_bytes!("../../tests/fixtures/a.prn");
        let start = 2220 + 45 * 29;
        let bitmap = &expected[start + 7..start + 29];
        let row = build_row(TapeWidth::Mm18, bitmap);
        assert_eq!(&row, &expected[start..start + 29]);
    }
}
