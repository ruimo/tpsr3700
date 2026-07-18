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
/// print jobs). Other widths supported by the printer are not yet mapped
/// (in particular, [`TapeWidth::aux_byte`] does not follow an obvious
/// formula from the three known data points, so it is a hardcoded lookup
/// rather than a computed value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapeWidth {
    Mm6,
    Mm12,
    Mm18,
}

impl TapeWidth {
    /// Tape width in millimeters, as encoded in the job header (offset 8).
    pub fn width_mm(self) -> u8 {
        match self {
            TapeWidth::Mm6 => 6,
            TapeWidth::Mm12 => 12,
            TapeWidth::Mm18 => 18,
        }
    }

    /// Secondary tape-width-dependent byte in the job header (offset 23).
    /// Exact meaning unknown; values observed empirically (6mm -> 3,
    /// 12mm -> 5, 18mm -> 6; no simple formula found relating this to
    /// `width_mm` or the bitmap size, hence the hardcoded lookup).
    pub fn aux_byte(self) -> u8 {
        match self {
            TapeWidth::Mm6 => 3,
            TapeWidth::Mm12 => 5,
            TapeWidth::Mm18 => 6,
        }
    }

    /// Tape-width byte used in each raster row header.
    ///
    /// Empirically `width_byte = width_mm * 28 / 3` exactly for all three
    /// known tape widths (6mm -> 56, 12mm -> 112, 18mm -> 168).
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_byte_matches_captures() {
        assert_eq!(TapeWidth::Mm6.width_byte(), 0x38);
        assert_eq!(TapeWidth::Mm12.width_byte(), 0x70);
        assert_eq!(TapeWidth::Mm18.width_byte(), 0xA8);
    }

    #[test]
    fn bitmap_bytes_matches_captures() {
        assert_eq!(TapeWidth::Mm6.bitmap_bytes(), 8);
        assert_eq!(TapeWidth::Mm12.bitmap_bytes(), 15);
        assert_eq!(TapeWidth::Mm18.bitmap_bytes(), 22);
    }
}
