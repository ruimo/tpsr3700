//! Font rendering: turns text into TEPRA raster rows.
//!
//! Empirically, the TEPRA raster row index (feed direction) corresponds
//! directly to a glyph's X axis, and the bit position within a row (tape
//! width direction) corresponds directly to the glyph's Y axis — no
//! explicit rotation step is needed; rendering a string left-to-right in
//! the usual way and mapping (x, y) -> (row, bit) produces correctly
//! oriented output, confirmed against captured "A"/"B" print jobs.

use crate::tepra::TapeWidth;
use anyhow::{Context, Result};

/// Default font bundled with the system, used when no `--font` path is
/// given on the command line.
pub const DEFAULT_FONT_PATH: &str = "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc";

/// Empirically derived print head resolution (dots per inch), used to
/// convert a point size (as used by the TEPRA desktop app, matching the
/// `T` command's margin value in captured jobs) into fontdue's
/// pixels-per-em `px` parameter.
///
/// Derived from a captured 36pt "A" on 18mm tape: fontdue's rasterized
/// glyph width (feed direction) matched the captured bitmap's column span
/// (63 dots) exactly at `px = 104.0`, giving `104 / 36 * 72 = 208` dpi.
const DOTS_PER_INCH: f32 = 208.0;

/// Converts a point size into fontdue's `px` (pixels-per-em) parameter.
fn pt_to_px(pt: f32) -> f32 {
    pt * DOTS_PER_INCH / 72.0
}

/// A loaded font, ready to rasterize text into TEPRA raster rows.
pub struct FontRenderer {
    font: fontdue::Font,
}

impl FontRenderer {
    /// Load a font from a file path (`.ttf`, `.otf`, or `.ttc`; for `.ttc`
    /// collections, the first face is used).
    pub fn load(path: &str) -> Result<Self> {
        let data = std::fs::read(path)
            .with_context(|| format!("failed to read font file: {path}"))?;
        let font = fontdue::Font::from_bytes(data, fontdue::FontSettings::default())
            .map_err(|e| anyhow::anyhow!("failed to parse font {path}: {e}"))?;
        Ok(Self { font })
    }

    /// Render `text` at the given point size (matching the size shown in
    /// the TEPRA desktop app, e.g. 36pt) into a sequence of TEPRA raster
    /// rows for the given tape width.
    ///
    /// Returns `(rows, margin_rows)` where `margin_rows` is the number of
    /// leading blank rows (matching the `T` command semantics).
    pub fn render_text(
        &self,
        text: &str,
        size_pt: f32,
        tape: TapeWidth,
    ) -> (Vec<Vec<u8>>, u32) {
        let size_px = pt_to_px(size_pt);
        let bit_depth = tape.bitmap_bits();

        // Render each character to a coverage bitmap and place it into a
        // column buffer: outer index = column (x), inner = row (y, i.e.
        // dot position across the tape width).
        let mut columns: Vec<Vec<bool>> = Vec::new();

        // Vertically center glyphs within the usable tape width, using the
        // font's overall ascent+descent extent (not per-glyph metrics), so
        // that all characters in a string sit on a common baseline.
        let line_metrics = self.font.horizontal_line_metrics(size_px);
        let (ascent, descent) = match line_metrics {
            Some(m) => (m.ascent, m.descent),
            None => (size_px, 0.0),
        };
        let text_height = ascent - descent;
        let top_margin = (bit_depth as f32 - text_height) / 2.0;

        let mut cursor_x = 0.0f32;
        for ch in text.chars() {
            let (metrics, bitmap) = self.font.rasterize(ch, size_px);
            let glyph_width = metrics.width;
            let glyph_height = metrics.height;

            // Ensure we have enough columns allocated.
            let start_col = cursor_x.round() as usize;
            while columns.len() < start_col + glyph_width {
                columns.push(vec![false; bit_depth]);
            }

            for gy in 0..glyph_height {
                for gx in 0..glyph_width {
                    let coverage = bitmap[gy * glyph_width + gx];
                    if coverage > 127 {
                        // fontdue's bitmap is top-left origin, with row 0
                        // at `ascent - (ymin + height)` relative to the
                        // baseline. Convert to an absolute row within the
                        // tape's dot range.
                        let glyph_top = ascent - (metrics.ymin as f32 + glyph_height as f32);
                        let row = (top_margin + glyph_top + gy as f32).round() as isize;
                        if row >= 0 && (row as usize) < bit_depth {
                            let col = start_col + gx;
                            columns[col][row as usize] = true;
                        }
                    }
                }
            }

            cursor_x += metrics.advance_width;
        }

        let rows: Vec<Vec<u8>> = columns
            .iter()
            .map(|col| pack_bits(col, tape.bitmap_bytes()))
            .collect();

        // Trim leading/trailing all-blank columns: `margin_rows` (the `T`
        // command) declares the leading blank count, so it is not
        // necessary to keep blank rows in `rows` itself.
        let first_nonblank = rows.iter().position(|r| r.iter().any(|&b| b != 0));
        let last_nonblank = rows.iter().rposition(|r| r.iter().any(|&b| b != 0));

        let (trimmed, margin) = match (first_nonblank, last_nonblank) {
            (Some(first), Some(last)) => (rows[first..=last].to_vec(), first as u32),
            _ => (Vec::new(), 0),
        };

        (trimmed, margin)
    }
}

/// Packs a boolean slice (MSB-first) into bytes.
fn pack_bits(bits: &[bool], out_len: usize) -> Vec<u8> {
    let mut out = vec![0u8; out_len];
    for (i, &bit) in bits.iter().enumerate() {
        if bit {
            let byte_idx = i / 8;
            let bit_idx = 7 - (i % 8);
            if byte_idx < out.len() {
                out[byte_idx] |= 1 << bit_idx;
            }
        }
    }
    out
}
