//! Job header block encoding.
//!
//! The header block is a large, mostly-constant `ESC { 0x80 ... }` frame
//! that precedes the configuration commands and raster data. Byte-for-byte
//! reverse engineering (diffing captures for different tape widths, font
//! sizes and character counts) showed that almost the entire block is
//! identical between jobs; only a handful of bytes vary with the tape
//! width, and the trailing checksum obviously depends on the whole block.
//!
//! Unresolved fields (kept as captured, unchanged):
//! - offset 20: appears to vary with glyph/object count in some captures,
//!   not yet understood; left at the template's default.
//! - large stretches of embedded metadata (`SPC10`, `306`,
//!   `obj_barcode`/`print_form`/`catalog_desc` strings) whose purpose is
//!   unknown but which are required verbatim for the printer to accept the
//!   job.

use crate::tepra::frame::checksum_of;
use crate::tepra::TapeWidth;

/// Raw bytes of the header block as captured from the Windows driver's
/// `FILE:`-port output for an 18mm tape job (`ESC { 0x80 ... checksum 0x7D`,
/// 2179 bytes total). Used as the template for all tape widths; only the
/// tape-width-dependent bytes and checksum are patched at build time.
const HEADER_TEMPLATE: &[u8] = include_bytes!("../../assets/header_18mm_template.bin");

/// Offset of the tape-width byte (in millimeters) within the header
/// payload (relative to the start of the whole frame, i.e. including the
/// `ESC {` and length byte).
const OFFSET_TAPE_WIDTH_MM: usize = 8;

/// Offset of a second tape-width-dependent byte whose exact meaning is not
/// yet understood (observed: 18mm -> 0x06, 6mm -> 0x03).
const OFFSET_TAPE_WIDTH_AUX: usize = 23;

/// Offset of a byte that grows by 4 for every extra character in the
/// print text (observed: 1 char -> 16 (0x10), 2 chars -> 20 (0x14)).
/// Formula: `12 + 4 * char_count`. Likely related to some per-object
/// table size; exact meaning otherwise unknown.
const OFFSET_CHAR_COUNT_FIELD: usize = 20;

/// Offset of the trailing checksum byte.
const OFFSET_CHECKSUM: usize = 2177;

/// Start (inclusive) / end (exclusive) of the checksum's input range.
const CHECKSUM_RANGE: std::ops::Range<usize> = 3..OFFSET_CHECKSUM;

/// Build the job header block for the given tape width and character
/// count (number of glyphs/characters being printed; used for
/// [`OFFSET_CHAR_COUNT_FIELD`]).
///
/// Returns the complete `ESC { 0x80 ... checksum 0x7D }` frame, ready to be
/// concatenated with the configuration commands and raster data.
pub fn build_header(tape: TapeWidth, char_count: u32) -> Vec<u8> {
    let mut header = HEADER_TEMPLATE.to_vec();
    header[OFFSET_TAPE_WIDTH_MM] = tape.width_mm();
    header[OFFSET_TAPE_WIDTH_AUX] = tape.aux_byte();
    header[OFFSET_CHAR_COUNT_FIELD] = (12 + 4 * char_count) as u8;
    header[OFFSET_CHECKSUM] = checksum_of(&header[CHECKSUM_RANGE]);
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_captured_18mm_header() {
        let expected = include_bytes!("../../tests/fixtures/a.prn");
        let header = build_header(TapeWidth::Mm18, 1);
        assert_eq!(header.as_slice(), &expected[..header.len()]);
    }

    #[test]
    fn matches_captured_6mm_header() {
        let expected = include_bytes!("../../tests/fixtures/a-6mm.prn");
        let header = build_header(TapeWidth::Mm6, 1);
        assert_eq!(header.as_slice(), &expected[..header.len()]);
    }

    #[test]
    fn matches_captured_two_char_header() {
        let expected = include_bytes!("../../tests/fixtures/aa.prn");
        let header = build_header(TapeWidth::Mm18, 2);
        assert_eq!(header.as_slice(), &expected[..header.len()]);
    }
}
