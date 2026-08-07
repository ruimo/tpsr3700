use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tpsr3700::font::{FontRenderer, TextAlign, DEFAULT_FONT_BYTES, DEFAULT_FEED_MARGIN_MM};
use tpsr3700::tepra::builder::build_print_job;
use tpsr3700::tepra::TapeWidth;

/// Parse `job` and print a human-readable summary plus an ASCII-art
/// rendering of the raster rows to stdout.
fn dump_explain(job: &[u8], tape: TapeWidth, top_margin: usize, bottom_margin: usize) {
    let bitmap_bytes = tape.bitmap_bytes();
    let bitmap_bits = tape.bitmap_bits();
    let head_offset = bottom_margin;
    let head_bits = tape.bitmap_bits().saturating_sub(top_margin + bottom_margin).max(1);

    // ---- Header block (fixed 2179 bytes) ----
    let tape_mm_byte = job[8];
    println!("=== TEPRA job dump ({} bytes) ===", job.len());
    println!();
    println!("[Header block]  bytes 0-2178 (2179 bytes, fixed)");
    println!("  tape width byte (offset 8) : 0x{:02X} = {}mm", tape_mm_byte, tape_mm_byte);

    // ---- Config commands ----
    // Frame layout: ESC { <len> <cmd> <params…> <checksum> }
    // Lengths: C=10, D=7, G=6, L=10, T=9  (ESC+{+len+cmd+params+cs+})
    const HEADER_LEN: usize = 2179;
    const CMD_FRAME_OVERHEAD: usize = 4; // ESC { len cmd ... cs }  (excl params)
    let cmd_c_len = CMD_FRAME_OVERHEAD + 4 + 2; // 4 params + cs + }
    let cmd_d_len = CMD_FRAME_OVERHEAD + 1 + 2;
    let cmd_g_len = CMD_FRAME_OVERHEAD + 0 + 2;
    let cmd_l_len = CMD_FRAME_OVERHEAD + 4 + 2;
    let cmd_t_len = CMD_FRAME_OVERHEAD + 2 + 2;

    let l_params = HEADER_LEN + cmd_c_len + cmd_d_len + cmd_g_len + 4; // skip ESC { len 'L'
    let row_count = u32::from_le_bytes(job[l_params..l_params + 4].try_into().unwrap());

    let t_params = HEADER_LEN + cmd_c_len + cmd_d_len + cmd_g_len + cmd_l_len + 4;
    let margin_rows = u16::from_le_bytes(job[t_params..t_params + 2].try_into().unwrap());

    println!();
    println!("[Config commands]");
    println!("  L (row count)  : {}", row_count);
    println!("  T (margin rows): {}", margin_rows);

    // ---- Raster rows ----
    let row_len = 7 + bitmap_bytes; // ESC . 00 0D 0D 01 <width_byte> <bitmap>
    let raster_start = HEADER_LEN + cmd_c_len + cmd_d_len + cmd_g_len + cmd_l_len + cmd_t_len;

    println!();
    println!(
        "[Raster rows]  {} rows × {} bitmap bytes ({} bits, printable: bits {}..{})",
        row_count,
        bitmap_bytes,
        bitmap_bits,
        head_offset,
        head_offset + head_bits - 1,
    );

    // Collect bitmaps.
    let mut bitmaps: Vec<&[u8]> = Vec::new();
    for i in 0..row_count as usize {
        let start = raster_start + i * row_len;
        bitmaps.push(&job[start + 7..start + 7 + bitmap_bytes]);
    }

    // Per-bit ink counts (for summary).
    let mut bit_ink = vec![0u32; bitmap_bits];
    for bm in &bitmaps {
        for (byte_i, &byte) in bm.iter().enumerate() {
            for bit_j in 0..8 {
                let bit_pos = byte_i * 8 + bit_j;
                if bit_pos >= bitmap_bits { break; }
                if (byte >> (7 - bit_j)) & 1 == 1 {
                    bit_ink[bit_pos] += 1;
                }
            }
        }
    }
    let first_ink = bit_ink.iter().position(|&v| v > 0);
    let last_ink  = bit_ink.iter().rposition(|&v| v > 0);
    match (first_ink, last_ink) {
        (Some(f), Some(l)) => println!("  ink bit range  : {}..{} (span {})", f, l, l - f + 1),
        _ => println!("  ink bit range  : (none)"),
    }

    // ---- Footer ----
    println!();
    println!("[Footer]  0x0C + ESC{{'{{'@'}}");
}

