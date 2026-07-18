use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tpsr3700::font::{FontRenderer, DEFAULT_FONT_PATH};
use tpsr3700::tepra::builder::build_print_job;
use tpsr3700::tepra::TapeWidth;

/// Print text on a KING JIM TEPRA PRO SR3700P label printer from Linux.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Text to print.
    text: String,

    /// Tape width in millimeters (6, 12, and 18 are currently supported).
    #[arg(long, default_value_t = 18)]
    tape_mm: u8,

    /// Font size in points (as shown in the TEPRA desktop app, e.g. 36).
    #[arg(long, default_value_t = 36.0)]
    size: f32,

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

    let tape = match args.tape_mm {
        6 => TapeWidth::Mm6,
        12 => TapeWidth::Mm12,
        18 => TapeWidth::Mm18,
        other => {
            anyhow::bail!("unsupported tape width: {other}mm (only 6, 12, and 18 are supported)")
        }
    };

    let renderer = FontRenderer::load(&args.font)?;
    let (rows, margin) = renderer.render_text(&args.text, args.size, tape);
    let char_count = args.text.chars().count() as u32;
    let job = build_print_job(tape, char_count, &rows, margin);

    if let Some(path) = &args.dump {
        std::fs::write(path, &job)?;
        eprintln!("wrote {} bytes to {}", job.len(), path.display());
    }

    if !args.no_print {
        let mut printer = tpsr3700::usb::Printer::open()?;
        printer.send_job(&job)?;
        eprintln!("sent job to printer ({} bytes)", job.len());
    }

    Ok(())
}
