// SPDX-License-Identifier: Apache-2.0
//! Cypress FX2LP / fx2lafw USB layer: enumerate, upload firmware (Cypress
//! 0xA0 bootloader), drive vendor commands, stream samples from EP2 bulk-IN.
//!
//! Written clean-room from the fx2lafw protocol description and live
//! observation. No GPL code is included; see `docs/fx2-plan/README.md` §3.

use std::thread;
use std::time::{Duration, Instant};

use nusb::descriptors::TransferType;
use nusb::transfer::{
    Bulk, Buffer, ControlIn, ControlOut, ControlType, Direction, In, Recipient,
    TransferError,
};
use nusb::{Device, Interface, MaybeFuture};

use crate::firmware;

// --- Cypress bootloader (FX2 ROM) --------------------------------------
// Vendor request 0xA0, recipient Device. The FX2 ROM handles these while the
// 8051 is held in reset; fx2lafw does NOT implement 0xA0, so never send it to
// a running device.
const CMD_FW_LOAD: u8 = 0xA0;
const CPUCS_ADDR: u16 = 0xE600; // 8051 CPUCS register (bit 0 = reset)

// --- fx2lafw vendor commands (EP0, recipient Device) -------------------
const CMD_GET_FW_VERSION: u8 = 0xB0; // IN, 2 bytes (major, minor)
const CMD_GET_REVID: u8 = 0xB2;      // IN, 1 byte
const CMD_START: u8 = 0xB1;          // OUT, 3-byte payload

// CMD_START flags byte (bit positions verified from the protocol description).
// FLAG_WIDE is kept for documentation / future 16-channel variants.
#[allow(dead_code)]
const FLAG_WIDE: u8 = 1 << 5; // 0 = 8-bit (8 ch), 1 = 16-bit (16 ch)
const FLAG_CLK_48: u8 = 1 << 6; // 0 = 30 MHz IFCLK, 1 = 48 MHz IFCLK

// --- Timing constants --------------------------------------------------
const CHUNK: usize = 1024; // firmware upload chunk size
const FW_PROBE_TIMEOUT: Duration = Duration::from_millis(200);
const CTRL_TIMEOUT: Duration = Duration::from_millis(500);
const REENUM_POLL: Duration = Duration::from_millis(100);
const REENUM_DEADLINE: Duration = Duration::from_secs(5);
// Let the 8051 boot + trigger USB re-enumeration before polling.
const REENUM_SETTLE: Duration = Duration::from_millis(300);

pub const VID_SALEAE: u16 = 0x0925;
pub const PID_SALEAE_LOGIC: u16 = 0x3881;

/// Sample rates supported by fx2lafw, all dividing 48 MHz (so all use the
/// 48 MHz IFCLK). Sanity: 24M→delay 1, 1M→47, 20k→2399 (all fit u16).
pub const SAMPLE_RATES: &[u32] = &[
    20_000, 25_000, 50_000, 100_000, 200_000, 250_000, 500_000,
    1_000_000, 2_000_000, 3_000_000, 4_000_000, 6_000_000,
    8_000_000, 12_000_000, 16_000_000, 24_000_000,
];

pub const DEFAULT_SAMPLERATE: u32 = 1_000_000;
pub const DEFAULT_LIMIT_SAMPLES: u64 = 1_000_000;

/// An fx2lafw-compatible device spotted on the bus.
#[derive(Debug, Clone)]
pub struct DeviceId {
    pub bus: u8,
    pub address: u8,
    pub vid: u16,
    pub pid: u16,
}

/// Open FX2 device: claimed interface 0 + open EP2 bulk-IN endpoint.
/// Public fields expose device identity for diagnostics / NOTES.
#[allow(dead_code)]
pub struct Fx2 {
    dev: Device,
    intf: Interface,
    ep_in: nusb::Endpoint<Bulk, In>,
    pub ep_in_addr: u8,
    pub samplerate: u32,
    pub fw_version: (u8, u8),
    pub revid: u8,
    pub bus: u8,
    pub address: u8,
}

/// Result of one bulk read iteration.
pub enum ReadResult {
    /// Raw sample bytes (1 byte/sample, bit i = channel D*i*).
    Data(Vec<u8>),
    /// Transfer timed out (no data in the window). Caller checks stop + retries.
    Timeout,
    /// Endpoint stalled; halt cleared, retry.
    Stall,
}

