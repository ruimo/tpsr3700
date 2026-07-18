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

/// Default feed-direction margin (in millimeters) added before and after
/// the printed content, matching the TEPRA desktop app's default margin
/// setting.
pub const DEFAULT_FEED_MARGIN_MM: f32 = 3.0;

/// Converts a length in millimeters to a number of raster rows (feed
/// direction dots), using [`DOTS_PER_INCH`].
///
/// This assumes the feed direction has the same dot density as the tape
/// width direction (from which `DOTS_PER_INCH` was derived); this has not
/// been independently verified and may not be exactly correct, but is the
/// best available estimate absent a captured multi-row-margin sample to
/// calibrate against directly.
pub fn mm_to_rows(mm: f32) -> u32 {
    (mm / 25.4 * DOTS_PER_INCH).round() as u32
}

/// Converts a point size into fontdue's `px` (pixels-per-em) parameter.
fn pt_to_px(pt: f32) -> f32 {
    pt * DOTS_PER_INCH / 72.0
}

/// Converts fontdue's `px` (pixels-per-em) parameter into a point size.
fn px_to_pt(px: f32) -> f32 {
    px * 72.0 / DOTS_PER_INCH
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

    /// Returns this font's line height (ascent - descent) in dots
    /// (fontdue `px` units) for a given `px` size. fontdue's line metrics
    /// scale linearly with `px`, so this is also usable as a per-font
    /// dots-per-`px` ratio (i.e. `line_height_px(1.0)`).
    fn line_height_px(&self, size_px: f32) -> f32 {
        match self.font.horizontal_line_metrics(size_px) {
            Some(m) => m.ascent - m.descent,
            None => size_px,
        }
    }

    /// Returns the largest point size at which this font's line height
    /// (ascent - descent) fits within `tape`'s usable dot range, i.e. the
    /// largest size that won't have its top or bottom clipped.
    ///
    /// Note this is a conservative, content-independent estimate based on
    /// the font's overall vertical metrics, not the actual ink extents of
    /// the text being printed (which are usually smaller, e.g. text
    /// without descenders like "A" or all-uppercase strings).
    pub fn max_size_pt_for_tape(&self, tape: TapeWidth) -> f32 {
        let ratio_per_px = self.line_height_px(1.0);
        let max_px = tape.bitmap_bits() as f32 / ratio_per_px;
        px_to_pt(max_px)
    }

    /// Returns `true` if rendering at `size_pt` would clip the top or
    /// bottom of the text on `tape` (based on the font's overall vertical
    /// metrics, same caveat as [`FontRenderer::max_size_pt_for_tape`]).
    pub fn would_clip(&self, size_pt: f32, tape: TapeWidth) -> bool {
        let size_px = pt_to_px(size_pt);
        self.line_height_px(size_px) > tape.bitmap_bits() as f32
    }

    /// Render `text` at the given point size (matching the size shown in
    /// the TEPRA desktop app, e.g. 36pt) into a sequence of TEPRA raster
    /// rows for the given tape width, with `feed_margin_mm` millimeters of
    /// blank tape feed added before and after the printed content (see
    /// [`DEFAULT_FEED_MARGIN_MM`]).
    ///
    /// Returns `(rows, margin_rows)` where `margin_rows` is the number of
    /// leading blank rows, including the feed margin (matching the `T`
    /// command semantics).
    pub fn render_text(
        &self,
        text: &str,
        size_pt: f32,
        tape: TapeWidth,
        feed_margin_mm: f32,
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
                            // The bit position within a raster row's
                            // bitmap is mirrored relative to fontdue's
                            // top-to-bottom glyph coordinates: printing an
                            // unmodified `row` produced upside-down (and
                            // effectively mirrored, since it also flips
                            // asymmetric multi-character strings)
                            // output on real hardware. Flipping the bit
                            // position fixes this; confirmed against
                            // real-hardware printouts of "AB" on 6mm tape.
                            let bit_pos = bit_depth - 1 - row as usize;
                            let col = start_col + gx;
                            columns[col][bit_pos] = true;
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

        // Trim leading/trailing all-blank columns (any inherent glyph side
        // bearing), then add an explicit feed margin of blank rows before
        // and after -- this is a fixed physical distance (tape feed),
        // independent of glyph shape, unlike the side-bearing trimming
        // above.
        let first_nonblank = rows.iter().position(|r| r.iter().any(|&b| b != 0));
        let last_nonblank = rows.iter().rposition(|r| r.iter().any(|&b| b != 0));

        let trimmed = match (first_nonblank, last_nonblank) {
            (Some(first), Some(last)) => rows[first..=last].to_vec(),
            _ => Vec::new(),
        };

        let margin_rows = mm_to_rows(feed_margin_mm);
        let blank_row = vec![0u8; tape.bitmap_bytes()];

        let mut padded = Vec::with_capacity(trimmed.len() + 2 * margin_rows as usize);
        padded.extend(std::iter::repeat(blank_row.clone()).take(margin_rows as usize));
        padded.extend(trimmed);
        padded.extend(std::iter::repeat(blank_row).take(margin_rows as usize));

        (padded, margin_rows)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn default_renderer() -> Option<FontRenderer> {
        if !std::path::Path::new(DEFAULT_FONT_PATH).exists() {
            eprintln!("skipping test: {DEFAULT_FONT_PATH} not found");
            return None;
        }
        Some(FontRenderer::load(DEFAULT_FONT_PATH).unwrap())
    }

    #[test]
    fn max_size_shrinks_with_narrower_tape() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let max_6mm = renderer.max_size_pt_for_tape(TapeWidth::Mm6);
        let max_12mm = renderer.max_size_pt_for_tape(TapeWidth::Mm12);
        let max_18mm = renderer.max_size_pt_for_tape(TapeWidth::Mm18);
        assert!(max_6mm < max_12mm);
        assert!(max_12mm < max_18mm);
    }

    #[test]
    fn max_size_does_not_clip() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        for tape in [TapeWidth::Mm6, TapeWidth::Mm12, TapeWidth::Mm18] {
            let max = renderer.max_size_pt_for_tape(tape);
            assert!(!renderer.would_clip(max, tape));
            // A couple of points larger should clip.
            assert!(renderer.would_clip(max + 2.0, tape));
        }
    }

    #[test]
    fn would_clip_detects_oversized_36pt_on_6mm() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        // 36pt was captured to clip visibly on 12mm tape in real hardware
        // testing; the much narrower 6mm tape should clip it too.
        assert!(renderer.would_clip(36.0, TapeWidth::Mm6));
    }
}
