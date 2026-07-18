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
/// Only 18mm and 6mm have been reverse engineered so far (from captured
/// print jobs). Other widths supported by the printer are not yet mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapeWidth {
    Mm6,
    Mm18,
}

impl TapeWidth {
    /// Tape width in millimeters, as encoded in the job header (offset 8).
    pub fn width_mm(self) -> u8 {
        match self {
            TapeWidth::Mm6 => 6,
            TapeWidth::Mm18 => 18,
        }
    }

    /// Secondary tape-width-dependent byte in the job header (offset 23).
    /// Exact meaning unknown; values observed empirically.
    pub fn aux_byte(self) -> u8 {
        match self {
            TapeWidth::Mm6 => 3,
            TapeWidth::Mm18 => 6,
        }
    }

    /// Tape-width byte used in each raster row header.
    pub fn width_byte(self) -> u8 {
        match self {
            TapeWidth::Mm6 => 0x38,
            TapeWidth::Mm18 => 0xA8,
        }
    }

    /// Number of bitmap bytes per raster row (i.e. usable dots / 8).
    pub fn bitmap_bytes(self) -> usize {
        match self {
            TapeWidth::Mm6 => 8,
            TapeWidth::Mm18 => 22,
        }
    }

    /// Number of usable dots (bits) per raster row.
    pub fn bitmap_bits(self) -> usize {
        self.bitmap_bytes() * 8
    }
}
