// SPDX-License-Identifier: Apache-2.0
//! OpenMSO capture plugin for Cypress FX2 (fx2lafw) logic analyzers.
//!
//! Written from scratch against the fx2lafw wire-protocol description and
//! live observation of a Saleae Logic clone (0925:3881). libsigrok's
//! `fx2lafw.c` was consulted as a behavioral reference only; no GPL code is
//! included. See `docs/fx2-plan/README.md` §3 for the clean-room discipline.
//!
//! The fx2lafw firmware blob (GPL-2.0+) is NOT vendored: we read the user's
//! system-installed blob at runtime and upload it via the Cypress 0xA0
//! bootloader on every open.

mod firmware;
mod fx2;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Map, Value};

use openmso::server::{self, Ctx, CaptureServer, RpcError, BUSY, DEVICE_ERROR,
                              INVALID_PARAMS, UNSUPPORTED};

use fx2::{Fx2, ReadResult, SAMPLE_RATES, DEFAULT_LIMIT_SAMPLES, DEFAULT_SAMPLERATE};

const LOGIC_CHANNELS: [&str; 8] = ["D0", "D1", "D2", "D3", "D4", "D5", "D6", "D7"];
// 8 in-flight URBs × 512-byte HS packets is plenty of headroom for 24 MB/s
// while keeping latency bounded; matches the queued-transfer recommendation
// in docs/fx2-plan/README.md §9.
const BULK_BUF_SIZE: usize = 4096;
const BULK_TIMEOUT: Duration = Duration::from_millis(1000);
const DATA_FRAME_BYTES: usize = 4 * 1024 * 1024;

fn dev_err(e: impl Into<String>) -> RpcError {
    RpcError::new(DEVICE_ERROR, e)
}

fn invalid(msg: impl Into<String>) -> RpcError {
    RpcError::new(INVALID_PARAMS, msg)
}

fn device_entry(bus: u8, addr: u8, vid: u16, _pid: u16) -> Value {
    let conn = format!("usb://{bus:03}-{addr:03}");
    json!({
        "device_id": conn,
        "vendor": format!("0x{vid:04x}"),
        "model": "fx2lafw",
        "serial": null,
        "connection": conn,
        "firmware": null,
    })
}

type Dev = Arc<Mutex<Option<Fx2>>>;

struct Fx2Plugin {
    dev: Dev,
    device: Option<Value>,
    capture_id: u64,
    stop: Arc<AtomicBool>,
    acq: Option<thread::JoinHandle<()>>,
    // Plugin-side config (samplerate snaps to nearest legal ladder value).
    samplerate: u32,
    limit_samples: u64,
}

impl Fx2Plugin {
    fn new() -> Self {
        Fx2Plugin {
            dev: Arc::new(Mutex::new(None)),
            device: None,
            capture_id: 0,
            stop: Arc::new(AtomicBool::new(false)),
            acq: None,
            samplerate: DEFAULT_SAMPLERATE,
            limit_samples: DEFAULT_LIMIT_SAMPLES,
        }
    }

    fn require_open(&self) -> Result<(), RpcError> {
        if self.dev.lock().unwrap().is_some() {
            Ok(())
        } else {
            Err(dev_err("no device open"))
        }
    }

    // ------------------------------------------------------------------
    // scan / open / close
    // ------------------------------------------------------------------
    fn scan(&mut self, _params: &Value, ctx: &Arc<Ctx>) -> Result<Value, RpcError> {
        let mut devices = Vec::new();
        for d in fx2::list_known() {
            devices.push(device_entry(d.bus, d.address, d.vid, d.pid));
        }
        if devices.is_empty() {
            ctx.log("info", "no fx2lafw devices found (expected 0925:3881)");
        }
        Ok(json!({"devices": devices}))
    }

    fn open(&mut self, params: &Value) -> Result<Value, RpcError> {
        if self.dev.lock().unwrap().is_some() {
            return Err(RpcError::new(BUSY, "device already open"));
        }
        // Parse "usb://BBB-AAA" (zero-padded) from scan's device_id; fall back
        // to "any known device" if absent (defensive — frontends should pass
        // the id they got from scan).
        let (bus, addr) = params.get("device_id").and_then(Value::as_str)
            .and_then(|s| s.strip_prefix("usb://"))
            .and_then(|s| s.split_once('-'))
            .and_then(|(b, a)| {
                let b = u8::from_str_radix(b, 10).ok()?;
                let a = u8::from_str_radix(a, 10).ok()?;
                Some((b, a))
            })
            .unwrap_or((0, 0));
        let dev = if bus == 0 && addr == 0 {
            Fx2::open().map_err(dev_err)?
        } else {
            Fx2::open_target(bus, addr).map_err(dev_err)?
        };
        let entry = device_entry(dev.bus, dev.address, fx2::VID_SALEAE,
                                  fx2::PID_SALEAE_LOGIC);
        self.device = Some(entry.clone());
        *self.dev.lock().unwrap() = Some(dev);
        Ok(json!({"device": entry}))
    }

