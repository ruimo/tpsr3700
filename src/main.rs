use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tpsr3700::font::{FontRenderer, TextAlign, DEFAULT_FONT_PATH, DEFAULT_FEED_MARGIN_MM};
use tpsr3700::tepra::builder::build_print_job;
use tpsr3700::tepra::TapeWidth;

/// Parse `job` and print a human-readable summary plus an ASCII-art
/// rendering of the raster rows to stdout.
fn dump_explain(job: &[u8], tape: TapeWidth) {
    let bitmap_bytes = tape.bitmap_bytes();
    let bitmap_bits = tape.bitmap_bits();
    let head_offset = tape.head_offset();
    let head_bits = tape.head_bits();

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

    // ASCII art — downsample feed direction to ≤120 columns for readability.
    println!();
    println!("[ASCII art]  '#'=ink  '.'=blank  '|'=non-printing margin boundary");
    let step = (row_count as usize / 120).max(1);
    for (i, bm) in bitmaps.iter().enumerate() {
        if i % step != 0 { continue; }
        let line: String = (0..bitmap_bits).map(|bit| {
            let byte_i = bit / 8;
            let bit_j  = bit % 8;
            let inked  = (bm[byte_i] >> (7 - bit_j)) & 1 == 1;
            if bit == head_offset || bit == head_offset + head_bits {
                '|'
            } else if inked {
                '#'
            } else {
                '.'
            }
        }).collect();
        println!("{}", line);
    }

    // ---- Footer ----
    println!();
    println!("[Footer]  0x0C + ESC{{'{{'@'}}");
}

/// Print text on a KING JIM TEPRA PRO SR3700P label printer from Linux.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Text to print.
    text: String,

    /// Tape width in millimeters (6, 12, and 18 are currently supported).
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

    /// Blank tape feed margin (in millimeters) added before and after the
    /// printed content.
    #[arg(long, default_value_t = DEFAULT_FEED_MARGIN_MM)]
    margin_mm: f32,

    /// Total label length in millimeters, including left and right feed
    /// margins.  When the text does not fit on one line at the chosen font
    /// size, it is wrapped character-by-character onto additional lines
    /// (up to the maximum allowed by the tape width; minimum line height
    /// is 3 mm, so a 6 mm tape allows at most 2 lines).  Omitting this
    /// option disables wrapping and prints everything on a single line.
    #[arg(long)]
    tape_length_mm: Option<f32>,

    /// Horizontal alignment of each line within the label length.
    /// Only meaningful when --tape-length-mm is set.
    /// Allowed values: left (default), center, right.
    #[arg(long, default_value = "left")]
    align: TextAlign,

    /// Path to a TrueType/OpenType font file (.ttf/.otf/.ttc). Defaults to
    /// the system's Noto Sans CJK JP font.
    #[arg(long, default_value = DEFAULT_FONT_PATH)]
    font: String,

    /// Write the generated raw TEPRA job bytes to this file instead of (or
    /// in addition to) sending it to the printer. Useful for debugging
    /// without hardware.
    #[arg(long)]
    dump: Option<PathBuf>,

    /// Skip sending the job to the printer over USB (only useful together
    /// with --dump).
    #[arg(long)]
    no_print: bool,
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
        Some(other) => {
            anyhow::bail!("unsupported tape width: {other}mm (only 6, 12, and 18 are supported)")
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
                     isn't supported yet (only 6, 12, and 18mm are); pass --tape-mm explicitly \
                     if you know the width, or see docs/protocol.md section 1.5 for the index \
                     mapping"
                ),
            }
        }
    };

    let renderer = FontRenderer::load(&args.font)?;

    let (rows, margin) = if let Some(tape_length_mm) = args.tape_length_mm {
        // Multi-line mode: wrap text to fit the specified tape length.
        let size = match args.size {
            Some(size) => {
                // Validate manually since would_clip is tape-width-based,
                // but the per-band check happens inside render_text_multiline.
                if renderer.would_clip(size, tape) {
                    let max = renderer.max_size_pt_for_tape(tape);
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
            args.margin_mm,
            tape_length_mm,
            args.align,
        )
    } else {
        // Single-line mode (original behaviour).
        let size = match args.size {
            Some(size) => {
                if renderer.would_clip(size, tape) {
                    let max = renderer.max_size_pt_for_tape(tape);
                    eprintln!(
                        "warning: {size}pt may be too large for {}mm tape and could be \
                         clipped at the top/bottom; try --size {max:.1} or smaller",
                        tape.width_mm()
                    );
                }
                size
            }
            None => {
                let max = renderer.max_size_pt_for_tape(tape);
                eprintln!("no --size given; using {max:.1}pt (largest that fits {}mm tape)", tape.width_mm());
                max
            }
        };
        renderer.render_text(&text, size, tape, args.margin_mm)
    };

    let char_count = text.chars().count() as u32;
    let job = build_print_job(tape, char_count, &rows, margin);

    if let Some(path) = &args.dump {
        std::fs::write(path, &job)?;
        eprintln!("wrote {} bytes to {}", job.len(), path.display());
        dump_explain(&job, tape);
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
