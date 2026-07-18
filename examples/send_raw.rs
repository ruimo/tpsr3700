//! Diagnostic tool: sends a raw TEPRA job file (as produced by `--dump`)
//! directly to the printer, without going through font rendering. Useful
//! for testing hand-crafted or modified job byte streams.

use anyhow::Result;
use tpsr3700::usb::Printer;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: send_raw <path-to-job-file>"))?;
    let job = std::fs::read(&path)?;
    eprintln!("sending {} bytes from {path}...", job.len());
    let mut printer = Printer::open()?;
    printer.send_job(&job)?;
    eprintln!("sent.");
    Ok(())
}
