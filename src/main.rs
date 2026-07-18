use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tpsr3700::font::{FontRenderer, DEFAULT_FONT_PATH, DEFAULT_FEED_MARGIN_MM};
use tpsr3700::tepra::builder::build_print_job;
use tpsr3700::tepra::TapeWidth;

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
    /// If omitted, the largest size that fits the tape width without
    /// clipping the top or bottom is used automatically.
    #[arg(long)]
    size: Option<f32>,

    /// Blank tape feed margin (in millimeters) added before and after the
    /// printed content.
    #[arg(long, default_value_t = DEFAULT_FEED_MARGIN_MM)]
    margin_mm: f32,

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

    let (rows, margin) = renderer.render_text(&args.text, size, tape, args.margin_mm);
    let char_count = args.text.chars().count() as u32;
    let job = build_print_job(tape, char_count, &rows, margin);

    if let Some(path) = &args.dump {
        std::fs::write(path, &job)?;
        eprintln!("wrote {} bytes to {}", job.len(), path.display());
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
