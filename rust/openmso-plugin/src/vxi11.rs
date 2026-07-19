// SPDX-License-Identifier: Apache-2.0
//! Minimal VXI-11 (TCP/IP Instrument Protocol) client.
//!
//! Implements just enough ONC-RPC (RFC 1057/5531) and VXI-11 (VXIbus TC
//! specification) to drive an instrument: portmapper GETPORT, create_link,
//! device_write, device_read, destroy_link. No dependencies; written from
//! the public specifications.
//!
//! Practical motivation: Siglent SDS1000X-E raw-socket SCPI (port 5025) is
//! fragile and can crash, taking 5024/5025 down until reboot; the VXI-11
//! service is a separate, more robust path and is what most vendor software
//! uses.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crate::scpi::{find_block, Result, Scpi, ScpiError};

const PMAP_PROG: u32 = 100000;
const PMAP_VERS: u32 = 2;
const PMAP_GETPORT: u32 = 3;
const CORE_PROG: u32 = 395183;
const CORE_VERS: u32 = 1;
const PROC_CREATE_LINK: u32 = 10;
const PROC_DEV_WRITE: u32 = 11;
const PROC_DEV_READ: u32 = 12;
const PROC_DESTROY_LINK: u32 = 23;
const IPPROTO_TCP: u32 = 6;

// device_read 'reason' bits
const REASON_CHR: u32 = 2; // termchar seen
const REASON_END: u32 = 4; // END indicator (end of message)

const WRITE_FLAG_END: u32 = 8;

fn err(msg: impl Into<String>) -> ScpiError {
    ScpiError(msg.into())
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn read_u32(buf: &[u8], off: usize) -> Result<u32> {
    let bytes: [u8; 4] = buf.get(off..off + 4)
        .ok_or_else(|| err("short RPC reply"))?
        .try_into().unwrap();
    Ok(u32::from_be_bytes(bytes))
}

fn opaque(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + data.len() + 3);
    push_u32(&mut out, data.len() as u32);
    out.extend_from_slice(data);
    out.resize(out.len() + (4 - data.len() % 4) % 4, 0);
    out
}

fn next_xid() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    if COUNTER.load(Ordering::Relaxed) == 0 {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() | 1)
            .unwrap_or(1);
        let _ = COUNTER.compare_exchange(0, seed, Ordering::Relaxed, Ordering::Relaxed);
    }
    COUNTER.fetch_add(1, Ordering::Relaxed) & 0x7FFF_FFFF
}

/// One ONC-RPC connection with record-marking framing.
struct RpcChannel {
    sock: TcpStream,
}

impl RpcChannel {
    fn connect(host: &str, port: u16, timeout: Duration) -> Result<Self> {
        let addr = (host, port).to_socket_addrs()?
            .next()
            .ok_or_else(|| err(format!("cannot resolve {host}")))?;
        let sock = TcpStream::connect_timeout(&addr, timeout)?;
        sock.set_read_timeout(Some(timeout))?;
        Ok(RpcChannel { sock })
    }

    fn call(&mut self, prog: u32, vers: u32, proc: u32, args: &[u8]) -> Result<Vec<u8>> {
        let xid = next_xid();
        let mut record = Vec::with_capacity(40 + args.len());
        for v in [xid, 0, 2, prog, vers, proc] {
            push_u32(&mut record, v);
        }
        record.extend_from_slice(&[0u8; 16]); // AUTH_NULL cred + verf
        record.extend_from_slice(args);
        let mut framed = Vec::with_capacity(4 + record.len());
        push_u32(&mut framed, 0x8000_0000 | record.len() as u32);
        framed.extend_from_slice(&record);
        self.sock.write_all(&framed)?;

        let reply = self.read_record()?;
        let rxid = read_u32(&reply, 0)?;
        let mtype = read_u32(&reply, 4)?;
        let rstat = read_u32(&reply, 8)?;
        if rxid != xid || mtype != 1 {
            return Err(err(format!("bad RPC reply (xid {rxid:#x} vs {xid:#x})")));
        }
        if rstat != 0 {
            return Err(err(format!("RPC call rejected (stat {rstat})")));
        }
        // skip verf (flavor + length + body), then accept_stat
        let vlen = read_u32(&reply, 16)? as usize;
        let off = 20 + vlen + (4 - vlen % 4) % 4;
        let astat = read_u32(&reply, off)?;
        if astat != 0 {
            return Err(err(format!("RPC accept_stat {astat}")));
        }
        Ok(reply[off + 4..].to_vec())
    }

