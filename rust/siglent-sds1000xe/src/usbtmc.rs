// SPDX-License-Identifier: Apache-2.0
//! SCPI over a Linux /dev/usbtmc* character device.
//!
//! The kernel usbtmc driver handles USB-TMC framing; reads return message
//! chunks (a reply may span several reads). Requires read/write access to
//! the device node (udev rule, see plugins/siglent-sds1000xe/99-openmso-usbtmc.rules).
//!
//! On the SDS1000X-E (USB full speed) the kernel driver truncates replies
//! longer than 52 bytes and the undrained remainder wedges the interface —
//! see plugins/siglent-sds1000xe/NOTES.md. Short control queries work; this
//! transport self-heals a wedged interface once per session via
//! USBDEVFS_RESET, exactly like the Python implementation.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::time::Duration;

use crate::scpi::{find_block, Result, Scpi, ScpiError};

const READ_CHUNK: usize = 1 << 20;
const USBDEVFS_RESET: libc::c_ulong = ((b'U' as libc::c_ulong) << 8) | 20;
const VENDOR_IDS: &[u16] = &[0xF4EC]; // Siglent; extend as other usbtmc gear is tested

pub struct UsbTmcScpi {
    path: String,
    file: File,
    recovered: bool,
}

impl UsbTmcScpi {
    pub fn new(path: &str) -> Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)
            .map_err(|e| ScpiError(format!("{path}: {e}")))?;
        Ok(UsbTmcScpi { path: path.to_string(), file, recovered: false })
    }

    /// One usbtmc read. A 0-byte result, ETIMEDOUT or EPIPE means the
    /// interface is stuck (a previous session closed with undrained reply
    /// data). Recover once per session via USB device reset + reopen.
    fn read_chunk(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; n];
        let got = match self.file.read(&mut buf) {
            Ok(k) => k,
            Err(e) => {
                let stuck = matches!(e.raw_os_error(),
                                     Some(libc::ETIMEDOUT) | Some(libc::EPIPE));
                if !stuck || self.recovered {
                    return Err(e.into());
                }
                0
            }
        };
        buf.truncate(got);
        if !buf.is_empty() || self.recovered {
            return Ok(buf);
        }
        self.recovered = true;
        self.usb_reset()?;
        Ok(Vec::new())
    }

    fn usb_reset(&mut self) -> Result<()> {
        let mut reset_done = false;
        for bus in std::fs::read_dir("/dev/bus/usb").map_err(ScpiError::from)? {
            let Ok(bus) = bus else { continue };
            let Ok(devices) = std::fs::read_dir(bus.path()) else { continue };
            for dev in devices.flatten() {
                let dev_path = dev.path();
                let Ok(mut f) = File::open(&dev_path) else { continue };
                let mut desc = [0u8; 18];
                if f.read_exact(&mut desc).is_err() {
                    continue;
                }
                let vid = u16::from_le_bytes([desc[8], desc[9]]);
                if !VENDOR_IDS.contains(&vid) {
                    continue;
                }
                let Ok(w) = OpenOptions::new().write(true).open(&dev_path) else {
                    continue;
                };
                if unsafe { libc::ioctl(w.as_raw_fd(), USBDEVFS_RESET, 0) } == 0 {
                    reset_done = true;
                    break;
                }
            }
            if reset_done {
                break;
            }
        }
        if !reset_done {
            return Err(ScpiError(format!(
                "{}: interface stuck and no USB device found to reset", self.path)));
        }
        std::thread::sleep(Duration::from_secs(1));
        self.file = OpenOptions::new().read(true).write(true).open(&self.path)
            .map_err(|e| ScpiError(format!("{}: reopen after reset: {e}", self.path)))?;
        Ok(())
    }
}

impl Scpi for UsbTmcScpi {
    fn command(&mut self, cmd: &str) -> Result<()> {
        self.file.write_all(cmd.as_bytes())?;
        self.file.write_all(b"\n")?;
        Ok(())
    }

    fn query(&mut self, cmd: &str) -> Result<String> {
        let mut buf: Vec<u8> = Vec::new();
        for _ in 0..2 { // retry once after an automatic recovery
            self.command(cmd)?;
            buf.clear();
            // Loop until we have real content ending in LF (tolerates stray
            // terminator bytes left over from a previous block transfer).
            loop {
                let trimmed_nonempty = buf.iter().any(|b| !b.is_ascii_whitespace());
                if trimmed_nonempty && buf.ends_with(b"\n") {
                    break;
                }
                let chunk = self.read_chunk(4096)?;
                if chunk.is_empty() {
                    break;
                }
                buf.extend_from_slice(&chunk);
            }
            if buf.iter().any(|b| !b.is_ascii_whitespace()) {
                break;
            }
        }
        Ok(String::from_utf8_lossy(&buf).trim().to_string())
    }

    fn query_block(&mut self, cmd: &str, _timeout: Option<Duration>) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        for _ in 0..2 { // retry once if a stuck interface was recovered
            self.command(cmd)?;
            buf = self.read_chunk(READ_CHUNK)?;
            if !buf.is_empty() {
                break;
            }
        }
        let mut chunk = vec![0u8; READ_CHUNK];
        let (start, dlen) = loop {
            if buf.is_empty() {
                return Err(ScpiError(format!("no reply to block query {cmd:?}")));
            }
            if let Some(loc) = find_block(&buf)? {
                break loc;
            }
            let n = self.file.read(&mut chunk)?;
            if n == 0 {
                return Err(ScpiError(format!("EOF reading block header for {cmd:?}")));
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let total = start + dlen;
        while buf.len() < total {
            let n = self.file.read(&mut chunk)?;
            if n == 0 {
                return Err(ScpiError(format!(
                    "EOF mid-block ({}/{} bytes)", buf.len(), total)));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        buf.truncate(total);
        Ok(buf.split_off(start))
    }
}
