// SPDX-License-Identifier: Apache-2.0
//! Collects `capture.*` notifications arriving on the client's reader thread.
//!
//! The notification handler is a `Fn`, and it runs on a thread we do not own,
//! so state lives behind a mutex and `capture.end` is signalled by condvar.
//! Keep the work here to buffering — the main thread does the arithmetic.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

#[derive(Default)]
pub struct State {
    pub begin: Option<Value>,
    pub end: Option<Value>,
    pub trigger_sample: Option<i64>,
    /// stream -> (first_sample, payload); chunks may arrive out of order.
    chunks: HashMap<i64, Vec<(u64, Vec<u8>)>>,
    bytes: usize,
    started: Option<Instant>,
}

pub struct Sink {
    state: Mutex<State>,
    finished: Condvar,
    verbose: bool,
}

impl Sink {
    pub fn new(verbose: bool) -> Arc<Self> {
        Arc::new(Sink { state: Mutex::new(State::default()),
                        finished: Condvar::new(), verbose })
    }

    pub fn handle(&self, method: &str, params: &Value, payload: Option<&[u8]>) {
        match method {
            "log" => log_notification(params),
            "event.status" if self.verbose => {
                eprintln!("[{}]", params.get("state").and_then(Value::as_str).unwrap_or("?"));
            }
            "capture.begin" => {
                let mut s = self.state.lock().unwrap();
                s.begin = Some(params.clone());
                s.started = Some(Instant::now());
            }
            "capture.data" => {
                let stream = params.get("stream").and_then(Value::as_i64).unwrap_or(0);
                let first = params.get("first_sample").and_then(Value::as_u64).unwrap_or(0);
                let data = payload.unwrap_or_default().to_vec();
                let mut s = self.state.lock().unwrap();
                s.bytes += data.len();
                s.chunks.entry(stream).or_default().push((first, data));
            }
            "capture.trigger" => {
                let mut s = self.state.lock().unwrap();
                s.trigger_sample = params.get("sample").and_then(Value::as_i64);
            }
            "capture.end" => {
                let mut s = self.state.lock().unwrap();
                if self.verbose {
                    if let Some(t0) = s.started {
                        let dt = t0.elapsed().as_secs_f64();
                        let mb = s.bytes as f64 / 1e6;
                        eprintln!("[transfer: {mb:.2} MB in {dt:.2}s = {:.2} MB/s]",
                                  if dt > 0.0 { mb / dt } else { f64::INFINITY });
                    }
                }
                s.end = Some(params.clone());
                self.finished.notify_all();
            }
            _ => {}
        }
    }

    /// Block until `capture.end` arrives. Returns false on timeout.
    pub fn wait(&self, timeout: Duration) -> bool {
        let s = self.state.lock().unwrap();
        let (_guard, result) = self.finished
            .wait_timeout_while(s, timeout, |s| s.end.is_none())
            .unwrap();
        !result.timed_out()
    }

    pub fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }
}

impl State {
    /// Reassemble one stream in sample order.
    pub fn stream_bytes(&self, stream: i64) -> Vec<u8> {
        let mut parts = match self.chunks.get(&stream) {
            Some(p) => p.iter().collect::<Vec<_>>(),
            None => return Vec::new(),
        };
        parts.sort_by_key(|(first, _)| *first);
        let mut out = Vec::with_capacity(parts.iter().map(|(_, d)| d.len()).sum());
        for (_, data) in parts {
            out.extend_from_slice(data);
        }
        out
    }
}

pub fn log_notification(params: &Value) {
    eprintln!("[plugin:{}] {}",
              params.get("level").and_then(Value::as_str).unwrap_or("?"),
              params.get("message").and_then(Value::as_str).unwrap_or(""));
}

/// Decode raw device codes to real units: `value = raw * scale + offset`.
pub fn decode(raw: &[u8], dtype: &str, scale: f64, offset: f64) -> Result<Vec<f64>, String> {
    macro_rules! decode_le {
        ($ty:ty, $width:expr) => {{
            raw.chunks_exact($width)
                .map(|c| <$ty>::from_le_bytes(c.try_into().expect("chunk_exact width")) as f64)
                .collect::<Vec<f64>>()
        }};
    }
    let values: Vec<f64> = match dtype {
        "int8" => raw.iter().map(|&b| b as i8 as f64).collect(),
        "uint8" => raw.iter().map(|&b| b as f64).collect(),
        "int16" => decode_le!(i16, 2),
        "uint16" => decode_le!(u16, 2),
        "float32" => decode_le!(f32, 4),
        "float64" => decode_le!(f64, 8),
        other => return Err(format!("unsupported dtype {other:?}")),
    };
    Ok(values.into_iter().map(|v| v * scale + offset).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn int8_codes_scale_to_volts() {
        let raw = [0u8, 100u8, (-100i8) as u8];
        let v = decode(&raw, "int8", 0.01, 0.0).unwrap();
        assert_eq!(v.len(), 3);
        assert!((v[0] - 0.0).abs() < 1e-12);
        assert!((v[1] - 1.0).abs() < 1e-12);
        assert!((v[2] + 1.0).abs() < 1e-12);
    }

    #[test]
    fn offset_is_applied_after_scaling() {
        let v = decode(&[2], "uint8", 0.5, 10.0).unwrap();
        assert!((v[0] - 11.0).abs() < 1e-12);
    }

    #[test]
    fn wide_dtypes_decode_little_endian() {
        let raw = (-2i16).to_le_bytes();
        assert!((decode(&raw, "int16", 1.0, 0.0).unwrap()[0] + 2.0).abs() < 1e-12);
        let raw = 1.5f32.to_le_bytes();
        assert!((decode(&raw, "float32", 1.0, 0.0).unwrap()[0] - 1.5).abs() < 1e-12);
    }

    #[test]
    fn unknown_dtype_is_an_error_not_a_panic() {
        assert!(decode(&[0], "float16", 1.0, 0.0).is_err());
    }

    #[test]
    fn chunks_reassemble_in_sample_order_regardless_of_arrival() {
        let sink = Sink::new(false);
        let chunk = |first: u64, byte: u8| {
            sink.handle("capture.data",
                        &json!({"stream": 0, "first_sample": first}), Some(&[byte]));
        };
        chunk(2, b'c');
        chunk(0, b'a');
        chunk(1, b'b');
        assert_eq!(sink.state().stream_bytes(0), b"abc");
        // A stream that never produced data is empty, not an error.
        assert!(sink.state().stream_bytes(7).is_empty());
    }

    #[test]
    fn wait_times_out_when_capture_never_ends() {
        let sink = Sink::new(false);
        assert!(!sink.wait(Duration::from_millis(20)));
        sink.handle("capture.end", &json!({"ok": true}), None);
        assert!(sink.wait(Duration::from_millis(20)));
    }
}