    fn read_record(&mut self) -> Result<Vec<u8>> {
        let mut record = Vec::new();
        loop {
            let mut head = [0u8; 4];
            self.sock.read_exact(&mut head)?;
            let mark = u32::from_be_bytes(head);
            let frag_len = (mark & 0x7FFF_FFFF) as usize;
            let start = record.len();
            record.resize(start + frag_len, 0);
            self.sock.read_exact(&mut record[start..])?;
            if mark & 0x8000_0000 != 0 {
                return Ok(record);
            }
        }
    }
}

/// A single VXI-11 instrument link (device "inst0").
pub struct Vxi11Client {
    chan: RpcChannel,
    lid: u32,
    max_recv: u32,
    io_timeout_ms: u32,
    timeout: Duration,
}

impl Vxi11Client {
    pub fn new(host: &str, timeout: Duration) -> Result<Self> {
        let port = {
            let mut pmap = RpcChannel::connect(host, 111, timeout)?;
            let mut args = Vec::new();
            for v in [CORE_PROG, CORE_VERS, IPPROTO_TCP, 0] {
                push_u32(&mut args, v);
            }
            let r = pmap.call(PMAP_PROG, PMAP_VERS, PMAP_GETPORT, &args)?;
            read_u32(&r, 0)?
        };
        if port == 0 {
            return Err(err("instrument does not register VXI-11 core channel"));
        }
        let mut chan = RpcChannel::connect(host, port as u16, timeout)?;
        // create_link args: clientId(int), lockDevice(bool), lock_timeout(u32)
        let mut args = Vec::new();
        for v in [1u32, 0, 0] {
            push_u32(&mut args, v);
        }
        args.extend_from_slice(&opaque(b"inst0"));
        let r = chan.call(CORE_PROG, CORE_VERS, PROC_CREATE_LINK, &args)?;
        let error = read_u32(&r, 0)?;
        if error != 0 {
            return Err(err(format!("create_link failed (error {error})")));
        }
        Ok(Vxi11Client {
            chan,
            lid: read_u32(&r, 4)?,
            max_recv: read_u32(&r, 12)?,
            io_timeout_ms: timeout.as_millis() as u32,
            timeout,
        })
    }

    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        let max_chunk = (self.max_recv.max(1024)) as usize;
        let mut off = 0;
        loop {
            let chunk = &data[off..data.len().min(off + max_chunk)];
            off += chunk.len();
            let end = if off >= data.len() { WRITE_FLAG_END } else { 0 };
            let mut args = Vec::with_capacity(16 + chunk.len() + 8);
            for v in [self.lid, self.io_timeout_ms, 0, end] {
                push_u32(&mut args, v);
            }
            args.extend_from_slice(&opaque(chunk));
            let r = self.chan.call(CORE_PROG, CORE_VERS, PROC_DEV_WRITE, &args)?;
            let error = read_u32(&r, 0)?;
            if error != 0 {
                return Err(err(format!("device_write failed (error {error})")));
            }
            if off >= data.len() {
                return Ok(());
            }
        }
    }

    /// Read one complete message (until END indicator).
    pub fn read(&mut self, request_size: u32, io_timeout_ms: Option<u32>) -> Result<Vec<u8>> {
        let io_ms = io_timeout_ms.unwrap_or(self.io_timeout_ms);
        // Give the socket a little more patience than the instrument-side
        // io_timeout so a slow first byte doesn't kill the RPC channel.
        if io_ms > self.io_timeout_ms {
            self.chan.sock.set_read_timeout(
                Some(Duration::from_millis(io_ms as u64 + 5000)))?;
        }
        let mut out = Vec::new();
        let result = loop {
            let mut args = Vec::new();
            for v in [self.lid, request_size, io_ms, 0, 0, 0] {
                push_u32(&mut args, v);
            }
            let r = match self.chan.call(CORE_PROG, CORE_VERS, PROC_DEV_READ, &args) {
                Ok(r) => r,
                Err(e) => break Err(e),
            };
            let header: Result<[u32; 3]> = (|| {
                Ok([read_u32(&r, 0)?, read_u32(&r, 4)?, read_u32(&r, 8)?])
            })();
            let [error, reason, dlen] = match header {
                Ok(h) => h,
                Err(e) => break Err(e),
            };
            if error != 0 {
                break Err(err(format!("device_read failed (error {error})")));
            }
            let dlen = dlen as usize;
            if r.len() < 12 + dlen {
                break Err(err("short device_read payload"));
            }
            out.extend_from_slice(&r[12..12 + dlen]);
            if reason & (REASON_END | REASON_CHR) != 0 {
                break Ok(out);
            }
            if reason == 0 && dlen == 0 {
                break Err(err("device_read returned no data, no reason"));
            }
        };
        if io_ms > self.io_timeout_ms {
            self.chan.sock.set_read_timeout(Some(self.timeout))?;
        }
        result
    }
}

