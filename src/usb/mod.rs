//! USB transport: sends a TEPRA print job to the printer over its USB
//! Printer-class bulk endpoints.
//!
//! Despite Windows' USBPcap capture showing what looked like a USB Mass
//! Storage Bulk-Only Transport exchange (`USBC`/`USBS` signatures used for
//! `TEST UNIT READY` / `REQUEST SENSE` polling), the device's USB
//! descriptors advertise a standard USB Printer class interface
//! (bInterfaceClass=7, bInterfaceSubClass=1, bInterfaceProtocol=2
//! "Bidirectional"), and `GET_DEVICE_ID` confirms this:
//!
//! ```text
//! <MFG:KING JIM;CMD:ESCPL2;MDL:TEPRA PRO SR3700P;CLS:PRINTER;
//! ```
//!
//! Empirically, sending the raw TEPRA job bytes (see [`crate::tepra`])
//! directly to the bulk OUT endpoint — with no Mass Storage command
//! wrapping at all — is sufficient to print. The BOT-like exchange seen in
//! the Windows capture appears to be either a red herring (from an
//! entirely different, coincidentally similar looking sequence) or
//! optional status-polling behavior that isn't required for the printer
//! to accept a job.

use anyhow::{bail, Context, Result};
use rusb::{Device, DeviceHandle, GlobalContext};
use std::time::Duration;

/// KING JIM TEPRA SR3700P's USB vendor ID.
///
/// Confirmed via `lsusb`: `ID 0d8a:0103 King Jim Co., Ltd SR3700P`.
pub const VENDOR_ID: u16 = 0x0d8a;

/// KING JIM TEPRA SR3700P's USB product ID.
///
/// Confirmed via `lsusb`: `ID 0d8a:0103 King Jim Co., Ltd SR3700P`.
pub const PRODUCT_ID: u16 = 0x0103;

/// Finds the TEPRA SR3700P on the USB bus.
pub fn find_device() -> Result<Device<GlobalContext>> {
    for device in rusb::devices()?.iter() {
        let desc = device.device_descriptor()?;
        if desc.vendor_id() == VENDOR_ID && desc.product_id() == PRODUCT_ID {
            return Ok(device);
        }
    }
    bail!(
        "TEPRA SR3700P not found (looking for vendor={:#06x} product={:#06x}); \
         run `lsusb` to find the correct IDs and update usb::VENDOR_ID / PRODUCT_ID",
        VENDOR_ID,
        PRODUCT_ID
    );
}

/// An open connection to the printer's bulk endpoints.
pub struct Printer {
    handle: DeviceHandle<GlobalContext>,
    endpoint_out: u8,
    #[allow(dead_code)]
    endpoint_in: u8,
}

impl Printer {
    /// Open the printer and claim its Printer-class interface.
    pub fn open() -> Result<Self> {
        let device = find_device()?;
        let config = device.active_config_descriptor()?;
        let interface = config
            .interfaces()
            .next()
            .context("device has no interfaces")?;
        let descriptor = interface
            .descriptors()
            .next()
            .context("interface has no descriptors")?;

        let mut endpoint_out = None;
        let mut endpoint_in = None;
        for ep in descriptor.endpoint_descriptors() {
            use rusb::Direction;
            use rusb::TransferType;
            if ep.transfer_type() != TransferType::Bulk {
                continue;
            }
            match ep.direction() {
                Direction::Out => endpoint_out = Some(ep.address()),
                Direction::In => endpoint_in = Some(ep.address()),
            }
        }

        let endpoint_out = endpoint_out.context("no bulk OUT endpoint found")?;
        let endpoint_in = endpoint_in.context("no bulk IN endpoint found")?;

        let handle = device.open()?;

        // The printer's interface is normally bound to the kernel's
        // `usblp` driver (it advertises USB Printer class 7/1/2), which
        // would otherwise make `claim_interface` fail with "Resource
        // busy". Ask libusb to detach it automatically while we hold the
        // interface, and reattach it when the handle is dropped.
        handle
            .set_auto_detach_kernel_driver(true)
            .context("failed to enable kernel driver auto-detach")?;

        handle.claim_interface(interface.number())?;

        Ok(Self {
            handle,
            endpoint_out,
            endpoint_in,
        })
    }

    /// Returns the (OUT, IN) bulk endpoint addresses in use. Exposed for
    /// diagnostics.
    pub fn endpoints(&self) -> (u8, u8) {
        (self.endpoint_out, self.endpoint_in)
    }

    /// USB Printer class `GET_DEVICE_ID` request (bRequest=0). Returns the
    /// device ID string (e.g.
    /// `<MFG:KING JIM;CMD:ESCPL2;MDL:TEPRA PRO SR3700P;CLS:PRINTER;`).
    pub fn get_device_id(&self) -> Result<String> {
        let mut buf = [0u8; 256];
        let n = self
            .handle
            .read_control(0xA1, 0, 0, 0, &mut buf, Duration::from_secs(3))
            .context("GET_DEVICE_ID control request failed")?;
        // First 2 bytes are a big-endian length prefix per the USB Printer
        // class spec; skip them if present and just decode what we can.
        let text = if n >= 2 { &buf[2..n] } else { &buf[..n] };
        Ok(String::from_utf8_lossy(text).into_owned())
    }

    /// Sends a raw TEPRA print job byte stream directly to the printer's
    /// bulk OUT endpoint, with no additional framing.
    pub fn send_job(&mut self, job: &[u8]) -> Result<()> {
        // The printer's bulk endpoints use 64-byte max packet size; large
        // writes are automatically split into multiple USB transactions by
        // the OS/libusb, so a single write_bulk call for the whole job is
        // fine.
        let n = self
            .handle
            .write_bulk(self.endpoint_out, job, Duration::from_secs(30))
            .context("failed to send print job")?;
        if n != job.len() {
            bail!("short write: sent {} of {} bytes", n, job.len());
        }
        Ok(())
    }
}
