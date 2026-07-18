//! Assembles the full byte stream for a TEPRA SR3700P print job.

use crate::tepra::frame::build_command_frame;
use crate::tepra::header::build_header;
use crate::tepra::raster::build_row;
use crate::tepra::TapeWidth;

/// Fixed parameters for the `C`, `D` and `G` configuration commands.
/// Their meaning is not understood; they were observed to be constant
/// across every captured sample (single/double characters, two font
/// sizes, two tape widths).
const CMD_C_PARAMS: [u8; 4] = [0x03, 0x01, 0x01, 0x01];
const CMD_D_PARAMS: [u8; 1] = [0x05];
const CMD_G_PARAMS: [u8; 0] = [];

/// Byte appended right after the last raster row and before the closing
/// `ESC { '@' '@' }` frame. Its purpose is unknown; observed constant in
/// every sample.
const TRAILING_BYTE: u8 = 0x0C;

/// Builds a complete print job byte stream from raster rows.
///
/// `rows` is a sequence of bitmaps, each exactly
/// `tape.bitmap_bytes()` long, one per raster column, in print order. This
/// corresponds 1:1 to the rows produced by [`crate::font`] rendering.
///
/// `char_count` is the number of characters/glyphs being printed (used for
/// a still not-fully-understood header field that scales with it; see
/// [`crate::tepra::header`]).
///
/// `margin_rows` is the number of leading blank rows before content
/// starts (encoded in the `T` command); it is the offset of the first
/// non-blank row in `rows`, or `0` if there is no leading margin to
/// declare separately (the caller may also just include blank rows in
/// `rows` and pass `0` here — both produce equivalent bitmaps, but to
/// exactly match the vendor driver's output, `margin_rows` should equal
/// the index of the first non-all-zero row).
pub fn build_print_job(
    tape: TapeWidth,
    char_count: u32,
    rows: &[Vec<u8>],
    margin_rows: u32,
) -> Vec<u8> {
    let mut job = Vec::new();

    job.extend_from_slice(&build_header(tape, char_count));

    job.extend_from_slice(&build_command_frame(b'C', &CMD_C_PARAMS));
    job.extend_from_slice(&build_command_frame(b'D', &CMD_D_PARAMS));
    job.extend_from_slice(&build_command_frame(b'G', &CMD_G_PARAMS));
    job.extend_from_slice(&build_command_frame(b'L', &(rows.len() as u32).to_le_bytes()));
    job.extend_from_slice(&build_command_frame(b'T', &(margin_rows as u16).to_le_bytes()));

    for bitmap in rows {
        job.extend_from_slice(&build_row(tape, bitmap));
    }

    job.push(TRAILING_BYTE);
    job.extend_from_slice(&build_command_frame(b'@', &[]));

    job
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extracts the raster rows and margin from a captured `.prn` file, for
    /// use as test input/expected-output pairs.
    fn extract_rows(data: &[u8], tape: TapeWidth) -> (Vec<Vec<u8>>, u32) {
        let bitmap_bytes = tape.bitmap_bytes();
        let row_len = 2 + 4 + 1 + bitmap_bytes;

        // Raster data starts right after the 5 configuration command
        // frames, which directly follow the fixed-size header block.
        let header_len = 2179;
        let cmd_c_len = 2 + 1 + 1 + CMD_C_PARAMS.len() + 1 + 1;
        let cmd_d_len = 2 + 1 + 1 + CMD_D_PARAMS.len() + 1 + 1;
        let cmd_g_len = 2 + 1 + 1 + CMD_G_PARAMS.len() + 1 + 1;
        let cmd_l_len = 2 + 1 + 1 + 4 + 1 + 1;
        let cmd_t_len = 2 + 1 + 1 + 2 + 1 + 1;
        let raster_start =
            header_len + cmd_c_len + cmd_d_len + cmd_g_len + cmd_l_len + cmd_t_len;

        // Find L parameter (row count) directly from the bytes instead of
        // recomputing, to keep the test independent from builder logic.
        let l_frame_start = header_len + cmd_c_len + cmd_d_len + cmd_g_len;
        let l_params_start = l_frame_start + 4; // ESC { len 'L' -> params start here
        let row_count = u32::from_le_bytes(
            data[l_params_start..l_params_start + 4].try_into().unwrap(),
        );

        let t_frame_start = l_frame_start + cmd_l_len;
        let t_params_start = t_frame_start + 4; // ESC { len 'T' -> params start here
        let margin = u16::from_le_bytes(
            data[t_params_start..t_params_start + 2].try_into().unwrap(),
        ) as u32;

        let mut rows = Vec::new();
        for i in 0..row_count as usize {
            let start = raster_start + i * row_len;
            let bitmap = data[start + 7..start + row_len].to_vec();
            rows.push(bitmap);
        }

        (rows, margin)
    }

    #[test]
    fn rebuilds_a_prn_exactly() {
        let expected = include_bytes!("../../tests/fixtures/a.prn").to_vec();
        let (rows, margin) = extract_rows(&expected, TapeWidth::Mm18);
        let job = build_print_job(TapeWidth::Mm18, 1, &rows, margin);
        assert_eq!(job, expected);
    }

    #[test]
    fn rebuilds_b_prn_exactly() {
        let expected = include_bytes!("../../tests/fixtures/b.prn").to_vec();
        let (rows, margin) = extract_rows(&expected, TapeWidth::Mm18);
        let job = build_print_job(TapeWidth::Mm18, 1, &rows, margin);
        assert_eq!(job, expected);
    }

    #[test]
    fn rebuilds_aa_prn_exactly() {
        let expected = include_bytes!("../../tests/fixtures/aa.prn").to_vec();
        let (rows, margin) = extract_rows(&expected, TapeWidth::Mm18);
        let job = build_print_job(TapeWidth::Mm18, 2, &rows, margin);
        assert_eq!(job, expected);
    }

    #[test]
    fn rebuilds_small_a_prn_exactly() {
        let expected = include_bytes!("../../tests/fixtures/small-a.prn").to_vec();
        let (rows, margin) = extract_rows(&expected, TapeWidth::Mm18);
        let job = build_print_job(TapeWidth::Mm18, 1, &rows, margin);
        assert_eq!(job, expected);
    }

    #[test]
    fn rebuilds_a_6mm_prn_exactly() {
        let expected = include_bytes!("../../tests/fixtures/a-6mm.prn").to_vec();
        let (rows, margin) = extract_rows(&expected, TapeWidth::Mm6);
        let job = build_print_job(TapeWidth::Mm6, 1, &rows, margin);
        assert_eq!(job, expected);
    }

    #[test]
    fn rebuilds_a_12mm_prn_exactly() {
        let expected = include_bytes!("../../tests/fixtures/a-12mm.prn").to_vec();
        let (rows, margin) = extract_rows(&expected, TapeWidth::Mm12);
        let job = build_print_job(TapeWidth::Mm12, 1, &rows, margin);
        assert_eq!(job, expected);
    }
}
