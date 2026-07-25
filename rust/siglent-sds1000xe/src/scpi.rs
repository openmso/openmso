// SPDX-License-Identifier: Apache-2.0
//! Device-side SCPI transports: raw TCP sockets plus the trait shared with
//! the VXI-11 and usbtmc implementations. Written from scratch (libsigrok's
//! src/scpi/ served as a behavioral reference only).
//!
//! IEEE 488.2 definite-length block parsing handles the `#<n><len><data>`
//! form used by waveform queries, tolerant of a text prefix before the `#`
//! (Siglent prepends e.g. `C1:WF DAT2,`).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Debug)]
pub struct ScpiError(pub String);

impl std::fmt::Display for ScpiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScpiError {}

impl From<std::io::Error> for ScpiError {
    fn from(e: std::io::Error) -> Self {
        ScpiError(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ScpiError>;

pub trait Scpi: Send {
    fn command(&mut self, cmd: &str) -> Result<()>;
    fn query(&mut self, cmd: &str) -> Result<String>;
    /// Send a query returning a definite-length block; return its bytes.
    fn query_block(&mut self, cmd: &str, timeout: Option<Duration>) -> Result<Vec<u8>>;
}

/// Locate a definite-length block header in `buf`.
///
/// Returns `(data_start, data_len)` if the full header is present, or `None`
/// if more bytes are needed. Errors if there is no '#' in a reasonable prefix.
pub fn find_block(buf: &[u8]) -> Result<Option<(usize, usize)>> {
    let Some(i) = buf.iter().position(|&b| b == b'#') else {
        if buf.len() > 64 {
            return Err(ScpiError(format!("no block header in response: {:?}",
                                         String::from_utf8_lossy(&buf[..64]))));
        }
        return Ok(None);
    };
    if buf.len() < i + 2 {
        return Ok(None);
    }
    let ndigits = buf[i + 1];
    if !ndigits.is_ascii_digit() {
        return Err(ScpiError(format!("bad block header at {:?}",
                                     &buf[i..buf.len().min(i + 8)])));
    }
    let ndigits = (ndigits - b'0') as usize;
    if buf.len() < i + 2 + ndigits {
        return Ok(None);
    }
    let dlen = std::str::from_utf8(&buf[i + 2..i + 2 + ndigits]).ok()
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(|| ScpiError(format!("bad block length at {:?}",
                                         &buf[i..i + 2 + ndigits])))?;
    Ok(Some((i + 2 + ndigits, dlen)))
}

/// Extract the numeric value from a reply like `SARA 1.00E+09Sa/s`.
pub fn scpi_float(text: &str) -> Result<f64> {
    let t = text.rsplit(',').next().unwrap_or(text);
    let b = t.as_bytes();
    for start in 0..b.len() {
        if let Some(len) = number_len(&b[start..]) {
            if let Ok(v) = t[start..start + len].parse::<f64>() {
                return Ok(v);
            }
        }
    }
    Err(ScpiError(format!("no number in reply: {text:?}")))
}

/// Length of a `[-+]?digits[.digits][eE[-+]digits]` number at the start of
/// `b`, or `None` if there isn't one.
fn number_len(b: &[u8]) -> Option<usize> {
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut have_digits = i > int_start;
    if i < b.len() && b[i] == b'.' {
        let frac_start = i + 1;
        let mut j = frac_start;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > frac_start {
            i = j;
            have_digits = true;
        } else if have_digits {
            i += 1; // trailing dot after integer part, e.g. "5."
        }
    }
    if !have_digits {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    Some(i)
}

/// Format a float the way SCPI gear likes it: `1.500000E+02` (Python `%.6E`).
pub fn fmt_scpi(v: f64) -> String {
    let s = format!("{v:.6e}");
    let (mantissa, exp) = s.split_once('e').expect("exponential format");
    let exp: i32 = exp.parse().expect("numeric exponent");
    format!("{}E{}{:02}", mantissa, if exp < 0 { '-' } else { '+' }, exp.abs())
}

/// SCPI over a raw TCP socket (e.g. Siglent port 5025).
pub struct TcpScpi {
    sock: TcpStream,
    timeout: Duration,
}

impl TcpScpi {
    pub fn new(host: &str, port: u16, timeout: Duration) -> Result<Self> {
        let addr = (host, port).to_socket_addrs()?
            .next()
            .ok_or_else(|| ScpiError(format!("cannot resolve {host}")))?;
        let sock = TcpStream::connect_timeout(&addr, timeout)?;
        sock.set_read_timeout(Some(timeout))?;
        Ok(TcpScpi { sock, timeout })
    }

    fn drain_terminator(&mut self, already: usize) -> Result<()> {
        // Siglent terminates blocks with "\n\n"; consume what wasn't already read.
        let want = 2usize.saturating_sub(already);
        if want == 0 {
            return Ok(());
        }
        self.sock.set_read_timeout(Some(Duration::from_millis(500)))?;
        let mut buf = [0u8; 2];
        let _ = self.sock.read(&mut buf[..want]); // timeout is fine here
        self.sock.set_read_timeout(Some(self.timeout))?;
        Ok(())
    }
}

impl Scpi for TcpScpi {
    fn command(&mut self, cmd: &str) -> Result<()> {
        self.sock.write_all(cmd.as_bytes())?;
        self.sock.write_all(b"\n")?;
        Ok(())
    }

    fn query(&mut self, cmd: &str) -> Result<String> {
        self.command(cmd)?;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        while !buf.ends_with(b"\n") {
            let n = self.sock.read(&mut chunk)?;
            if n == 0 {
                return Err(ScpiError(format!("connection closed during query {cmd:?}")));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Ok(String::from_utf8_lossy(&buf).trim().to_string())
    }

    fn query_block(&mut self, cmd: &str, timeout: Option<Duration>) -> Result<Vec<u8>> {
        self.command(cmd)?;
        self.sock.set_read_timeout(Some(timeout.unwrap_or(self.timeout)))?;
        let mut buf = Vec::new();
        let mut chunk = vec![0u8; 1 << 20];
        let (start, dlen) = loop {
            let n = self.sock.read(&mut chunk)?;
            if n == 0 {
                return Err(ScpiError(format!("connection closed reading block for {cmd:?}")));
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(loc) = find_block(&buf)? {
                break loc;
            }
        };
        let total = start + dlen;
        while buf.len() < total {
            let n = self.sock.read(&mut chunk)?;
            if n == 0 {
                return Err(ScpiError(format!(
                    "connection closed mid-block ({}/{} bytes)", buf.len(), total)));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let extra = buf.len() - total;
        buf.truncate(total);
        let data = buf.split_off(start);
        self.drain_terminator(extra)?;
        self.sock.set_read_timeout(Some(self.timeout))?;
        Ok(data)
    }
}

/// Open a transport from a connection URL:
/// `vxi11://host`, `tcp://host[:port]` or `usbtmc:///dev/usbtmc0`.
pub fn open_transport(connection: &str) -> Result<Box<dyn Scpi>> {
    if let Some(host) = connection.strip_prefix("vxi11://") {
        return Ok(Box::new(crate::vxi11::Vxi11Scpi::new(host, Duration::from_secs(10))?));
    }
    if let Some(rest) = connection.strip_prefix("tcp://") {
        let (host, port) = match rest.split_once(':') {
            Some((h, p)) => (h, p.parse::<u16>()
                .map_err(|_| ScpiError(format!("bad port in {connection:?}")))?),
            None => (rest, 5025),
        };
        return Ok(Box::new(TcpScpi::new(host, port, Duration::from_secs(5))?));
    }
    #[cfg(target_os = "linux")]
    if let Some(path) = connection.strip_prefix("usbtmc://") {
        return Ok(Box::new(crate::usbtmc::UsbTmcScpi::new(path)?));
    }
    Err(ScpiError(format!("unsupported connection: {connection:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_header_parsing() {
        // Siglent-style prefix before the '#'
        let buf = b"C1:WF DAT2,#9000000004abcd\n\n";
        let (start, dlen) = find_block(buf).unwrap().unwrap();
        assert_eq!(dlen, 4);
        assert_eq!(&buf[start..start + dlen], b"abcd");
    }

    #[test]
    fn block_header_incomplete() {
        assert!(find_block(b"C1:WF DAT2,#9").unwrap().is_none());
        assert!(find_block(b"C1:WF DAT2,").unwrap().is_none());
    }

    #[test]
    fn block_header_missing() {
        let long = vec![b'x'; 100];
        assert!(find_block(&long).is_err());
    }

    #[test]
    fn scpi_float_variants() {
        assert_eq!(scpi_float("SARA 1.00E+09Sa/s").unwrap(), 1.00e9);
        assert_eq!(scpi_float("1.00E-03").unwrap(), 1.0e-3);
        assert_eq!(scpi_float("TDIV 5.00E-04S").unwrap(), 5.0e-4);
        // First number wins (same as the Python regex) — fine in practice
        // because the plugin sets CHDR OFF and gets numeric-only replies.
        assert_eq!(scpi_float("10").unwrap(), 10.0);
        assert_eq!(scpi_float("C1:ATTN 10").unwrap(), 1.0);
        assert_eq!(scpi_float("SANU 1.40E+07pts").unwrap(), 1.4e7);
        assert_eq!(scpi_float("OFST -1.55E+00V").unwrap(), -1.55);
        assert!(scpi_float("no digits here").is_err());
    }

    #[test]
    fn fmt_scpi_matches_python() {
        assert_eq!(fmt_scpi(0.5), "5.000000E-01");
        assert_eq!(fmt_scpi(1.0), "1.000000E+00");
        assert_eq!(fmt_scpi(-1.55), "-1.550000E+00");
        assert_eq!(fmt_scpi(1e-9), "1.000000E-09");
        assert_eq!(fmt_scpi(150.0), "1.500000E+02");
    }
}
