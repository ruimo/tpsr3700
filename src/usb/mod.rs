//! USB transport: sends a TEPRA print job to the printer over USB Mass
//! Storage's Bulk-Only Transport (BOT), which is the protocol the printer
//! actually speaks (it enumerates as a USB Mass Storage device to the
//! host).
//!
//! This module is a work-in-progress skeleton. The `TEST UNIT READY` /
//! `REQUEST SENSE` polling sequence has been observed in a USBPcap capture
//! of the Windows driver; the exact SCSI command used to transmit the
//! actual print data (presumably a vendor-specific `WRITE`-like command)
//! has not been captured yet and needs to be determined empirically
//! against real hardware (e.g. with `usbmon` on Linux) before this module
//! can drive a real printer end-to-end.

use anyhow::{bail, Context, Result};
use rusb::{Device, DeviceHandle, GlobalContext};
use std::time::Duration;

/// KING JIM TEPRA SR3700P's USB vendor ID.
///
/// TODO: confirm against `lsusb -v` output from real hardware; this is a
/// placeholder until verified.
pub const VENDOR_ID: u16 = 0x0000;

/// KING JIM TEPRA SR3700P's USB product ID.
///
/// TODO: confirm against `lsusb -v` output from real hardware; this is a
/// placeholder until verified.
pub const PRODUCT_ID: u16 = 0x0000;

const CBW_SIGNATURE: u32 = 0x4342_5355; // "USBC" little-endian
const CSW_SIGNATURE: u32 = 0x5342_5355; // "USBS" little-endian
const CBW_LEN: usize = 31;
const CSW_LEN: usize = 13;

/// A Command Block Wrapper, as used by USB Mass Storage's Bulk-Only
/// Transport.
struct Cbw {
    tag: u32,
    data_transfer_length: u32,
    flags: u8,
    lun: u8,
    command: Vec<u8>,
}

impl Cbw {
    fn to_bytes(&self) -> [u8; CBW_LEN] {
        let mut buf = [0u8; CBW_LEN];
        buf[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
        buf[4..8].copy_from_slice(&self.tag.to_le_bytes());
        buf[8..12].copy_from_slice(&self.data_transfer_length.to_le_bytes());
        buf[12] = self.flags;
        buf[13] = self.lun;
        buf[14] = self.command.len() as u8;
        buf[15..15 + self.command.len()].copy_from_slice(&self.command);
        buf
    }
}

/// Direction flag for [`Cbw::flags`].
const CBW_FLAGS_DATA_IN: u8 = 0x80;
const CBW_FLAGS_DATA_OUT: u8 = 0x00;

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

/// An open connection to the printer's Mass Storage bulk endpoints.
pub struct Printer {
    handle: DeviceHandle<GlobalContext>,
    endpoint_out: u8,
    endpoint_in: u8,
    next_tag: u32,
}

impl Printer {
    /// Open the printer and claim its Mass Storage interface.
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
        handle.claim_interface(interface.number())?;

        Ok(Self {
            handle,
            endpoint_out,
            endpoint_in,
            next_tag: 1,
        })
    }

