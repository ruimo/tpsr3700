//! TEPRA SR3700P Linux printing library.
//!
//! This crate re-implements (via reverse engineering of the Windows driver's
//! `FILE:`-port output) the proprietary printer language used by the
//! KING JIM TEPRA PRO SR3700P label printer, so it can be driven directly
//! from Linux without the vendor driver.

pub mod font;
pub mod tepra;
pub mod usb;
