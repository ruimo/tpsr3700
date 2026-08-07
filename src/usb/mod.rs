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

        let handle = device.open().context(
            "failed to open the printer's USB device (permission denied?)\n\n\
             This usually means your user doesn't have read/write permission to\n\
             access the printer's USB device node. Try one of:\n\
             \n\
             1. Create a udev rule (recommended — survives reboots):\n\
             \n\
             \x20   echo 'SUBSYSTEM==\"usb\", ATTRS{idVendor}==\"0d8a\", ATTRS{idProduct}==\"0103\", GROUP=\"lp\", MODE=\"0660\"' \\\n\
             \x20       | sudo tee /etc/udev/rules.d/99-tepra.rules\n\
             \x20   sudo usermod -aG lp $USER\n\
             \x20   sudo udevadm control --reload && sudo udevadm trigger\n\
             \n\
             \x20   Then unplug/re-plug the printer and start a new login session\n\
             \x20   (or run `newgrp lp` in the current shell).\n\
             \n\
             2. Add your user to the `lp` group and start a new login session\n\
             (log out/in, or run `newgrp lp` in a new shell):\n\
             \n\
             \x20   sudo usermod -aG lp $USER\n\
             \n\
             3. Or, run this command with a group override for the current\n\
             shell only (no logout required):\n\
             \n\
             \x20   sg lp -c '<the command you just ran>'\n\
             \n\
             4. Or, run as root (not recommended for routine use):\n\
             \n\
             \x20   sudo <the command you just ran>",
        )?;

        // On Linux the printer's interface is normally bound to the
        // kernel's `usblp` driver (it advertises USB Printer class 7/1/2),
        // which would otherwise make `claim_interface` fail with "Resource
        // busy". Ask libusb to detach it automatically while we hold the
        // interface, and reattach it when the handle is dropped.
        // This API is Linux-only; macOS has no equivalent kernel driver to
        // detach (IOKit releases the interface automatically).
        #[cfg(target_os = "linux")]
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

    /// Vendor-specific control request (`bmRequestType=0xC1`,
    /// `bRequest=0x01`, `wValue=0`, `wIndex=0`, `wLength=8`) observed being
    /// sent by the TEPRA desktop app's "detect tape width" feature
    /// (captured via USBPcap on Windows). Returns the raw 8-byte response.
    ///
    /// The exact encoding of the response is not fully understood yet; see
    /// `docs/protocol.md` for analysis notes.
    pub fn get_tape_info_raw(&self) -> Result<[u8; 8]> {
        let mut buf = [0u8; 8];
        let n = self
            .handle
            .read_control(0xC1, 0x01, 0, 0, &mut buf, Duration::from_secs(3))
            .context("tape info control request failed")?;
        if n != 8 {
            bail!("unexpected response length: got {n} bytes, expected 8");
        }
        Ok(buf)
    }

    /// Detects the currently loaded tape's width by querying the printer
    /// (see [`Printer::get_tape_info_raw`]) and decoding the tape width
    /// index in byte 3 of the response via
    /// [`crate::tepra::TapeWidth::from_detected_index`].
    ///
    /// Returns `Ok(None)` if the printer reports a tape width index that
    /// isn't currently supported by this crate (see
    /// `TapeWidth::from_detected_index`'s docs for which indices those
    /// are), along with the raw index value for diagnostics.
    pub fn get_tape_width(&self) -> Result<Option<crate::tepra::TapeWidth>> {
        let raw = self.get_tape_info_raw()?;
        let index = raw[3];
        Ok(crate::tepra::TapeWidth::from_detected_index(index))
    }

    /// Same as [`Printer::get_tape_width`], but also returns the raw
    /// detected index (useful for diagnostics/error messages when the
    /// index isn't a currently-supported width).
    pub fn get_tape_width_with_index(&self) -> Result<(Option<crate::tepra::TapeWidth>, u8)> {
        let raw = self.get_tape_info_raw()?;
        let index = raw[3];
        Ok((crate::tepra::TapeWidth::from_detected_index(index), index))
    }

    /// Sends a raw TEPRA print job byte stream directly to the printer's
    /// bulk OUT endpoint, with no additional framing.
    pub fn send_job(&mut self, job: &[u8]) -> Result<()> {
        let mut sent = 0;
        while sent < job.len() {
            let n = self
                .handle
                .write_bulk(self.endpoint_out, &job[sent..], Duration::from_secs(30))
                .with_context(|| format!("failed to send print job after {sent} bytes"))?;
            if n == 0 {
                bail!("USB bulk write made no progress after {sent} of {} bytes", job.len());
            }
            sent += n;
        }
        Ok(())
    }
}
