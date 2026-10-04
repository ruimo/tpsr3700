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
/// Raster dimensions for 4mm, 9mm, and 24mm were confirmed from USB
/// captures of official KING JIM SPC print jobs. 6mm, 12mm, and 18mm
/// values come from the existing captured print jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapeWidth {
    Mm4,
    Mm6,
    Mm9,
    Mm12,
    Mm18,
    Mm24,
}

impl TapeWidth {
    /// Tape width in millimeters, as encoded in the job header (offset 8).
    pub fn width_mm(self) -> u8 {
        match self {
            TapeWidth::Mm4 => 4,
            TapeWidth::Mm6 => 6,
            TapeWidth::Mm9 => 9,
            TapeWidth::Mm12 => 12,
            TapeWidth::Mm18 => 18,
            TapeWidth::Mm24 => 24,
        }
    }

    /// Secondary tape-width-dependent byte in the legacy job header
    /// (offset 23). Exact meaning unknown.
    ///
    /// Values 3, 5, and 6 were observed in captured FILE:-port jobs for
    /// 6mm, 12mm, and 18mm respectively. Values for 4mm and 9mm have not
    /// been observed; zero is used pending further reverse engineering.
    /// The 24mm value remains provisional.
    pub fn aux_byte(self) -> u8 {
        match self {
            TapeWidth::Mm4 => 0,
            TapeWidth::Mm6 => 3,
            TapeWidth::Mm9 => 0,
            TapeWidth::Mm12 => 5,
            TapeWidth::Mm18 => 6,
            TapeWidth::Mm24 => 7,
        }
    }

    /// Tape-width byte used in each raster row header.
    ///
    /// Values are taken from captured print jobs. They must not be
    /// extrapolated from the nominal tape width: notably 4mm and 24mm do
    /// not follow the relation previously inferred from 6/12/18mm.
    pub fn width_byte(self) -> u8 {
        match self {
            TapeWidth::Mm4 => 0x20,
            TapeWidth::Mm6 => 0x38,
            TapeWidth::Mm9 => 0x50,
            TapeWidth::Mm12 => 0x70,
            TapeWidth::Mm18 => 0xA8,
            TapeWidth::Mm24 => 0xC0,
        }
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
    /// These mappings have been tested against a physical SR3700P.
    pub fn from_detected_index(index: u8) -> Option<Self> {
        match index {
            11 => Some(TapeWidth::Mm4),
            1 => Some(TapeWidth::Mm6),
            2 => Some(TapeWidth::Mm9),
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
        assert_eq!(TapeWidth::Mm4.width_byte(), 0x20);
        assert_eq!(TapeWidth::Mm6.width_byte(), 0x38);
        assert_eq!(TapeWidth::Mm9.width_byte(), 0x50);
        assert_eq!(TapeWidth::Mm12.width_byte(), 0x70);
        assert_eq!(TapeWidth::Mm18.width_byte(), 0xA8);
        assert_eq!(TapeWidth::Mm24.width_byte(), 0xC0);
    }

    #[test]
    fn bitmap_bytes_matches_captures() {
        assert_eq!(TapeWidth::Mm4.bitmap_bytes(), 5);
        assert_eq!(TapeWidth::Mm6.bitmap_bytes(), 8);
        assert_eq!(TapeWidth::Mm9.bitmap_bytes(), 11);
        assert_eq!(TapeWidth::Mm12.bitmap_bytes(), 15);
        assert_eq!(TapeWidth::Mm18.bitmap_bytes(), 22);
        assert_eq!(TapeWidth::Mm24.bitmap_bytes(), 25);
    }

    #[test]
    fn from_detected_index_matches_hardware() {
        // Tested against a physical SR3700P with each tape width loaded
        // (see docs/protocol.md section 1.5).
        assert_eq!(TapeWidth::from_detected_index(11), Some(TapeWidth::Mm4));
        assert_eq!(TapeWidth::from_detected_index(1), Some(TapeWidth::Mm6));
        assert_eq!(TapeWidth::from_detected_index(2), Some(TapeWidth::Mm9));
        assert_eq!(TapeWidth::from_detected_index(3), Some(TapeWidth::Mm12));
        assert_eq!(TapeWidth::from_detected_index(4), Some(TapeWidth::Mm18));
        assert_eq!(TapeWidth::from_detected_index(5), Some(TapeWidth::Mm24));
    }

    #[test]
    fn from_detected_index_rejects_unknown_widths() {
        for index in [0, 6, 7, 8, 9, 10, 12] {
            assert_eq!(TapeWidth::from_detected_index(index), None);
        }
    }
}
