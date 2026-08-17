// SPDX-License-Identifier: Apache-2.0
//! Framed link to the Pico's USB-CDC port.
//!
//! The firmware speaks JSON-lines: one object per line, and a header carrying
//! `binlen` is followed by that many raw bytes. A reader thread routes replies
//! by id and pushes notifications to a channel.

use std::collections::HashMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

/// Enough for a slow analog capture to drain without the reader stalling.
const NOTIFY_QUEUE: usize = 1024;

pub struct Frame {
    pub msg: Value,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn method(&self) -> &str {
        self.msg.get("method").and_then(Value::as_str).unwrap_or_default()
    }

    pub fn params(&self) -> &Value {
        self.msg.get("params").unwrap_or(&Value::Null)
    }
}

#[derive(Debug)]
pub struct LinkError {
    /// The device's `ErrorCode` name, empty when the link itself failed.
    pub code: String,
    pub message: String,
}

impl LinkError {
    fn local(message: impl Into<String>) -> Self {
        LinkError { code: String::new(), message: message.into() }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

pub struct Link {
    writer: Mutex<File>,
    pending: Mutex<HashMap<u64, SyncSender<Frame>>>,
    next_id: AtomicU64,
}

impl Link {
    /// Open `path` raw at 115200 baud and start the reader thread. Never 1200:
    /// the firmware reads that line speed as a reboot-to-BOOTSEL request.
    pub fn open(path: &str) -> io::Result<(Arc<Link>, Receiver<Frame>)> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(path)?;
        set_raw(&file)?;
        let reader = file.try_clone()?;

        let (tx, rx) = sync_channel(NOTIFY_QUEUE);
        let link = Arc::new(Link {
            writer: Mutex::new(file),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        });

        let owned = Arc::clone(&link);
        thread::spawn(move || {
            read_loop(&owned, BufReader::new(reader), &tx);
            // Waking every waiter is what turns a dead port into an error
            // rather than a hang.
            owned.pending.lock().unwrap().clear();
        });
        Ok((link, rx))
    }

    pub fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, LinkError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = sync_channel(1);
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = self.write(&json!({ "id": id, "method": method, "params": params })) {
            self.pending.lock().unwrap().remove(&id);
            return Err(LinkError::local(format!("{method}: {e}")));
        }

        let frame = match rx.recv_timeout(timeout) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                return Err(LinkError::local(format!("device timed out on {method}")));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(LinkError::local("device disconnected"))
            }
        };
        if let Some(error) = frame.msg.get("error") {
            return Err(LinkError {
                code: error["code"].as_str().unwrap_or_default().to_string(),
                message: format!(
                    "{method}: {}",
                    error["message"].as_str().unwrap_or("device error")
                ),
            });
        }
        Ok(frame.msg.get("result").cloned().unwrap_or_else(|| json!({})))
    }

    /// Send a request without waiting for its reply, which the reader drops.
    /// `acquire.stop` needs this: the firmware only reads it between blocks,
    /// so a blocking call would sit behind the capture it is trying to end.
    pub fn send_nowait(&self, method: &str, params: Value) -> Result<(), LinkError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.write(&json!({ "id": id, "method": method, "params": params }))
            .map_err(|e| LinkError::local(format!("{method}: {e}")))
    }

    fn write(&self, msg: &Value) -> io::Result<()> {
        let mut line = serde_json::to_vec(msg)?;
        line.push(b'\n');
        let mut file = self.writer.lock().unwrap();
        file.write_all(&line)?;
        file.flush()
    }
}

fn read_loop(link: &Link, mut reader: BufReader<File>, notify: &SyncSender<Frame>) {
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let Ok(msg) = serde_json::from_slice::<Value>(&line) else { continue };

        let binlen = msg.get("binlen").and_then(Value::as_u64).unwrap_or(0) as usize;
        let mut payload = vec![0u8; binlen];
        if binlen > 0 && reader.read_exact(&mut payload).is_err() {
            return;
        }

        let frame = Frame { msg, payload };
        match frame.msg.get("id").and_then(Value::as_u64) {
            Some(id) => {
                let slot = link.pending.lock().unwrap().remove(&id);
                if let Some(slot) = slot {
                    slot.send(frame).ok();
                }
            }
            None => {
                if notify.send(frame).is_err() {
                    return;
                }
            }
        }
    }
}

fn set_raw(file: &File) -> io::Result<()> {
    let fd = file.as_raw_fd();
    let mut attr: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut attr) } != 0 {
        return Err(io::Error::last_os_error());
    }
    attr.c_iflag = 0;
    attr.c_oflag = 0;
    attr.c_lflag = 0;
    attr.c_cflag = (attr.c_cflag & !libc::CSIZE) | libc::CS8 | libc::CREAD | libc::CLOCAL;
    attr.c_cc[libc::VMIN] = 1;
    attr.c_cc[libc::VTIME] = 0;
    if unsafe { libc::cfsetspeed(&mut attr, libc::B115200) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &attr) } != 0 {
        return Err(io::Error::last_os_error());
    }
    unsafe { libc::tcflush(fd, libc::TCIOFLUSH) };
    Ok(())
}