/// Print text on a KING JIM TEPRA PRO SR3700P label printer from Linux or macOS.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Text to print.
    text: String,

    /// Tape width in millimeters (6, 12, 18, and experimental 24 are supported).
    /// If omitted, the width is auto-detected by querying the printer
    /// (requires the printer to be connected; incompatible with
    /// --no-print).
    #[arg(long)]
    tape_mm: Option<u8>,

    /// Font size in points (as shown in the TEPRA desktop app, e.g. 36).
    /// If omitted, the largest size that fits the tape width (or one
    /// line-band when multi-line wrapping is active) is used automatically.
    #[arg(long)]
    size: Option<f32>,

    /// Blank tape feed margin (in millimeters) added before the printed
    /// content (left side when the label exits the printer).
    #[arg(long, default_value_t = 10.0)]
    left_margin_mm: f32,

    /// Blank tape feed margin (in millimeters) added after the printed
    /// content (right side).
    #[arg(long, default_value_t = DEFAULT_FEED_MARGIN_MM)]
    right_margin_mm: f32,

    /// Non-printing margin at the top edge of the tape, in dots.
    #[arg(long, default_value_t = 2)]
    top_margin: usize,

    /// Non-printing margin at the bottom edge of the tape, in dots.
    #[arg(long, default_value_t = 8)]
    bottom_margin: usize,

    /// Total label length in millimeters, including left and right feed
    /// margins.  When the text does not fit on one line at the chosen font
    /// size, it is wrapped character-by-character onto additional lines
    /// (up to the maximum allowed by the tape width; minimum line height
    /// is 3 mm, so a 6 mm tape allows at most 2 lines).  Omitting this
    /// option disables wrapping and prints everything on a single line.
    #[arg(long)]
    max_tape_length_mm: Option<f32>,

    /// Horizontal alignment of each line within the label length.
    /// Only meaningful when --max-tape-length-mm is set.
    /// Allowed values: left (default), center, right.
    #[arg(long, default_value = "left")]
    align: TextAlign,

    /// Path to a TrueType/OpenType font file (.ttf/.otf/.ttc).
    /// When omitted, the bundled BIZ UDPゴシック Regular is used.
    #[arg(long)]
    font: Option<String>,

    /// Write the generated raw TEPRA job bytes to this file instead of (or
    /// in addition to) sending it to the printer. Useful for debugging
    /// without hardware.
    #[arg(long)]
    dump: Option<PathBuf>,

    /// Skip sending the job to the printer over USB (only useful together
    /// with --dump).
    #[arg(long)]
    no_print: bool,

    /// Skip the print preview and send the job straight to the
    /// printer (or to --dump) without asking for confirmation. By
    /// default an SVG preview is shown before printing; see the
    /// `preview` feature (enabled by default).
    #[arg(long)]
    skip_preview: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Replace literal "\n" (two characters: backslash + n) with a real
    // newline, so the user can write --  "line1\nline2" on the command line
    // without needing shell $'...' quoting.
    let text = args.text.replace("\\n", "\n");

    // Open the printer early if we might need it, either for tape width
    // auto-detection or for actually sending the job, so both can share
    // the same connection.
    let mut printer = if !args.no_print || args.tape_mm.is_none() {
        Some(tpsr3700::usb::Printer::open()?)
    } else {
        None
    };

    let tape = match args.tape_mm {
        Some(6) => TapeWidth::Mm6,
        Some(12) => TapeWidth::Mm12,
        Some(18) => TapeWidth::Mm18,
        Some(24) => TapeWidth::Mm24,
        Some(other) => {
            anyhow::bail!("unsupported tape width: {other}mm (only 6, 12, 18, and experimental 24 are supported)")
        }
        None => {
            let printer = printer
                .as_ref()
                .expect("printer connection opened above when --tape-mm is omitted");
            let (detected, index) = printer.get_tape_width_with_index()?;
            match detected {
                Some(tape) => {
                    eprintln!("detected {}mm tape loaded in printer", tape.width_mm());
                    tape
                }
                None => anyhow::bail!(
                    "detected tape width index {index} from the printer, but this width \
                     isn't supported yet (only 6, 12, 18, and experimental 24mm are); pass --tape-mm explicitly \
                     if you know the width, or see docs/protocol.md section 1.5 for the index \
                     mapping"
                ),
            }
        }
    };

    let renderer = match &args.font {
        Some(path) => FontRenderer::load(path)?,
        None => FontRenderer::from_bytes(DEFAULT_FONT_BYTES)
            .map_err(|e| anyhow::anyhow!("bundled font failed to load: {e}"))?,
    };

    let (rows, margin) = if let Some(tape_length_mm) = args.max_tape_length_mm {
        // Multi-line mode: wrap text to fit the specified tape length.
        let size = match args.size {
            Some(size) => {
                // Validate manually since would_clip is tape-width-based,
                // but the per-band check happens inside render_text_multiline.
                if renderer.would_clip(size, tape, args.top_margin, args.bottom_margin) {
                    let max = renderer.max_size_pt_for_tape(tape, args.top_margin, args.bottom_margin);
                    eprintln!(
                        "warning: {size}pt may be too large for {}mm tape and could be \
                         clipped at the top/bottom; try --size {max:.1} or smaller",
                        tape.width_mm()
                    );
                }
                Some(size)
            }
            None => None,
        };
        renderer.render_text_multiline(
            &text,
            size,
            tape,
            args.left_margin_mm,
            args.right_margin_mm,
            tape_length_mm,
            args.align,
            args.top_margin,
            args.bottom_margin,
        )
    } else {
        // Single-line mode, or implicit multi-line via '\n'.
        let num_lines = text.lines().count().max(1);
        let size = match args.size {
            Some(size) => {
                if renderer.would_clip(size, tape, args.top_margin, args.bottom_margin) {
                    let max = renderer.max_size_pt_for_tape(tape, args.top_margin, args.bottom_margin);
                    eprintln!(
                        "warning: {size}pt may be too large for {}mm tape and could be \
                         clipped at the top/bottom; try --size {max:.1} or smaller",
                        tape.width_mm()
                    );
                }
                size
            }
            None => {
                let max = renderer.max_size_pt_for_tape(tape, args.top_margin, args.bottom_margin);
                let size = max / num_lines as f32;
                if num_lines > 1 {
                    eprintln!("no --size given; using {size:.1}pt ({num_lines} lines on {}mm tape)", tape.width_mm());
                } else {
                    eprintln!("no --size given; using {size:.1}pt (largest that fits {}mm tape)", tape.width_mm());
                }
                size
            }
        };
        renderer.render_text(&text, size, tape, args.left_margin_mm, args.right_margin_mm, args.top_margin, args.bottom_margin)
    };

    let char_count = text.chars().count() as u32;
    let job = build_print_job(tape, char_count, &rows, margin);

    if let Some(path) = &args.dump {
        std::fs::write(path, &job)?;
        eprintln!("wrote {} bytes to {}", job.len(), path.display());
        dump_explain(&job, tape, args.top_margin, args.bottom_margin);
    }

    if !args.skip_preview {
        #[cfg(feature = "preview")]
        {
            let right_margin_rows = tpsr3700::font::mm_to_rows(args.right_margin_mm);
            match tpsr3700::preview::show_preview(
                &rows,
                tape,
                &text,
                args.top_margin,
                args.bottom_margin,
                margin,
                right_margin_rows,
            )? {
                tpsr3700::preview::PreviewResult::Print => {}
                tpsr3700::preview::PreviewResult::Cancel => {
                    eprintln!("cancelled by user in preview; not printing.");
                    return Ok(());
                }
            }
        }
        #[cfg(not(feature = "preview"))]
        {
            anyhow::bail!(
                "the print preview requires the crate to be built with the \
                 `preview` feature; pass --skip-preview to print without previewing"
            );
        }
    }

    if !args.no_print {
        let printer = printer
            .as_mut()
            .expect("printer connection opened above when --no-print is not set");
        printer.send_job(&job)?;
        eprintln!("sent job to printer ({} bytes)", job.len());
    }

    Ok(())
}