    fn release(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.acq.take() {
            let _ = handle.join();
        }
        if let Some(mut dev) = self.dev.lock().unwrap().take() {
            let _ = dev.stop();
        }
        self.device = None;
    }

    // ------------------------------------------------------------------
    // describe / config
    // ------------------------------------------------------------------
    fn describe(&self) -> Result<Value, RpcError> {
        self.require_open()?;
        let device = self.device.clone()
            .ok_or_else(|| dev_err("describe: no device open"))?;
        let channels: Vec<Value> = LOGIC_CHANNELS.iter().enumerate()
            .map(|(i, ch)| json!({"id": ch, "kind": "logic",
                                   "name": format!("D{i}"), "index": i}))
            .collect();
        let rate_choices: Vec<Value> = SAMPLE_RATES.iter()
            .map(|&r| json!(r)).collect();
        let config = json!({
            "samplerate": {"scope": "device", "type": "number", "unit": "Sa/s",
                           "choices": rate_choices, "get": true, "set": true},
            "limit_samples": {"scope": "device", "type": "number",
                              "get": true, "set": true},
            "enabled": {"scope": "logic", "type": "bool",
                        "get": true, "set": true},
        });
        Ok(json!({"device": device, "channels": channels, "config": config}))
    }

    fn config_get(&mut self, params: &Value) -> Result<Value, RpcError> {
        self.require_open()?;
        let channel = params.get("channel").and_then(Value::as_str);
        if channel.is_some() {
            // Per-channel: only `enabled` is meaningful for fx2lafw (the
            // device always samples all 8 bits; disabling is a host-side
            // filter). Report all channels enabled.
            let mut values = Map::new();
            values.insert("enabled".into(), json!(true));
            return Ok(json!({"values": values}));
        }
        let requested: Option<Vec<String>> = params.get("keys")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect());
        let default_keys: &[&str] = &["samplerate", "limit_samples"];
        let keys: Vec<String> = requested
            .unwrap_or_else(|| default_keys.iter().map(|s| s.to_string()).collect());
        let mut values = Map::new();
        for key in keys {
            let v = match key.as_str() {
                "samplerate" => json!(self.samplerate),
                "limit_samples" => json!(self.limit_samples),
                _ => return Err(invalid(format!("unknown key {key:?} in scope"))),
            };
            values.insert(key, v);
        }
        Ok(json!({"values": values}))
    }

    fn config_set(&mut self, params: &Value) -> Result<Value, RpcError> {
        self.require_open()?;
        let values = params.get("values").and_then(Value::as_object)
            .cloned().unwrap_or_default();
        let mut applied = Map::new();
        for (key, value) in values {
            let v = match key.as_str() {
                "samplerate" => {
                    let r = value.as_u64()
                        .ok_or_else(|| invalid("samplerate must be a number"))?;
                    let r = snap_samplerate(r as u32);
                    self.samplerate = r;
                    json!(r)
                }
                "limit_samples" => {
                    let n = value.as_u64()
                        .ok_or_else(|| invalid("limit_samples must be a number"))?;
                    self.limit_samples = n;
                    json!(n)
                }
                _ => return Err(invalid(format!("unknown key {key:?} in scope"))),
            };
            applied.insert(key, v);
        }
        Ok(json!({"applied": applied}))
    }

    // ------------------------------------------------------------------
    // acquisition
    // ------------------------------------------------------------------
    fn acquire_start(&mut self, params: &Value, ctx: &Arc<Ctx>) -> Result<Value, RpcError> {
        self.require_open()?;
        if let Some(handle) = &self.acq {
            if !handle.is_finished() {
                return Err(RpcError::new(BUSY, "acquisition already running"));
            }
        }
        let mode = params.get("mode").and_then(Value::as_str).unwrap_or("single");
        if mode != "single" && mode != "continuous" {
            return Err(RpcError::new(UNSUPPORTED, format!("mode {mode:?} not supported")));
        }
        if mode == "snapshot" {
            return Err(RpcError::new(UNSUPPORTED,
                "snapshot not supported (fx2lafw has no on-device memory)"));
        }
        self.capture_id += 1;
        let cid = self.capture_id;
        self.stop.store(false, Ordering::SeqCst);
        let (dev, stop, ctx, mode, rate, limit) = (
            self.dev.clone(), self.stop.clone(), ctx.clone(), mode.to_string(),
            self.samplerate, self.limit_samples,
        );
        self.acq = Some(thread::spawn(move || {
            if let Err(e) = acquire_inner(&dev, &stop, &ctx, cid, &mode, rate, limit) {
                ctx.notify("capture.end",
                           json!({"capture_id": cid, "ok": false, "error": e}), None);
            }
        }));
        Ok(json!({"capture_id": cid}))
    }
}

fn snap_samplerate(r: u32) -> u32 {
    // Snap to the nearest legal ladder value (matches siglent-sds1000xe coercion).
    SAMPLE_RATES.iter().copied()
        .min_by_key(|s| (*s as i64 - r as i64).abs())
        .unwrap_or(DEFAULT_SAMPLERATE)
}
// ------------------------------------------------------------------
// acquisition worker
// ------------------------------------------------------------------