    fn next_tag(&mut self) -> u32 {
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1);
        tag
    }

    /// Sends a single SCSI command with no data phase, and returns the
    /// resulting command status (0 = success).
    fn send_command_no_data(&mut self, lun: u8, command: &[u8]) -> Result<u8> {
        let tag = self.next_tag();
        let cbw = Cbw {
            tag,
            data_transfer_length: 0,
            flags: CBW_FLAGS_DATA_OUT,
            lun,
            command: command.to_vec(),
        };
        self.write_bulk(&cbw.to_bytes())?;
        self.read_csw(tag)
    }

    /// Sends a SCSI command followed by an OUT data phase.
    fn send_command_with_data_out(&mut self, lun: u8, command: &[u8], data: &[u8]) -> Result<u8> {
        let tag = self.next_tag();
        let cbw = Cbw {
            tag,
            data_transfer_length: data.len() as u32,
            flags: CBW_FLAGS_DATA_OUT,
            lun,
            command: command.to_vec(),
        };
        self.write_bulk(&cbw.to_bytes())?;
        self.write_bulk(data)?;
        self.read_csw(tag)
    }

    /// Sends a SCSI command with an IN data phase, returning the received
    /// data along with the command status.
    fn send_command_with_data_in(
        &mut self,
        lun: u8,
        command: &[u8],
        expected_len: usize,
    ) -> Result<(Vec<u8>, u8)> {
        let tag = self.next_tag();
        let cbw = Cbw {
            tag,
            data_transfer_length: expected_len as u32,
            flags: CBW_FLAGS_DATA_IN,
            lun,
            command: command.to_vec(),
        };
        self.write_bulk(&cbw.to_bytes())?;
        let data = self.read_bulk(expected_len)?;
        let status = self.read_csw(tag)?;
        Ok((data, status))
    }

    fn write_bulk(&mut self, data: &[u8]) -> Result<()> {
        self.handle
            .write_bulk(self.endpoint_out, data, Duration::from_secs(5))
            .context("bulk OUT transfer failed")?;
        Ok(())
    }

    fn read_bulk(&mut self, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        let n = self
            .handle
            .read_bulk(self.endpoint_in, &mut buf, Duration::from_secs(5))
            .context("bulk IN transfer failed")?;
        buf.truncate(n);
        Ok(buf)
    }

    fn read_csw(&mut self, expected_tag: u32) -> Result<u8> {
        let buf = self.read_bulk(CSW_LEN)?;
        if buf.len() < CSW_LEN {
            bail!("short CSW: got {} bytes, expected {}", buf.len(), CSW_LEN);
        }
        let signature = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let tag = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let status = buf[12];
        if signature != CSW_SIGNATURE {
            bail!("bad CSW signature: {:#010x}", signature);
        }
        if tag != expected_tag {
            bail!("CSW tag mismatch: expected {:#010x}, got {:#010x}", expected_tag, tag);
        }
        Ok(status)
    }

    /// SCSI `TEST UNIT READY` (opcode 0x00).
    pub fn test_unit_ready(&mut self) -> Result<u8> {
        self.send_command_no_data(0, &[0x00, 0, 0, 0, 0, 0])
    }

    /// SCSI `REQUEST SENSE` (opcode 0x03). Returns the raw sense data.
    pub fn request_sense(&mut self) -> Result<Vec<u8>> {
        let command = [0x03, 0, 0, 0, 18, 0];
        let (data, _status) = self.send_command_with_data_in(0, &command, 18)?;
        Ok(data)
    }

    /// Polls `TEST UNIT READY` until the printer reports ready (tape
    /// loaded), or the timeout elapses.
    pub fn wait_until_ready(&mut self, timeout: Duration) -> Result<()> {
        let start = std::time::Instant::now();
        loop {
            if self.test_unit_ready()? == 0 {
                return Ok(());
            }
            let _sense = self.request_sense()?;
            if start.elapsed() > timeout {
                bail!("timed out waiting for printer to become ready (tape not loaded?)");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Sends a raw TEPRA print job byte stream to the printer.
    ///
    /// TODO: the exact SCSI command opcode/parameters used by the vendor
    /// driver to transmit print data have not been captured yet (see
    /// module docs). This is a placeholder using SCSI `WRITE(10)`
    /// (opcode 0x2A) with LBA=0, which is *not* confirmed to work with
    /// this printer and will likely need adjustment once verified against
    /// real hardware traffic.
    pub fn send_job(&mut self, job: &[u8]) -> Result<()> {
        self.wait_until_ready(Duration::from_secs(10))?;

        // Placeholder SCSI WRITE(10): opcode, flags, LBA(4), group(1),
        // transfer length in blocks(2), control(1). Block size and LBA
        // semantics are not yet confirmed for this device.
        let block_size = 512usize;
        let blocks = job.len().div_ceil(block_size) as u16;
        let mut command = vec![0x2A, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        command[7..9].copy_from_slice(&blocks.to_be_bytes());

        let mut padded = job.to_vec();
        padded.resize(blocks as usize * block_size, 0);

        let status = self.send_command_with_data_out(0, &command, &padded)?;
        if status != 0 {
            bail!("printer returned non-zero status for print job: {status}");
        }
        Ok(())
    }
}
