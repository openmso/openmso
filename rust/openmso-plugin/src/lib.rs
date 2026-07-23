// SPDX-License-Identifier: Apache-2.0
//! OCP (OpenMSO Capture Protocol) plugin-side library.
//!
//! Rust counterpart of `python/openmso`: NDJSON+binary framing, the JSON-RPC
//! serve loop for plugin processes, and device-side SCPI transports (raw TCP,
//! VXI-11, Linux usbtmc). See `docs/protocol.md` for the normative spec.

pub mod framing;
pub mod scpi;
pub mod server;
// USBTMC is implemented over Linux usbdevfs (/dev/usbtmc*, /dev/bus/usb,
// USBDEVFS_RESET) and uses std::os::unix, so it only compiles/works on Linux.
// Other platforms drive the same instruments over the network (TCP/VXI-11).
#[cfg(target_os = "linux")]
pub mod usbtmc;
pub mod vxi11;

pub const PROTOCOL_VERSION: i64 = 0;