fn acquire_inner(dev: &Dev, stop: &AtomicBool, ctx: &Ctx, cid: u64, mode: &str,
                 rate: u32, limit: u64) -> Result<(), String> {
    // Configure + start streaming on the device. We keep the lock only for
    // the setup; the bulk-read loop takes the lock per read so the serve
    // loop can still observe `acquire.stop` and we don't deadlock if a
    // config.get arrives mid-stream.
    {
        let mut guard = dev.lock().unwrap();
        let Some(d) = guard.as_mut() else { return Err("no device open".into()) };
        d.start(rate)?;
    }

    ctx.notify("event.status", json!({"state": "armed"}), None);
    ctx.notify("capture.begin", json!({
        "capture_id": cid, "samplerate": rate, "t0": 0,
        "streams": [
            {"stream": 0, "kind": "logic",
             "channels": LOGIC_CHANNELS.to_vec(),
             "encoding": {"unitsize": 1}}
        ]
    }), None);
    ctx.notify("event.status", json!({"state": "transferring"}), None);

    let mut first_sample: u64 = 0;
    let mut seq: u64 = 0;
    let mut collected: u64 = 0;
    let mps = {
        let guard = dev.lock().unwrap();
        guard.as_ref().map(|d| d.max_packet_size()).unwrap_or(512)
    };
    let buf_size = BULK_BUF_SIZE.max(mps).max(512);
    // Round up to a multiple of max_packet_size (nusb requires this for IN).
    let buf_size = (buf_size + mps - 1) / mps * mps;

    loop {
        if stop.load(Ordering::SeqCst) { break; }

        let result = {
            let mut guard = dev.lock().unwrap();
            match guard.as_mut() {
                Some(d) => d.read_blocking(buf_size, BULK_TIMEOUT),
                None => return Err("device closed during acquisition".into()),
            }
        }?;

        let data = match result {
            ReadResult::Data(b) => b,
            ReadResult::Timeout => continue,
            ReadResult::Stall => continue,
        };
        if data.is_empty() { continue; }

        // In single mode, truncate the final chunk to hit exactly `limit`.
        let n: u64;
        let mut data = data;
        if mode == "single" && collected + data.len() as u64 > limit {
            let keep = (limit - collected) as usize;
            data.truncate(keep);
            n = keep as u64;
        } else {
            n = data.len() as u64;
        }
        if data.is_empty() { break; }

        // Frame into <= DATA_FRAME_BYTES chunks; `first_sample` tracks the
        // absolute sample index across the whole capture.
        let mut off = 0;
        while off < data.len() {
            let part = &data[off..data.len().min(off + DATA_FRAME_BYTES)];
            ctx.notify("capture.data", json!({
                "capture_id": cid, "stream": 0, "seq": seq,
                "first_sample": first_sample + off as u64,
                "nsamples": part.len(),
            }), Some(part));
            seq += 1;
            off += part.len();
        }
        first_sample += n;
        collected += n;

        if mode == "single" && collected >= limit {
            break;
        }
    }

    // Stop streaming + clean up the endpoint. fx2lafw has no stop command;
    // cancelling URBs + clear_halt is the documented stop.
    {
        let mut guard = dev.lock().unwrap();
        if let Some(d) = guard.as_mut() {
            let _ = d.stop();
        }
    }

    let aborted = stop.load(Ordering::SeqCst);
    let mut end = json!({"capture_id": cid, "ok": !aborted});
    if aborted {
        end["error"] = json!("aborted");
    }
    ctx.notify("capture.end", end, None);
    ctx.notify("event.status", json!({"state": "idle"}), None);
    Ok(())
}

impl CaptureServer for Fx2Plugin {
    fn info(&self) -> Value {
        json!({"name": "generic-fx2", "version": "0.1.0", "vendor": "OpenMSO",
               "description": "Cypress FX2 (fx2lafw) logic analyzers"})
    }

    fn capabilities(&self) -> Value {
        json!({"scan": true, "modes": ["continuous", "single"],
               "raw": false, "trigger_forms": []})
    }

    fn handle(&mut self, method: &str, params: &Value, _payload: Option<Vec<u8>>,
              ctx: &Arc<Ctx>) -> Result<Value, RpcError> {
        match method {
            "scan" => self.scan(params, ctx),
            "open" => self.open(params),
            "close" => {
                self.release();
                Ok(json!({}))
            }
            "describe" => self.describe(),
            "config.get" => self.config_get(params),
            "config.set" => self.config_set(params),
            "acquire.start" => self.acquire_start(params, ctx),
            "acquire.stop" => {
                self.stop.store(true, Ordering::SeqCst);
                Ok(json!({}))
            }
            _ => Err(RpcError::method_not_found(method)),
        }
    }

    fn on_disconnect(&mut self) {
        self.release();
    }
}

fn main() {
    let mut plugin = Fx2Plugin::new();
    server::run_from_argv(&mut plugin);
}
