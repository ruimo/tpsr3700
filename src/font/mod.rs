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
    /// (ascent - descent) fits within `tape`'s usable dot range.
    /// Used for the `--size` warning and single-line mode default size.
    ///
    /// `top_margin_bits` / `bottom_margin_bits` are the number of non-printing
    /// dots at the respective tape edge (default: [`TapeWidth::HEAD_MARGIN_BITS`]
    /// each).
    pub fn max_size_pt_for_tape(
        &self,
        tape: TapeWidth,
        top_margin_bits: usize,
        bottom_margin_bits: usize,
    ) -> f32 {
        let ratio_per_px = self.line_height_px(1.0);
        let head_bits = tape
            .bitmap_bits()
            .saturating_sub(top_margin_bits + bottom_margin_bits)
            .max(1);
        let max_px = head_bits as f32 / ratio_per_px;
        px_to_pt(max_px)
    }

    /// Returns `true` if rendering at `size_pt` would clip the top or
    /// bottom of the text on `tape` (based on the font's overall vertical
    /// metrics).
    ///
    /// `top_margin_bits` / `bottom_margin_bits` are the number of non-printing
    /// dots at the respective tape edge (default: [`TapeWidth::HEAD_MARGIN_BITS`]
    /// each).
    pub fn would_clip(
        &self,
        size_pt: f32,
        tape: TapeWidth,
        top_margin_bits: usize,
        bottom_margin_bits: usize,
    ) -> bool {
        let size_px = pt_to_px(size_pt);
        let head_bits = tape
            .bitmap_bits()
            .saturating_sub(top_margin_bits + bottom_margin_bits)
            .max(1);
        self.line_height_px(size_px) > head_bits as f32
    }

    /// Render one line of text into a framebuffer using font metrics.
    ///
    /// The framebuffer uses a logical coordinate system where row 0 is the
    /// top of the line.  Glyph placement follows the font's `ascent` so all
    /// characters share a common baseline.  The framebuffer height is
    /// `ascent - descent` (i.e. `line_height_px`); glyphs are not clipped.
    ///
    /// Returns `(columns, advance_width)` where `columns[x][y]` is true if
    /// dot (x, y) is inked.  `y=0` is the topmost row.
    fn rasterize_line(&self, text: &str, size_px: f32) -> (Vec<Vec<bool>>, f32) {
        let line_metrics = self.font.horizontal_line_metrics(size_px);
        let ascent = match line_metrics {
            Some(m) => m.ascent,
            None => size_px,
        };
        let line_height = self.line_height_px(size_px).ceil() as usize;

        let mut columns: Vec<Vec<bool>> = Vec::new();
        let mut cursor_x = 0.0f32;

        for ch in text.chars() {
            let (metrics, bitmap) = self.font.rasterize(ch, size_px);
            let glyph_width = metrics.width;
            let glyph_height = metrics.height;

            let start_col = cursor_x.round() as usize;
            while columns.len() < start_col + glyph_width.max(1) {
                columns.push(vec![false; line_height]);
            }

            for gy in 0..glyph_height {
                for gx in 0..glyph_width {
                    let coverage = bitmap[gy * glyph_width + gx];
                    if coverage > 127 {
                        let glyph_top =
                            ascent - (metrics.ymin as f32 + glyph_height as f32);
                        let row = (glyph_top + gy as f32).round() as isize;
                        if row >= 0 && (row as usize) < line_height {
                            let col = start_col + gx;
                            columns[col][row as usize] = true;
                        }
                    }
                }
            }
            cursor_x += metrics.advance_width;
        }

        (columns, cursor_x)
    }

    /// Returns the topmost inked row index across all columns, or 0 if no ink.
    fn ink_top(columns: &[Vec<bool>]) -> usize {
        let mut top: Option<usize> = None;
        for col in columns {
            for (y, &ink) in col.iter().enumerate() {
                if ink {
                    top = Some(top.map_or(y, |v| v.min(y)));
                    break; // only need the topmost in this column
                }
            }
        }
        top.unwrap_or(0)
    }

    /// Returns the span from the topmost to bottommost inked row (inclusive).
    /// Returns 0 if there is no ink.
    fn ink_height(columns: &[Vec<bool>]) -> usize {
        let mut top: Option<usize> = None;
        let mut bot: Option<usize> = None;
        for col in columns {
            for (y, &ink) in col.iter().enumerate() {
                if ink {
                    top = Some(top.map_or(y, |v| v.min(y)));
                    bot = Some(bot.map_or(y, |v| v.max(y)));
                }
            }
        }
        match (top, bot) {
            (Some(t), Some(b)) => b - t + 1,
            _ => 0,
        }
    }

    /// Place a pre-rasterized framebuffer into the tape bitmap.
    ///
    /// `bit_start_signed` is the framebuffer row-0 position within the
    /// printable area (may be negative when centering pushes the top up).
    /// Only rows that fall within `[band_start, band_start + band_head_bits)`
    /// are written, so ink never bleeds into adjacent bands.
    fn place_framebuf_at(
        framebuf: &[Vec<bool>],
        merged: &mut Vec<Vec<bool>>,
        total_bits: usize,
        head_offset: usize,
        head_bits: usize,
        bit_start_signed: isize,
        band_start: usize,
        band_head_bits: usize,
        h_offset: usize,
        content_dots: usize,
    ) {
        let band_end = band_start + band_head_bits;
        for (ci, col_bits) in framebuf.iter().enumerate() {
            let dest_col = h_offset + ci;
            if dest_col >= content_dots {
                break;
            }
            for (row, &ink) in col_bits.iter().enumerate() {
                if !ink {
                    continue;
                }
                let abs_head_row = bit_start_signed + row as isize;
                if abs_head_row < 0 {
                    continue;
                }
                let abs_head_row = abs_head_row as usize;
                // Clamp to this line's band.
                if abs_head_row < band_start || abs_head_row >= band_end {
                    continue;
                }
                if abs_head_row >= head_bits {
                    continue;
                }
                // Mirror within printable area, then offset into bitmap.
                let bit_pos = head_offset + (head_bits - 1 - abs_head_row);
                if bit_pos < total_bits {
                    merged[dest_col][bit_pos] = true;
                }
            }
        }
    }

    /// Render `text` at the given point size (matching the size shown in
    /// the TEPRA desktop app, e.g. 36pt) into a sequence of TEPRA raster
    /// rows for the given tape width, with `left_margin_mm` / `right_margin_mm`
    /// millimeters of blank tape feed added before and after the printed
    /// content (see [`DEFAULT_FEED_MARGIN_MM`]).
    ///
    /// Returns `(rows, margin_rows)` where `margin_rows` is the number of
    /// leading (left) blank rows (matching the `T` command semantics).
    pub fn render_text(
        &self,
        text: &str,
        size_pt: f32,
        tape: TapeWidth,
        left_margin_mm: f32,
        right_margin_mm: f32,
        top_margin_bits: usize,
        bottom_margin_bits: usize,
    ) -> (Vec<Vec<u8>>, u32) {
        let size_px = pt_to_px(size_pt);
        let bit_depth = tape.bitmap_bits();
        let head_bits = tape
            .bitmap_bits()
            .saturating_sub(top_margin_bits + bottom_margin_bits)
            .max(1);
        let head_offset = bottom_margin_bits;

        // Render each character to a coverage bitmap and place it into a
        // column buffer: outer index = column (x), inner = row (y, i.e.
        // dot position across the tape width).
        let mut columns: Vec<Vec<bool>> = Vec::new();

        // Align glyphs to the top of the printable area, using the font's
        // overall ascent extent (not per-glyph metrics), so that all
        // characters in a string sit on a common baseline.
        let line_metrics = self.font.horizontal_line_metrics(size_px);
        let ascent = match line_metrics {
            Some(m) => m.ascent,
            None => size_px,
        };

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
                        // baseline. Convert to a row within the printable area.
                        let glyph_top = ascent - (metrics.ymin as f32 + glyph_height as f32);
                        let row = (glyph_top + gy as f32).round() as isize;
                        // Clip to printable area.
                        if row < 0 || row as usize >= head_bits {
                            continue;
                        }
                        // Mirror within printable area, then offset into bitmap.
                        // Confirmed against real-hardware printouts of "AB" on
                        // 6mm tape.
                        let bit_pos = head_offset + (head_bits - 1 - row as usize);
                        if bit_pos < bit_depth {
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

        let left_rows = mm_to_rows(left_margin_mm);
        let right_rows = mm_to_rows(right_margin_mm);
        let blank_row = vec![0u8; tape.bitmap_bytes()];

        let mut padded = Vec::with_capacity(trimmed.len() + left_rows as usize + right_rows as usize);
        padded.extend(std::iter::repeat(blank_row.clone()).take(left_rows as usize));
        padded.extend(trimmed);
        padded.extend(std::iter::repeat(blank_row).take(right_rows as usize));

        (padded, left_rows)
    }

    /// Multi-line variant of [`render_text`].
    ///
    /// Algorithm:
    /// 1. Count `num_lines` from explicit `\n` separators (≥ 1).
    ///    If `num_lines > max_lines`, clamp to `max_lines`.
    /// 2. `band_head_bits = head_bits / num_lines` — fixed for the whole job.
    /// 3. For each segment (in order):
    ///    a. Rasterize at a provisional size and split at `content_length_dots`.
    ///       Overflow is prepended back to the pending queue (same band).
    ///    b. Rasterize the fitting text at the provisional size, measure the
    ///       actual ink height (`ink_h`) from the bitmap.
    ///    c. Scale: `final_px = provisional_px * band_head_bits / ink_h`.
    ///    d. Re-check width at `final_px`.  If overflow, prepend to pending
    ///       and repeat (b–d) until stable.
    ///    e. If pending now has more segments than remaining bands, and
    ///       `num_lines < max_lines`, restart with `num_lines + 1`.
    ///       Otherwise discard excess segments.
    /// 4. Render each confirmed line into the tape bitmap, centering the ink
    ///    vertically within its band.
    pub fn render_text_multiline(
        &self,
        text: &str,
        size_pt: Option<f32>,
        tape: TapeWidth,
        left_margin_mm: f32,
        right_margin_mm: f32,
        tape_length_mm: f32,
        align: TextAlign,
        top_margin_bits: usize,
        bottom_margin_bits: usize,
    ) -> (Vec<Vec<u8>>, u32) {
        let total_bits = tape.bitmap_bits();
        let head_bits = tape
            .bitmap_bits()
            .saturating_sub(top_margin_bits + bottom_margin_bits)
            .max(1);
        let head_offset = bottom_margin_bits;
        let left_rows = mm_to_rows(left_margin_mm);
        let right_rows = mm_to_rows(right_margin_mm);
        let content_length_dots =
            (mm_to_rows(tape_length_mm) as f32 - left_rows as f32 - right_rows as f32).max(1.0);
        let content_dots = content_length_dots as usize;

        let max_lines = (tape.width_mm() as f32 / MIN_LINE_HEIGHT_MM).floor() as usize;
        let max_lines = max_lines.max(1);

        // Step 1: split on explicit \n.  num_lines is fixed by the \n count,
        // but may grow (up to max_lines) if wrap overflow exceeds remaining bands.
        let initial_segments: Vec<String> = text
            .split('\n')
            .map(|s| s.to_string())
            .collect();

        let mut num_lines = initial_segments.len().min(max_lines).max(1);

        // Outer retry loop: re-run with num_lines+1 when overflow can't be absorbed.
        let confirmed: Vec<(String, f32)> = 'retry: loop {
            let band_head_bits = head_bits / num_lines;

            // provisional_px: size such that font's ascent–descent fills one band.
            let provisional_px = match size_pt {
                Some(pt) => pt_to_px(pt),
                None => {
                    let ratio = self.line_height_px(1.0);
                    band_head_bits as f32 / ratio
                }
            };

            let mut pending: std::collections::VecDeque<String> =
                initial_segments.iter().cloned().collect();
            let mut confirmed: Vec<(String, f32)> = Vec::new();

            'band: loop {
                let Some(segment) = pending.pop_front() else { break 'band };
                let bands_left = num_lines - confirmed.len();

                // Step 3a: split at content width using provisional size.
                let (mut fit_text, mut overflow) =
                    self.split_at_width(&segment, provisional_px, content_length_dots);

                // Step 3b–d: measure actual ink height, scale, re-check width.
                let final_px = loop {
                    let (fb, _) = self.rasterize_line(&fit_text, provisional_px);
                    let ink_h = Self::ink_height(&fb);
                    let scaled_px = if ink_h > 0 {
                        provisional_px * band_head_bits as f32 / ink_h as f32
                    } else {
                        provisional_px
                    };

                    let (refit, new_ov) =
                        self.split_at_width(&fit_text, scaled_px, content_length_dots);
                    if refit == fit_text {
                        break scaled_px;
                    }
                    // scaled_px caused new overflow — merge and retry with shorter text.
                    overflow = if overflow.is_empty() {
                        new_ov
                    } else {
                        format!("{}{}", new_ov, overflow)
                    };
                    fit_text = refit;
                };

                // Prepend leftover back to pending.
                if !overflow.is_empty() {
                    pending.push_front(overflow);
                }

                confirmed.push((fit_text, final_px));

                // Step 3e: check if pending overflows remaining bands.
                let remaining_bands = bands_left - 1; // we just used one
                if pending.len() > remaining_bands {
                    if num_lines < max_lines {
                        // Restart with one more line.
                        num_lines += 1;
                        continue 'retry;
                    } else {
                        // Can't grow further — discard overflow.
                        break 'band;
                    }
                }

                if confirmed.len() >= num_lines {
                    break 'band;
                }
            }

            break 'retry confirmed;
        };

        // Step 4: render confirmed lines into tape bitmap.
        let num_confirmed = confirmed.len().max(1);
        let band_head_bits = head_bits / num_confirmed;

        let mut merged: Vec<Vec<bool>> = vec![vec![false; total_bits]; content_dots];

        for (line_idx, (line_text, final_px)) in confirmed.iter().enumerate() {
            let band_start = line_idx * band_head_bits;

            let (fb, advance) = self.rasterize_line(line_text, *final_px);

            // Center ink vertically within the band.
            let ink_h = Self::ink_height(&fb);
            let ink_top = Self::ink_top(&fb);
            let v_offset = if ink_h < band_head_bits {
                (band_head_bits - ink_h) / 2
            } else {
                0
            };
            // bit_start: where row 0 of this band lands, adjusted so that
            // the ink block is centred.  We subtract ink_top so the
            // framebuffer's logical row 0 maps to (band_start + v_offset - ink_top).
            let bit_start_signed =
                band_start as isize + v_offset as isize - ink_top as isize;

            let line_width_dots = advance.round() as usize;
            let h_offset = match align {
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

            Self::place_framebuf_at(
                &fb,
                &mut merged,
                total_bits,
                head_offset,
                head_bits,
                bit_start_signed,
                band_start,
                band_head_bits,
                h_offset,
                content_dots,
            );
        }

        // Pack, trim, add margins.
        let content_rows: Vec<Vec<u8>> = merged
            .iter()
            .map(|col| pack_bits(col, tape.bitmap_bytes()))
            .collect();

        let first_nonblank = content_rows.iter().position(|r| r.iter().any(|&b| b != 0));
        let last_nonblank = content_rows.iter().rposition(|r| r.iter().any(|&b| b != 0));
        let trimmed = match (first_nonblank, last_nonblank) {
            (Some(f), Some(l)) => content_rows[f..=l].to_vec(),
            _ => Vec::new(),
        };

        let blank_row = vec![0u8; tape.bitmap_bytes()];
        let mut padded = Vec::with_capacity(trimmed.len() + left_rows as usize + right_rows as usize);
        padded.extend(std::iter::repeat(blank_row.clone()).take(left_rows as usize));
        padded.extend(trimmed);
        padded.extend(std::iter::repeat(blank_row).take(right_rows as usize));

        (padded, left_rows)
    }

    /// Split `text` at the first character that would cause the cumulative
    /// advance width to exceed `max_width` when rasterized at `size_px`.
    /// Returns `(fit, overflow)` where `fit` is the prefix that fits and
    /// `overflow` is the remainder (may be empty).
    fn split_at_width(&self, text: &str, size_px: f32, max_width: f32) -> (String, String) {
        let mut width = 0.0f32;
        let mut split_byte = text.len(); // default: everything fits
        for (byte_idx, ch) in text.char_indices() {
            let (metrics, _) = self.font.rasterize(ch, size_px);
            if width + metrics.advance_width > max_width && byte_idx > 0 {
                split_byte = byte_idx;
                break;
            }
            width += metrics.advance_width;
        }
        (text[..split_byte].to_string(), text[split_byte..].to_string())
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

/// Converts raster rows into an ASCII-art string for snapshot testing.
/// `#`=ink  `.`=blank  `|`=non-printing margin boundary at `head_offset`
/// and `head_offset + head_bits`.
/// Feed direction is downsampled to at most `max_rows` lines.
#[cfg(test)]
fn rows_to_ascii(
    rows: &[Vec<u8>],
    total_bits: usize,
    head_offset: usize,
    head_bits: usize,
    max_rows: usize,
) -> String {
    let step = (rows.len() / max_rows).max(1);
    rows.iter()
        .enumerate()
        .filter(|(i, _)| i % step == 0)
        .map(|(_, row)| {
            (0..total_bits)
                .map(|bit| {
                    if bit == head_offset || bit == head_offset + head_bits {
                        '|'
                    } else {
                        let byte_idx = bit / 8;
                        let bit_idx = 7 - (bit % 8);
                        if byte_idx < row.len() && (row[byte_idx] >> bit_idx) & 1 == 1 {
                            '#'
                        } else {
                            '.'
                        }
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
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
        let max_6mm = renderer.max_size_pt_for_tape(TapeWidth::Mm6, TapeWidth::HEAD_MARGIN_BITS, TapeWidth::HEAD_MARGIN_BITS);
        let max_12mm = renderer.max_size_pt_for_tape(TapeWidth::Mm12, TapeWidth::HEAD_MARGIN_BITS, TapeWidth::HEAD_MARGIN_BITS);
        let max_18mm = renderer.max_size_pt_for_tape(TapeWidth::Mm18, TapeWidth::HEAD_MARGIN_BITS, TapeWidth::HEAD_MARGIN_BITS);
        assert!(max_6mm < max_12mm);
        assert!(max_12mm < max_18mm);
    }

    #[test]
    fn max_size_does_not_clip() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        for tape in [TapeWidth::Mm6, TapeWidth::Mm12, TapeWidth::Mm18] {
            let max = renderer.max_size_pt_for_tape(tape, TapeWidth::HEAD_MARGIN_BITS, TapeWidth::HEAD_MARGIN_BITS);
            assert!(!renderer.would_clip(max, tape, TapeWidth::HEAD_MARGIN_BITS, TapeWidth::HEAD_MARGIN_BITS));
            // A couple of points larger should clip.
            assert!(renderer.would_clip(max + 2.0, tape, TapeWidth::HEAD_MARGIN_BITS, TapeWidth::HEAD_MARGIN_BITS));
        }
    }

    #[test]
    fn would_clip_detects_oversized_36pt_on_6mm() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        // 36pt was captured to clip visibly on 12mm tape in real hardware
        // testing; the much narrower 6mm tape should clip it too.
        assert!(renderer.would_clip(36.0, TapeWidth::Mm6, TapeWidth::HEAD_MARGIN_BITS, TapeWidth::HEAD_MARGIN_BITS));
    }

    #[test]
    fn text_align_from_str() {
        assert_eq!("left".parse::<TextAlign>().unwrap(), TextAlign::Left);
        assert_eq!("center".parse::<TextAlign>().unwrap(), TextAlign::Center);
        assert_eq!("right".parse::<TextAlign>().unwrap(), TextAlign::Right);
        assert!("other".parse::<TextAlign>().is_err());
    }

    #[test]
    fn split_at_width_fits_single_line() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let size_px = pt_to_px(12.0);
        // A very wide max width: everything fits, overflow is empty.
        let (fit, overflow) = renderer.split_at_width("ABC", size_px, 10000.0);
        assert_eq!(fit, "ABC");
        assert_eq!(overflow, "");
    }

    #[test]
    fn split_at_width_wraps_when_narrow() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let size_px = pt_to_px(12.0);
        // Width=1: only the first char (or nothing extra) fits.
        let (fit, overflow) = renderer.split_at_width("ABC", size_px, 1.0);
        assert!(!overflow.is_empty(), "expected overflow for narrow width");
        assert!(fit.len() < "ABC".len());
    }

    #[test]
    fn split_at_width_explicit_newline_handled_by_caller() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let size_px = pt_to_px(12.0);
        // \n splitting is done before split_at_width (by render_text_multiline).
        // split_at_width sees each segment without \n.
        let segments: Vec<&str> = "AB\nCD".split('\n').collect();
        assert_eq!(segments.len(), 2);
        let (fit0, ov0) = renderer.split_at_width(segments[0], size_px, 10000.0);
        let (fit1, ov1) = renderer.split_at_width(segments[1], size_px, 10000.0);
        assert_eq!(fit0, "AB");
        assert_eq!(ov0, "");
        assert_eq!(fit1, "CD");
        assert_eq!(ov1, "");
    }

    #[test]
    fn split_at_width_empty_segment() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let size_px = pt_to_px(12.0);
        let (fit, overflow) = renderer.split_at_width("", size_px, 10000.0);
        assert_eq!(fit, "");
        assert_eq!(overflow, "");
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
            DEFAULT_FEED_MARGIN_MM,
            50.0,
            TextAlign::Left,
            TapeWidth::HEAD_MARGIN_BITS,
            TapeWidth::HEAD_MARGIN_BITS,
        );
        assert!(!rows.is_empty());
        assert!(margin > 0);
    }

    /// Asserts that the ASCII-art rendering of `rows` matches `expected`.
    /// On failure prints both expected and actual art.
    #[track_caller]
    fn assert_ascii_art(rows: &[Vec<u8>], tape: TapeWidth, max_cols: usize, expected: &str) {
        let actual = rows_to_ascii(rows, tape.bitmap_bits(), tape.head_offset(), tape.head_bits(), max_cols);
        assert_eq!(
            actual, expected,
            "\nASCII art mismatch.\nExpected:\n{expected}\nActual:\n{actual}"
        );
    }

    /// Returns true if any bit in `[bit_start, bit_end)` is set in any row.
    fn any_ink_in_bit_range(rows: &[Vec<u8>], bit_start: usize, bit_end: usize) -> bool {
        rows.iter().any(|row| {
            (bit_start..bit_end).any(|bit| {
                let byte_idx = bit / 8;
                let bit_idx = 7 - (bit % 8);
                byte_idx < row.len() && (row[byte_idx] >> bit_idx) & 1 == 1
            })
        })
    }

    #[test]
    fn multiline_uses_both_bands_when_explicit_newline() {
        // With an explicit \n the text is split into two lines.
        // Line 0 occupies the first half of the printable area (bit_start=0),
        // line 1 occupies the second half (bit_start=head_bits/2).
        // Both halves should contain ink.
        let Some(renderer) = default_renderer() else {
            return;
        };
        for tape in [TapeWidth::Mm12, TapeWidth::Mm18] {
            let (rows, _margin) = renderer.render_text_multiline(
                "D39M-06N\n420V20A",
                None,
                tape,
                DEFAULT_FEED_MARGIN_MM,
                DEFAULT_FEED_MARGIN_MM,
                32.0,
                TextAlign::Left,
                TapeWidth::HEAD_MARGIN_BITS,
                TapeWidth::HEAD_MARGIN_BITS,
            );
            let ho = tape.head_offset();
            let hb = tape.head_bits();
            let half = hb / 2;
            // Line 0: bit_start=0, mirrored into [ho + half .. ho + hb)
            assert!(
                any_ink_in_bit_range(&rows, ho + half, ho + hb),
                "expected ink in first-line band on {}mm tape",
                tape.width_mm()
            );
            // Line 1: bit_start=half, mirrored into [ho .. ho + half)
            assert!(
                any_ink_in_bit_range(&rows, ho, ho + half),
                "expected ink in second-line band on {}mm tape",
                tape.width_mm()
            );
        }
    }

    /// Snapshot test: `--tape-length-mm 32 "D39M-06N 420V20A"` on 6mm tape.
    ///
    /// The auto-size logic picks nl=2 (half-height font) because that yields a
    /// larger point size than nl=1 for this text/length combination.  At that
    /// smaller font size the whole text fits on a single line, so wrap_chars
    /// returns 1 line and render_text_multiline renders it as one line using
    /// the full tape height.  This test pins that behaviour.
    #[test]
    fn multiline_6mm_tape_length_32_fits_single_line() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let text = "D39M-06N 420V20A";
        let tape = TapeWidth::Mm6;
        let tape_length_mm = 32.0;

        let (rows, _margin) = renderer.render_text_multiline(
            text,
            None,
            tape,
            DEFAULT_FEED_MARGIN_MM,
            DEFAULT_FEED_MARGIN_MM,
            tape_length_mm,
            TextAlign::Left,
            TapeWidth::HEAD_MARGIN_BITS,
            TapeWidth::HEAD_MARGIN_BITS,
        );

        // Output must be non-empty and contain ink somewhere.
        assert!(!rows.is_empty());
        let total_bits = tape.bitmap_bits();
        assert!(any_ink_in_bit_range(&rows, 0, total_bits));
    }

    /// Regression test: `"D39pb"` on 6mm tape.
    ///
    /// `p` and `b` have descenders that extend below the baseline.
    /// Verifies that:
    /// 1. Output is non-empty and contains ink.
    /// 2. Ink is present in both the upper half (ascenders: D, 3, 9) and
    ///    the lower half (descenders: p, b) of the printable area.
    ///    The printable area spans [head_offset .. head_offset + head_bits).
    ///    With top-aligned rendering and the font's ascent placing the
    ///    baseline roughly 70% down, ascenders land in the upper portion
    ///    and descenders extend into the lower portion.
    #[test]
    fn render_6mm_descender_text_not_clipped() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let tape = TapeWidth::Mm6;
        let (rows, _margin) = renderer.render_text_multiline(
            "D39pb",
            None,
            tape,
            DEFAULT_FEED_MARGIN_MM,
            DEFAULT_FEED_MARGIN_MM,
            32.0,
            TextAlign::Left,
            TapeWidth::HEAD_MARGIN_BITS,
            TapeWidth::HEAD_MARGIN_BITS,
        );

        assert!(!rows.is_empty(), "expected non-empty output");

        assert_ascii_art(&rows, tape, 120, "\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|........#############################..|...........\n\
............|........#############################..|...........\n\
............|........#############################..|...........\n\
............|........#############################..|...........\n\
............|........###.......................###..|...........\n\
............|........###.......................###..|...........\n\
............|........###.......................###..|...........\n\
............|........###.......................###..|...........\n\
............|........###.......................###..|...........\n\
............|........###.......................###..|...........\n\
............|........####.....................####..|...........\n\
............|.........###.....................###...|...........\n\
............|.........###.....................###...|...........\n\
............|.........####...................####...|...........\n\
............|..........####.................####....|...........\n\
............|..........#####...............#####....|...........\n\
............|...........#######.........#######.....|...........\n\
............|............#####################......|...........\n\
............|.............###################.......|...........\n\
............|...............###############.........|...........\n\
............|...................#######.............|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|...........#...........................|...........\n\
............|..........###..........................|...........\n\
............|..........###...................###....|...........\n\
............|.........###....................####...|...........\n\
............|........####.....................###...|...........\n\
............|........###......................####..|...........\n\
............|........###...........##..........###..|...........\n\
............|........###...........##..........###..|...........\n\
............|........###...........###.........###..|...........\n\
............|........###..........####.........###..|...........\n\
............|........###..........####.........###..|...........\n\
............|........###..........#####........###..|...........\n\
............|........###.........######.......####..|...........\n\
............|........####........###.####....#####..|...........\n\
............|.........####.....####..############...|...........\n\
............|.........#############...##########....|...........\n\
............|..........###########.....########.....|...........\n\
............|...........#########........####.......|...........\n\
............|..............###......................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|........................########.......|...........\n\
............|.........###..........############.....|...........\n\
............|.........###.........##############....|...........\n\
............|........####........#####......#####...|...........\n\
............|........###.........###..........####..|...........\n\
............|........###........###............###..|...........\n\
............|........###........###............###..|...........\n\
............|........###........###.............##..|...........\n\
............|........###........###.............##..|...........\n\
............|........###........###............###..|...........\n\
............|........####........##............###..|...........\n\
............|.........###........###..........####..|...........\n\
............|.........#####.......##.........####...|...........\n\
............|..........######......###.....######...|...........\n\
............|...........########################....|...........\n\
............|.............####################......|...........\n\
............|...............################........|...........\n\
............|...................########............|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|#############################..........|...........\n\
............|#############################..........|...........\n\
............|#############################..........|...........\n\
............|..........###...........###............|...........\n\
............|.........###.............###...........|...........\n\
............|........###...............###..........|...........\n\
............|........###...............###..........|...........\n\
............|........###................###.........|...........\n\
............|........###................###.........|...........\n\
............|........###................###.........|...........\n\
............|........###................###.........|...........\n\
............|........###...............####.........|...........\n\
............|........####..............####.........|...........\n\
............|.........#####..........#####..........|...........\n\
............|..........#######....########..........|...........\n\
............|...........#################...........|...........\n\
............|............##############.............|...........\n\
............|..............##########...............|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|........###############################|...........\n\
............|........###############################|...........\n\
............|........###############################|...........\n\
............|..........###...........###............|...........\n\
............|.........###.............###...........|...........\n\
............|........###...............###..........|...........\n\
............|........###...............###..........|...........\n\
............|........###................###.........|...........\n\
............|........###................###.........|...........\n\
............|........###................###.........|...........\n\
............|........###................###.........|...........\n\
............|........###...............####.........|...........\n\
............|........####..............####.........|...........\n\
............|.........#####..........#####..........|...........\n\
............|..........#######....########..........|...........\n\
............|...........#################...........|...........\n\
............|............##############.............|...........\n\
............|..............##########...............|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........");
    }

    /// Snapshot test: `"Dpb\nFpb"` on 6mm tape (2 lines, all with descenders).
    #[test]
    fn render_6mm_two_line_descenders() {
        let Some(renderer) = default_renderer() else {
            return;
        };
        let tape = TapeWidth::Mm6;
        let (rows, _margin) = renderer.render_text_multiline(
            "Dpb\nFpb",
            None,
            tape,
            DEFAULT_FEED_MARGIN_MM,
            DEFAULT_FEED_MARGIN_MM,
            32.0,
            TextAlign::Left,
            TapeWidth::HEAD_MARGIN_BITS,
            TapeWidth::HEAD_MARGIN_BITS,
        );

        assert!(!rows.is_empty(), "expected non-empty output");

        assert_ascii_art(&rows, tape, 120, "\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|...##############......##############..|...........\n\
............|...##############......##############..|...........\n\
............|.........##.....#......#............#..|...........\n\
............|.........##.....#......##...........#..|...........\n\
............|.........##.....#......##...........#..|...........\n\
............|.........##.....#......##...........#..|...........\n\
............|.........##.....#......##..........##..|...........\n\
............|................#.......##........##...|...........\n\
............|.........................#####.#####...|...........\n\
............|..........................########.....|...........\n\
............|.......................................|...........\n\
............|##############.........................|...........\n\
............|#############..........................|...........\n\
............|...##.......#..........................|...........\n\
............|...#........##.....###############.....|...........\n\
............|...#........##.....##############......|...........\n\
............|...##.......##.........##.......#......|...........\n\
............|...###......##.........#........##.....|...........\n\
............|....#########..........#........##.....|...........\n\
............|.....#######...........##.......##.....|...........\n\
............|.......................###......##.....|...........\n\
............|........................#########......|...........\n\
............|.........................#######.......|...........\n\
............|...################....................|...........\n\
............|....##############.....................|...........\n\
............|...##.......#..........................|...........\n\
............|...#........##.........################|...........\n\
............|...#........##..........##############.|...........\n\
............|...##.......##.........##.......#......|...........\n\
............|...###......##.........#........##.....|...........\n\
............|....#########..........#........##.....|...........\n\
............|.....#######...........##.......##.....|...........\n\
............|.......................###......##.....|...........\n\
............|........................#########......|...........\n\
............|.........................#######.......|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........\n\
............|.......................................|...........");
    }

}