enum FwState {
    /// Bootloader: 0xA0 will be handled by the FX2 ROM; firmware not running.
    Bootloader,
    /// Any other probe error (likely transitional during re-enumeration).
    Other(String),
}

pub fn list_known() -> Vec<DeviceId> {
    let mut out = Vec::new();
    let Ok(list) = nusb::list_devices().wait() else { return out };
    for di in list {
        if is_known(di.vendor_id(), di.product_id()) {
            out.push(DeviceId {
                bus: di.busnum(),
                address: di.device_address(),
                vid: di.vendor_id(),
                pid: di.product_id(),
            });
        }
    }
    out
}

fn is_known(vid: u16, pid: u16) -> bool {
    matches!((vid, pid), (VID_SALEAE, PID_SALEAE_LOGIC))
}

impl Fx2 {
    /// Open the Saleae clone (0925:3881), uploading firmware if it's in the
    /// bootloader state. After return, the device is in capture-firmware mode,
    /// interface 0 is claimed, and the EP2 bulk-IN endpoint is open.
    pub fn open() -> Result<Self, String> {
        let first = list_known().into_iter().next()
            .ok_or_else(|| "no fx2lafw-compatible USB device found (expected \
                            0925:3881 Saleae Logic)".to_string())?;
        Self::open_at(&first)
    }

    /// Like `open` but limited to a specific bus/address (from `scan`).
    pub fn open_target(bus: u8, address: u8) -> Result<Self, String> {
        let target = list_known().into_iter()
            .find(|d| d.bus == bus && d.address == address)
            .ok_or_else(|| format!("no fx2lafw device at {bus:03}:{address:03}"))?;
        Self::open_at(&target)
    }

    fn open_at(target: &DeviceId) -> Result<Self, String> {
        let bus = target.bus;
        let address = target.address;
        let di = nusb::list_devices().wait()
            .map_err(|e| format!("enumerate: {e}"))?
            .find(|d| d.busnum() == target.bus
                       && d.device_address() == target.address
                       && d.vendor_id() == target.vid
                       && d.product_id() == target.pid)
            .ok_or_else(|| format!("device {:03}:{:03} vanished before open",
                                   target.bus, target.address))?;
        let dev = di.open().wait().map_err(|e| format!("open: {e}"))?;
        let intf = dev.claim_interface(0).wait()
            .map_err(|e| format!("claim interface 0: {e}"))?;

        match Self::probe_fw(&intf) {
            Ok(v) => {
                let (ep_in, ep_in_addr) = Self::open_bulk_in(&intf)?;
                let revid = Self::get_revid(&intf).unwrap_or(0);
                Ok(Fx2 { dev, intf, ep_in, ep_in_addr,
                         samplerate: 0, fw_version: v, revid, bus, address })
            }
            Err(FwState::Bootloader) => {
                let (_, fw_bytes) = firmware::locate()?;
                Self::upload_firmware(&intf, &fw_bytes)?;
                drop(intf);
                drop(dev);
                Self::wait_and_open_after_reset()
            }
            Err(FwState::Other(e)) => Err(e),
        }
    }

    /// After firmware upload + reset release, the device re-enumerates at a
    /// (likely) new bus address. Poll for a known VID:PID, open it, probe
    /// firmware: repeat until the firmware answers or the deadline expires.
    fn wait_and_open_after_reset() -> Result<Self, String> {
        let deadline = Instant::now() + REENUM_DEADLINE;
        thread::sleep(REENUM_SETTLE);
        while Instant::now() < deadline {
            let Ok(list) = nusb::list_devices().wait() else {
                thread::sleep(REENUM_POLL);
                continue;
            };
            for di in list {
                if !is_known(di.vendor_id(), di.product_id()) { continue }
                let bus = di.busnum();
                let address = di.device_address();
                // Try open + claim + probe. Any failure means the device is
                // in a transitional state — drop handles and retry next tick.
                let dev = match di.open().wait() { Ok(d) => d, Err(_) => continue };
                let intf = match dev.claim_interface(0).wait() {
                    Ok(i) => i, Err(_) => continue,
                };
                match Self::probe_fw(&intf) {
                    Ok(v) => {
                        let (ep_in, ep_in_addr) = Self::open_bulk_in(&intf)?;
                        let revid = Self::get_revid(&intf).unwrap_or(0);
                        return Ok(Fx2 { dev, intf, ep_in, ep_in_addr,
                                         samplerate: 0, fw_version: v, revid,
                                         bus, address });
                    }
                    Err(FwState::Bootloader) => continue, // reset didn't take
                    Err(FwState::Other(_)) => continue,
                }
            }
            thread::sleep(REENUM_POLL);
        }
        Err(format!("device did not re-enumerate as fx2lafw within {} s",
                    REENUM_DEADLINE.as_secs()))
    }

