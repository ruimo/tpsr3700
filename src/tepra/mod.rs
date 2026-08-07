//! TEPRA SR3700P proprietary print job encoding.
//!
//! This module re-implements the printer language captured from the
//! Windows driver's `FILE:`-port output (see `docs/` and `captures/` in the
//! repository root for the raw samples and analysis notes).

pub mod builder;
pub mod frame;
pub mod header;
pub mod raster;

/// Supported tape widths.
///
/// 6mm, 12mm and 18mm have been reverse engineered so far (from captured
/// print jobs). 24mm support is experimental: its raster dimensions are
/// extrapolated from the known formula, and [`TapeWidth::aux_byte`] is a
/// provisional value pending a captured 24mm print job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapeWidth {
    Mm6,
    Mm12,
    Mm18,
    Mm24,
}

impl TapeWidth {
    /// Tape width in millimeters, as encoded in the job header (offset 8).
    pub fn width_mm(self) -> u8 {
        match self {
            TapeWidth::Mm6 => 6,
            TapeWidth::Mm12 => 12,
            TapeWidth::Mm18 => 18,
            TapeWidth::Mm24 => 24,
        }
    }

    /// Secondary tape-width-dependent byte in the job header (offset 23).
    /// Exact meaning unknown; values observed empirically (6mm -> 3,
    /// 12mm -> 5, 18mm -> 6; no simple formula found relating this to
    /// `width_mm` or the bitmap size, hence the hardcoded lookup). 24mm is
    /// a provisional value for experimental support.
    pub fn aux_byte(self) -> u8 {
        match self {
            TapeWidth::Mm6 => 3,
            TapeWidth::Mm12 => 5,
            TapeWidth::Mm18 => 6,
            TapeWidth::Mm24 => 7,
        }
    }

    /// Tape-width byte used in each raster row header.
    ///
    /// Empirically `width_byte = width_mm * 28 / 3` exactly for all three
    /// known tape widths (6mm -> 56, 12mm -> 112, 18mm -> 168). 24mm uses
    /// the same extrapolated formula.
    pub fn width_byte(self) -> u8 {
        (self.width_mm() as u32 * 28 / 3) as u8
    }

    /// Number of bitmap bytes per raster row (i.e. usable dots / 8).
    ///
    /// Empirically `bitmap_bits = width_byte + 8` for all three known tape
    /// widths; this does *not* simply equal `width_mm` scaled by a fixed
    /// dots-per-mm factor (verified against 6mm/12mm/18mm captures, which
    /// use 8/15/22 bitmap bytes respectively -- not proportional to
    /// width_mm).
    pub fn bitmap_bytes(self) -> usize {
        (self.bitmap_bits() + 7) / 8
    }

    /// Number of usable dots (bits) per raster row.
    pub fn bitmap_bits(self) -> usize {
        self.width_byte() as usize + 8
    }

    /// Number of dots per raster row that the print head actually prints.
    /// The top and bottom `HEAD_MARGIN_BITS` bits of each bitmap row are
    /// non-printing margins (empirically observed: printed output is centred
    /// within this range).
    pub fn head_bits(self) -> usize {
        self.bitmap_bits() - 2 * Self::HEAD_MARGIN_BITS
    }

    /// Bit offset within the bitmap row at which the printable area starts.
    pub fn head_offset(self) -> usize {
        Self::HEAD_MARGIN_BITS
    }

    /// Non-printing margin at each edge of the bitmap, in bits.
    /// This is the hardware default (empirically confirmed).  Pass a
    /// different value to [`crate::font::FontRenderer::render_text`] /
    /// [`crate::font::FontRenderer::render_text_multiline`] via the
    /// `head_margin_bits` parameter to override it at print time.
    pub const HEAD_MARGIN_BITS: usize = 12;

    /// Converts a tape width index, as returned by the printer's
    /// "detect tape width" vendor control request (byte 3 of the
    /// response; see [`crate::usb::Printer::get_tape_info_raw`] and
    /// `docs/protocol.md` section 1.5), into a [`TapeWidth`].
    ///
    /// Only indices 1 (6mm), 3 (12mm), and 4 (18mm) have been confirmed
    /// against real hardware. Index 5 is assumed to be 24mm from KING JIM's
    /// official tape width lineup. Indices 0, 2, 6, and 7 are extrapolated
    /// from KING JIM's official tape width lineup for this printer line
    /// (4/6/9/12/18/24/36/50mm) but are not currently representable as a
    /// [`TapeWidth`], so they return `None` here too, pending support for
    /// those widths.
    pub fn from_detected_index(index: u8) -> Option<Self> {
        match index {
            1 => Some(TapeWidth::Mm6),
            3 => Some(TapeWidth::Mm12),
            4 => Some(TapeWidth::Mm18),
            5 => Some(TapeWidth::Mm24),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_byte_matches_captures() {
        assert_eq!(TapeWidth::Mm6.width_byte(), 0x38);
        assert_eq!(TapeWidth::Mm12.width_byte(), 0x70);
        assert_eq!(TapeWidth::Mm18.width_byte(), 0xA8);
        assert_eq!(TapeWidth::Mm24.width_byte(), 0xE0);
    }

    #[test]
    fn bitmap_bytes_matches_captures() {
        assert_eq!(TapeWidth::Mm6.bitmap_bytes(), 8);
        assert_eq!(TapeWidth::Mm12.bitmap_bytes(), 15);
        assert_eq!(TapeWidth::Mm18.bitmap_bytes(), 22);
        assert_eq!(TapeWidth::Mm24.bitmap_bytes(), 29);
    }

    #[test]
    fn from_detected_index_matches_hardware() {
        // Confirmed against real hardware with 6mm/12mm/18mm tapes loaded
        // (see docs/protocol.md section 1.5).
        assert_eq!(TapeWidth::from_detected_index(1), Some(TapeWidth::Mm6));
        assert_eq!(TapeWidth::from_detected_index(3), Some(TapeWidth::Mm12));
        assert_eq!(TapeWidth::from_detected_index(4), Some(TapeWidth::Mm18));
    }

    #[test]
    fn from_detected_index_supports_experimental_24mm() {
        assert_eq!(TapeWidth::from_detected_index(5), Some(TapeWidth::Mm24));
    }

    #[test]
    fn from_detected_index_rejects_unsupported_widths() {
        // Indices 0 (4mm), 2 (9mm), 6 (36mm), 7 (50mm) are not yet
        // implemented.
        for index in [0, 2, 6, 7] {
            assert_eq!(TapeWidth::from_detected_index(index), None);
        }
    }
}