impl Drop for Vxi11Client {
    fn drop(&mut self) {
        let mut args = Vec::new();
        push_u32(&mut args, self.lid);
        let _ = self.chan.call(CORE_PROG, CORE_VERS, PROC_DESTROY_LINK, &args);
    }
}

/// SCPI transport over VXI-11, matching the TcpScpi/UsbTmcScpi interface.
pub struct Vxi11Scpi {
    cli: Vxi11Client,
}

impl Vxi11Scpi {
    pub fn new(host: &str, timeout: Duration) -> Result<Self> {
        Ok(Vxi11Scpi { cli: Vxi11Client::new(host, timeout)? })
    }
}

impl Scpi for Vxi11Scpi {
    fn command(&mut self, cmd: &str) -> Result<()> {
        self.cli.write(format!("{cmd}\n").as_bytes())
    }

    fn query(&mut self, cmd: &str) -> Result<String> {
        self.command(cmd)?;
        let reply = self.cli.read(65536, None)?;
        Ok(String::from_utf8_lossy(&reply).trim().to_string())
    }

    fn query_block(&mut self, cmd: &str, timeout: Option<Duration>) -> Result<Vec<u8>> {
        self.command(cmd)?;
        let io_ms = timeout.map(|t| t.as_millis() as u32);
        let buf = self.cli.read(1 << 22, io_ms)?;
        let (start, dlen) = find_block(&buf)?
            .ok_or_else(|| err(format!("incomplete block reply for {cmd:?}")))?;
        if buf.len() < start + dlen {
            return Err(err(format!("short block: have {}/{dlen}", buf.len() - start)));
        }
        Ok(buf[start..start + dlen].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_padding() {
        assert_eq!(opaque(b"inst0"),
                   [&[0, 0, 0, 5][..], b"inst0", &[0, 0, 0][..]].concat());
        assert_eq!(opaque(b"abcd"), [&[0, 0, 0, 4][..], b"abcd"].concat());
        assert_eq!(opaque(b""), vec![0, 0, 0, 0]);
    }

    #[test]
    fn xids_are_unique_and_positive() {
        let a = next_xid();
        let b = next_xid();
        assert_ne!(a, b);
        assert_eq!(a & 0x8000_0000, 0);
    }
}