    /// Probe `CMD_GET_FW_VERSION`. Ok → firmware running; Stall/Fault/
    /// Cancelled (timeout) → bootloader (the FX2 ROM doesn't implement 0xB0,
    /// and on this bench it ignores the request rather than STALLing it);
    /// other error → transitional.
    fn probe_fw(intf: &Interface) -> Result<(u8, u8), FwState> {
        let r = intf.control_in(ControlIn {
            control_type: ControlType::Vendor,
            recipient: Recipient::Device,
            request: CMD_GET_FW_VERSION,
            value: 0, index: 0, length: 2,
        }, FW_PROBE_TIMEOUT).wait();
        match r {
            Ok(v) if v.len() >= 2 => Ok((v[0], v[1])),
            Ok(_) => Err(FwState::Bootloader),
            Err(TransferError::Stall) | Err(TransferError::Fault)
            | Err(TransferError::Cancelled) => Err(FwState::Bootloader),
            Err(e) => Err(FwState::Other(format!("GET_FW_VERSION: {e}"))),
        }
    }

    fn get_revid(intf: &Interface) -> Result<u8, String> {
        let v = intf.control_in(ControlIn {
            control_type: ControlType::Vendor,
            recipient: Recipient::Device,
            request: CMD_GET_REVID,
            value: 0, index: 0, length: 1,
        }, FW_PROBE_TIMEOUT).wait()
            .map_err(|e| format!("GET_REVID: {e}"))?;
        if v.is_empty() { return Err("GET_REVID: empty".into()); }
        Ok(v[0])
    }

    /// Upload the firmware blob via the Cypress 0xA0 bootloader.
    /// 1. Halt 8051 (CPUCS=0x01). 2. Write image chunks. 3. Release 8051.
    fn upload_firmware(intf: &Interface, fw: &[u8]) -> Result<(), String> {
        Self::cpucs(intf, 0x01)?;
        let mut addr: u16 = 0;
        for chunk in fw.chunks(CHUNK) {
            intf.control_out(ControlOut {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request: CMD_FW_LOAD,
                value: addr,
                index: 0,
                data: chunk,
            }, CTRL_TIMEOUT).wait()
                .map_err(|e| format!("firmware chunk @ {addr:#06x}: {e}"))?;
            addr = addr.wrapping_add(chunk.len() as u16);
        }
        Self::cpucs(intf, 0x00)?;
        Ok(())
    }

    fn cpucs(intf: &Interface, value: u8) -> Result<(), String> {
        intf.control_out(ControlOut {
            control_type: ControlType::Vendor,
            recipient: Recipient::Device,
            request: CMD_FW_LOAD,
            value: CPUCS_ADDR,
            index: 0,
            data: &[value],
        }, CTRL_TIMEOUT).wait()
            .map_err(|e| format!("CPUCS write 0x{value:02x}: {e}"))
    }

    /// Walk the current alt setting for a bulk IN endpoint; return its
    /// address + an open `Endpoint`. (Discovers rather than hardcoding 0x82
    /// so the code is robust to alternate descriptor layouts.)
    fn open_bulk_in(intf: &Interface) -> Result<(nusb::Endpoint<Bulk, In>, u8), String> {
        let desc = intf.descriptor()
            .ok_or_else(|| "no current interface descriptor".to_string())?;
        let ep_desc = desc.endpoints()
            .find(|e| e.transfer_type() == TransferType::Bulk
                       && e.direction() == Direction::In)
            .ok_or_else(|| "no bulk IN endpoint in current alt setting".to_string())?;
        let addr = ep_desc.address();
        let ep = intf.endpoint::<Bulk, In>(addr)
            .map_err(|e| format!("open bulk IN endpoint {addr:#04x}: {e}"))?;
        Ok((ep, addr))
    }

