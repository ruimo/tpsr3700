//! Diagnostic tool: dumps the internal decisions made by
//! `render_text_multiline` (line count, font size, wrapped line contents)
//! without needing the printer connected. Used to debug wrapping issues.

use tpsr3700::font::{FontRenderer, TextAlign, DEFAULT_FONT_PATH, DEFAULT_FEED_MARGIN_MM};
use tpsr3700::tepra::TapeWidth;

fn main() -> anyhow::Result<()> {
    let text = std::env::args().nth(1).unwrap_or_else(|| "D39pb".to_string());
    let tape_length_mm: f32 = std::env::args()
        .nth(2)
        .map(|s| s.parse().unwrap())
        .unwrap_or(32.0);

    let renderer = FontRenderer::load(DEFAULT_FONT_PATH)?;

    for tape in [TapeWidth::Mm6, TapeWidth::Mm12, TapeWidth::Mm18] {
        println!("=== tape = {}mm ===", tape.width_mm());
        let (rows, margin) = renderer.render_text_multiline(
            &text,
            None,
            tape,
            DEFAULT_FEED_MARGIN_MM,
            tape_length_mm,
            TextAlign::Left,
            TapeWidth::HEAD_MARGIN_BITS,
            TapeWidth::HEAD_MARGIN_BITS,
        );
        println!("total rows = {}, margin = {}", rows.len(), margin);

        // Check which "bands" (vertical halves/etc) have any ink at all.
        let total_bits = tape.bitmap_bits();
        let mut band_has_ink = vec![false; 8];
        for row in &rows {
            let bits: String = row.iter().map(|b| format!("{:08b}", b)).collect();
            for (bi, c) in bits.chars().enumerate() {
                if bi >= total_bits {
                    break;
                }
                if c == '1' {
                    // crude: split total_bits into up to 8 bands for reporting
                    let band = bi * 8 / total_bits;
                    band_has_ink[band] = true;
                }
            }
        }
        println!("bands with ink (of 8 crude slices): {:?}", band_has_ink);

        // Print an ASCII-art rendering to visually confirm line placement.
        print_ascii_art(&rows, total_bits);
        println!();
    }

    Ok(())
}

fn print_ascii_art(rows: &[Vec<u8>], total_bits: usize) {
    // Downsample rows to at most 200 columns for readability.
    let step = (rows.len() / 200).max(1);
    for (i, row) in rows.iter().enumerate() {
        if i % step != 0 {
            continue;
        }
        let bits: String = row.iter().map(|b| format!("{:08b}", b)).collect();
        let line: String = bits
            .chars()
            .take(total_bits)
            .map(|c| if c == '1' { '#' } else { '.' })
            .collect();
        println!("{line}");
    }
}
