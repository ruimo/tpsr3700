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

/// Minimum height of a single printed line in millimeters.
/// A 6mm tape allows at most 2 lines (6 / 3 = 2).
const MIN_LINE_HEIGHT_MM: f32 = 3.0;

/// Text alignment for multi-line (and single-line) output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    /// Left-align each line (default).
    #[default]
    Left,
    /// Center each line within the tape length.
    Center,
    /// Right-align each line.
    Right,
}

impl std::str::FromStr for TextAlign {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "left" => Ok(TextAlign::Left),
            "center" => Ok(TextAlign::Center),
            "right" => Ok(TextAlign::Right),
            other => Err(format!("unknown alignment '{other}'; expected left, center, or right")),
        }
    }
}

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

    /// Returns the largest point size at which this font's line height fits
    /// within `bit_height` bits (used for multi-line rendering where each
    /// line occupies only a fraction of the tape height).
    fn max_size_pt_for_bits(&self, bit_height: usize) -> f32 {
        let ratio_per_px = self.line_height_px(1.0);
        let max_px = bit_height as f32 / ratio_per_px;
        px_to_pt(max_px)
    }

    /// Returns `true` if rendering at `size_pt` would clip the top or
    /// bottom of the text on `tape` (based on the font's overall vertical
    /// metrics, same caveat as [`FontRenderer::max_size_pt_for_tape`]).
    pub fn would_clip(&self, size_pt: f32, tape: TapeWidth) -> bool {
        let size_px = pt_to_px(size_pt);
        self.line_height_px(size_px) > tape.bitmap_bits() as f32
    }

    /// Wraps `text` into lines such that each line's advance width in dots
    /// does not exceed `max_width_dots`.  Wrapping is character-granular
    /// (suitable for label printing, where word-wrap is rarely meaningful).
    /// Explicit `\n` characters always force a line break.
    /// Returns the list of `&str` slices that constitute the wrapped lines.
    fn wrap_chars<'a>(&self, text: &'a str, size_px: f32, max_width_dots: f32) -> Vec<&'a str> {
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        if chars.is_empty() {
            return vec![""];
        }

        let mut lines: Vec<&'a str> = Vec::new();
        let mut line_start_idx = 0; // byte offset in `text`
        let mut line_start_char = 0; // char index
        let mut current_width = 0.0f32;

        for (ci, (byte_idx, ch)) in chars.iter().enumerate() {
            // Explicit newline: flush current line and start a new one.
            if *ch == '\n' {
                lines.push(&text[line_start_idx..*byte_idx]);
                line_start_idx = byte_idx + '\n'.len_utf8();
                line_start_char = ci + 1;
                current_width = 0.0;
                continue;
            }

            let (metrics, _) = self.font.rasterize(*ch, size_px);
            let advance = metrics.advance_width;

            if current_width + advance > max_width_dots && ci > line_start_char {
                // Flush the current line (up to but not including this char).
                let end_byte = *byte_idx;
                lines.push(&text[line_start_idx..end_byte]);
                line_start_idx = *byte_idx;
                line_start_char = ci;
                current_width = 0.0;
            }
            current_width += advance;
        }

        // Push the remaining text as the last line.
        lines.push(&text[line_start_idx..]);
        lines
    }

    /// Render one line of text into a column buffer (outer = feed col,
    /// inner = bit position across tape width), placed within a vertical
    /// band of `[bit_start, bit_start + band_bits)` within the full tape
    /// height (`total_bits`).
    ///
    /// Returns `(columns, total_advance_dots)`.
    fn render_line_columns(
        &self,
        text: &str,
        size_px: f32,
        total_bits: usize,
        bit_start: usize,
        band_bits: usize,
    ) -> (Vec<Vec<bool>>, f32) {
        let line_metrics = self.font.horizontal_line_metrics(size_px);
        let (ascent, descent) = match line_metrics {
            Some(m) => (m.ascent, m.descent),
            None => (size_px, 0.0),
        };
        let text_height = ascent - descent;
        // Vertically center within the band.
        let band_top_margin = (band_bits as f32 - text_height) / 2.0;

        let mut columns: Vec<Vec<bool>> = Vec::new();
        let mut cursor_x = 0.0f32;

        for ch in text.chars() {
            let (metrics, bitmap) = self.font.rasterize(ch, size_px);
            let glyph_width = metrics.width;
            let glyph_height = metrics.height;

            let start_col = cursor_x.round() as usize;
            while columns.len() < start_col + glyph_width.max(1) {
                columns.push(vec![false; total_bits]);
            }

            for gy in 0..glyph_height {
                for gx in 0..glyph_width {
                    let coverage = bitmap[gy * glyph_width + gx];
                    if coverage > 127 {
                        let glyph_top =
                            ascent - (metrics.ymin as f32 + glyph_height as f32);
                        // Row within the band (0-based, top of band = 0).
                        let band_row =
                            (band_top_margin + glyph_top + gy as f32).round() as isize;
                        // Absolute row within the full tape bitmap.
                        let abs_row = bit_start as isize + band_row;
                        if abs_row >= 0 && (abs_row as usize) < total_bits {
                            // Mirror vertically (same as single-line render).
                            let bit_pos = total_bits - 1 - abs_row as usize;
                            let col = start_col + gx;
                            columns[col][bit_pos] = true;
                        }
                    }
                }
            }

            cursor_x += metrics.advance_width;
        }

        (columns, cursor_x)
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

    /// Multi-line variant of [`render_text`].
    ///
    /// `tape_length_mm` is the total label length including left and right
    /// feed margins.  If the text does not fit in one line within that
    /// length, it is wrapped character-by-character onto additional lines.
    /// The maximum number of lines is determined by `tape.width_mm()` and
    /// [`MIN_LINE_HEIGHT_MM`] (e.g. 6mm tape → 2 lines max).
    ///
    /// `size_pt` controls the font size. If `None`, the largest size that
    /// fills one line-band is chosen automatically.
    ///
    /// `align` controls horizontal alignment of each line within the label
    /// length (left/center/right).
    ///
    /// Returns `(rows, margin_rows)` in the same format as [`render_text`].
    pub fn render_text_multiline(
        &self,
        text: &str,
        size_pt: Option<f32>,
        tape: TapeWidth,
        feed_margin_mm: f32,
        tape_length_mm: f32,
        align: TextAlign,
    ) -> (Vec<Vec<u8>>, u32) {
        let total_bits = tape.bitmap_bits();
        let margin_rows = mm_to_rows(feed_margin_mm);
        // Content length (dots) = total tape length minus both feed margins.
        let content_length_dots =
            mm_to_rows(tape_length_mm) as f32 - 2.0 * margin_rows as f32;
        let content_length_dots = content_length_dots.max(1.0);

        // Maximum number of lines that fit on this tape.
        let max_lines = (tape.width_mm() as f32 / MIN_LINE_HEIGHT_MM).floor() as usize;
        let max_lines = max_lines.max(1);

        // Each line band has an equal share of the tape's bit depth.
        // We try 1 line first; if the text doesn't fit, increase line count
        // up to max_lines.
        let size_px;
        let num_lines;
        let wrapped_lines: Vec<&str>;

        // Find the minimum number of lines needed.
        let mut found = false;
        let mut nl = 1usize;
        let mut sz = 0.0f32;
        let mut wl: Vec<&str> = Vec::new();

        while nl <= max_lines {
            let band_bits = total_bits / nl;
            let candidate_pt = match size_pt {
                Some(pt) => pt,
                None => self.max_size_pt_for_bits(band_bits),
            };
            let candidate_px = pt_to_px(candidate_pt);
            // Wrap with this size.
            let candidate_lines = self.wrap_chars(text, candidate_px, content_length_dots);
            if candidate_lines.len() <= nl {
                sz = candidate_pt;
                wl = candidate_lines;
                found = true;
                break;
            }
            nl += 1;
        }

        if !found {
            // Even with max_lines the text overflows; use max_lines and let
            // it truncate naturally (last line may be too long but we don't
            // truncate the content — the caller is responsible for checking).
            nl = max_lines;
            let band_bits = total_bits / nl;
            sz = match size_pt {
                Some(pt) => pt,
                None => self.max_size_pt_for_bits(band_bits),
            };
            let candidate_px = pt_to_px(sz);
            wl = self.wrap_chars(text, candidate_px, content_length_dots);
        }

        num_lines = nl;
        size_px = pt_to_px(sz);
        wrapped_lines = wl;

        // Render each wrapped line into the appropriate vertical band.
        let band_bits = total_bits / num_lines;
        let content_dots = content_length_dots as usize;

        // Build a merged column buffer (outer = feed col 0..content_dots,
        // inner = bit position across tape width 0..total_bits).
        let mut merged: Vec<Vec<bool>> = vec![vec![false; total_bits]; content_dots];

        for (line_idx, &line_text) in wrapped_lines.iter().enumerate() {
            if line_idx >= num_lines {
                break;
            }
            let bit_start = line_idx * band_bits;

            let (line_columns, line_width) =
                self.render_line_columns(line_text, size_px, total_bits, bit_start, band_bits);

            // Compute horizontal offset based on alignment.
            let col_count = line_columns.len();
            let line_width_dots = line_width.round() as usize;
            let offset = match align {
                TextAlign::Left => 0,
                TextAlign::Center => {
                    if content_dots > line_width_dots {
                        (content_dots - line_width_dots) / 2
                    } else {
                        0
                    }
                }
                TextAlign::Right => {
                    if content_dots > line_width_dots {
                        content_dots - line_width_dots
                    } else {
                        0
                    }
                }
            };

            for (ci, col_bits) in line_columns.iter().enumerate() {
                let dest_col = offset + ci;
                if dest_col < content_dots {
                    for (bi, &bit) in col_bits.iter().enumerate() {
                        if bit {
                            merged[dest_col][bi] = true;
                        }
                    }
                }
            }
            let _ = col_count; // suppress unused warning
        }

        // Pack merged columns into raster rows.
        let content_rows: Vec<Vec<u8>> = merged
            .iter()
            .map(|col| pack_bits(col, tape.bitmap_bytes()))
            .collect();

        // Trim leading/trailing blank columns.
        let first_nonblank = content_rows.iter().position(|r| r.iter().any(|&b| b != 0));
        let last_nonblank = content_rows.iter().rposition(|r| r.iter().any(|&b| b != 0));

        let trimmed = match (first_nonblank, last_nonblank) {
            (Some(first), Some(last)) => content_rows[first..=last].to_vec(),
            _ => Vec::new(),
        };

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

    #[test]
    fn text_align_from_str() {
        assert_eq!("left".parse::<TextAlign>().unwrap(), TextAlign::Left);
        assert_eq!("center".parse::<TextAlign>().unwrap(), TextAlign::Center);
        assert_eq!("right".parse::<TextAlign>().unwrap(), TextAlign::Right);
        assert!("other".parse::<TextAlign>().is_err());
    }

    #[test]
    fn wrap_chars_fits_single_line() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let size_px = pt_to_px(12.0);
        // A very wide max width should give a single line.
        let lines = renderer.wrap_chars("ABC", size_px, 10000.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "ABC");
    }

    #[test]
    fn wrap_chars_wraps_when_narrow() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let size_px = pt_to_px(12.0);
        // Force each character onto its own line by using width = 1.
        let lines = renderer.wrap_chars("ABC", size_px, 1.0);
        // Should produce at least 2 lines for "ABC".
        assert!(lines.len() >= 2);
    }

    #[test]
    fn wrap_chars_respects_explicit_newline() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let size_px = pt_to_px(12.0);
        // Even with a very wide max width, \n must split the string.
        let lines = renderer.wrap_chars("AB\nCD", size_px, 10000.0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "AB");
        assert_eq!(lines[1], "CD");
    }

    #[test]
    fn wrap_chars_newline_at_start_and_end() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let size_px = pt_to_px(12.0);
        let lines = renderer.wrap_chars("\nAB\n", size_px, 10000.0);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "");
        assert_eq!(lines[1], "AB");
        assert_eq!(lines[2], "");
    }

    #[test]
    fn render_multiline_produces_rows() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let (rows, margin) = renderer.render_text_multiline(
            "Hello World",
            None,
            TapeWidth::Mm18,
            DEFAULT_FEED_MARGIN_MM,
            50.0,
            TextAlign::Left,
        );
        assert!(!rows.is_empty());
        assert!(margin > 0);
    }
}