    /// Start streaming at `samplerate` Hz, 8-bit samples (8 channels).
    pub fn start(&self, samplerate: u32) -> Result<(), String> {
        let (flags, delay) = encode_start(samplerate)
            .ok_or_else(|| format!("unsupported samplerate {samplerate}; \
                                    must divide 30 or 48 MHz evenly"))?;
        let payload = [flags,
                       ((delay >> 8) & 0xff) as u8,
                       (delay & 0xff) as u8];
        self.intf.control_out(ControlOut {
            control_type: ControlType::Vendor,
            recipient: Recipient::Device,
            request: CMD_START,
            value: 0, index: 0,
            data: &payload,
        }, CTRL_TIMEOUT).wait()
            .map_err(|e| format!("CMD_START: {e}"))?;
        Ok(())
    }

    /// Stop streaming: cancel pending URBs, drain them, clear endpoint halt.
    /// fx2lafw has no explicit stop command — this is the documented stop.
    pub fn stop(&mut self) -> Result<(), String> {
        self.ep_in.cancel_all();
        while self.ep_in.pending() > 0 {
            let _ = self.ep_in.wait_next_complete(Duration::from_millis(200));
        }
        self.ep_in.clear_halt().wait()
            .map_err(|e| format!("clear_halt: {e}"))
    }

    /// Read one bulk buffer (blocking, with timeout). Returns Data on success,
    /// Timeout when no data arrived in the window (caller re-checks stop and
    /// retries), Stall after auto-clearing a halt.
    pub fn read_blocking(&mut self, buf_size: usize,
                         timeout: Duration) -> Result<ReadResult, String> {
        // transfer_blocking asserts no transfer is pending.
        let buf = Buffer::new(buf_size);
        let completion = self.ep_in.transfer_blocking(buf, timeout);
        match completion.status {
            Ok(()) => Ok(ReadResult::Data(completion.buffer.into_vec())),
            Err(TransferError::Cancelled) => Ok(ReadResult::Timeout),
            Err(TransferError::Stall) => {
                self.ep_in.clear_halt().wait()
                    .map_err(|e| format!("clear_halt after stall: {e}"))?;
                Ok(ReadResult::Stall)
            }
            Err(TransferError::Disconnected) => Err("device disconnected".into()),
            Err(e) => Err(format!("bulk transfer: {e}")),
        }
    }

    pub fn max_packet_size(&self) -> usize { self.ep_in.max_packet_size() }
}

/// Encode the (flags, delay) pair for the CMD_START payload at `rate` Hz,
/// 8-bit samples (FLAG_WIDE=0). Returns None if the rate doesn't divide
/// 48 MHz or 30 MHz evenly, or doesn't fit a 16-bit delay.
pub fn encode_start(rate: u32) -> Option<(u8, u32)> {
    if rate == 0 { return None }
    if 48_000_000 % rate == 0 {
        let delay = 48_000_000 / rate - 1;
        if delay > 0xffff { return None }
        Some((FLAG_CLK_48, delay))
    } else if 30_000_000 % rate == 0 {
        let delay = 30_000_000 / rate - 1;
        if delay > 0xffff { return None }
        Some((0, delay))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_ladder_encodes_cleanly() {
        for &r in SAMPLE_RATES {
            let (flags, delay) = encode_start(r)
                .unwrap_or_else(|| panic!("rate {r} should encode"));
            assert_eq!(flags, FLAG_CLK_48, "rate {r}: expected 48 MHz clk");
            assert!(delay <= 0xffff, "rate {r}: delay {delay} > u16");
        }
    }

    #[test]
    fn known_delays() {
        assert_eq!(encode_start(24_000_000), Some((FLAG_CLK_48, 1)));
        assert_eq!(encode_start(1_000_000), Some((FLAG_CLK_48, 47)));
        assert_eq!(encode_start(20_000), Some((FLAG_CLK_48, 2399)));
    }

    #[test]
    fn rejects_non_divisor() {
        assert_eq!(encode_start(7), None);
        assert_eq!(encode_start(0), None);
    }
}
