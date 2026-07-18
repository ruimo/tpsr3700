//! Diagnostic tool: opens the printer over USB and reports its device ID
//! string, without sending any print data. Useful for verifying USB
//! connectivity/permissions before attempting a real print job.

use anyhow::Result;
use tpsr3700::usb::Printer;

fn main() -> Result<()> {
    println!("Opening printer...");
    let printer = Printer::open()?;
    println!("Printer opened successfully.");
    let (ep_out, ep_in) = printer.endpoints();
    println!("Endpoints: OUT={ep_out:#04x} IN={ep_in:#04x}");

    println!("Requesting device ID...");
    let id = printer.get_device_id()?;
    println!("Device ID: {id}");

    Ok(())
}
