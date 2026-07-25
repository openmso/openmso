// SPDX-License-Identifier: Apache-2.0
//! OpenMSO capture plugin for Siglent SDS1000X-E series oscilloscopes.
//!
//! Written from scratch against the Siglent "Digital Oscilloscope Series
//! Programming Guide" (EN02E). libsigrok's siglent-sds driver and PR #247
//! were consulted as behavioral references only; no GPL code is included.
//!
//! Rust port of the Python plugin that was verified live on an SDS1104X-E
//! (firmware 8.3.6.1.37R8) over VXI-11 and raw TCP :5025 — wire-compatible
//! with it, message for message. The digital (D0-D15 / SLA1016) path follows
//! the documentation but has not been exercised on hardware — channels are
//! advertised with "untested": true.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

// Device-side SCPI transports (raw TCP, VXI-11, Linux usbtmc). Only this plugin
// uses them, so they live here rather than in a shared crate.
mod scpi;
mod vxi11;
#[cfg(target_os = "linux")]
mod usbtmc;

use crate::scpi::{fmt_scpi, open_transport, scpi_float, Scpi};
use openmso::server::{self, Ctx, CaptureServer, RpcError, BUSY, DEVICE_ERROR,
                             INVALID_PARAMS, UNSUPPORTED};

const VDIVS: [f64; 14] = [500e-6, 1e-3, 2e-3, 5e-3, 10e-3, 20e-3, 50e-3,
                          100e-3, 200e-3, 500e-3, 1.0, 2.0, 5.0, 10.0];
const TDIVS: [f64; 34] = [1e-9, 2e-9, 5e-9, 10e-9, 20e-9, 50e-9, 100e-9, 200e-9,
                          500e-9, 1e-6, 2e-6, 5e-6, 10e-6, 20e-6, 50e-6, 100e-6,
                          200e-6, 500e-6, 1e-3, 2e-3, 5e-3, 10e-3, 20e-3, 50e-3,
                          100e-3, 200e-3, 500e-3, 1.0, 2.0, 5.0, 10.0, 20.0,
                          50.0, 100.0];
const MEMORY_DEPTHS: [&str; 4] = ["14K", "140K", "1.4M", "14M"]; // interleave-mode values
const PROBE_FACTORS: [f64; 5] = [0.1, 1.0, 10.0, 100.0, 1000.0];
const HORIZ_DIVS: f64 = 14.0;
const CODES_PER_DIV: f64 = 25.0;
const ANALOG_CHANNELS: [&str; 4] = ["C1", "C2", "C3", "C4"];
// Verified live: a full 14 Mpt (14 MB) block reads fine in one WF?
// transaction at ~6.5 MB/s on both TCP and VXI-11, and paging via WFSU NP/FP
// also works but costs ~35% throughput — so read the whole depth in one shot.
// The paging path below remains for models with deeper memory.
const PAGE_SAMPLES: usize = 14_000_363; // SDS1000X-E max buffer
const DATA_FRAME_BYTES: usize = 4 * 1024 * 1024;

fn coupling_to_scpi(c: &str) -> Option<&'static str> {
    match c {
        "ac" => Some("A1M"),
        "dc" => Some("D1M"),
        "gnd" => Some("GND"),
        _ => None,
    }
}

fn scpi_to_coupling(s: &str) -> &'static str {
    match s {
        "A1M" => "ac",
        "GND" => "gnd",
        _ => "dc",
    }
}

fn fmt_volts(v: f64) -> String {
    format!("{}V", fmt_scpi(v))
}

type Dev = Arc<Mutex<Option<Box<dyn Scpi>>>>;

/// Run one or more SCPI transactions under the device lock (serializes
/// access between the serve loop and the acquisition worker).
fn with_dev<T>(dev: &Dev, f: impl FnOnce(&mut dyn Scpi) -> crate::scpi::Result<T>)
               -> Result<T, String> {
    let mut guard = dev.lock().unwrap();
    match guard.as_mut() {
        Some(d) => f(d.as_mut()).map_err(|e| e.to_string()),
        None => Err("no device open".into()),
    }
}

fn q(dev: &Dev, cmd: &str) -> Result<String, String> {
    with_dev(dev, |d| d.query(cmd))
}

fn c(dev: &Dev, cmd: &str) -> Result<(), String> {
    with_dev(dev, |d| d.command(cmd))
}

fn dev_err(e: String) -> RpcError {
    RpcError::new(DEVICE_ERROR, e)
}

fn invalid(msg: impl Into<String>) -> RpcError {
    RpcError::new(INVALID_PARAMS, msg)
}

fn device_entry(connection: &str, idn: &str) -> Value {
    let parts: Vec<&str> = idn.split(',').map(str::trim).collect();
    let get = |i: usize| parts.get(i).copied().unwrap_or("?");
    json!({
        "device_id": connection,
        "vendor": get(0),
        "model": get(1),
        "serial": get(2),
        "connection": connection,
        "firmware": parts.get(3).copied(),
    })
}

struct SdsPlugin {
    dev: Dev,
    idn: Option<Value>,
    capture_id: u64,
    stop: Arc<AtomicBool>,
    acq: Option<thread::JoinHandle<()>>,
}

impl SdsPlugin {
    fn new() -> Self {
        SdsPlugin {
            dev: Arc::new(Mutex::new(None)),
            idn: None,
            capture_id: 0,
            stop: Arc::new(AtomicBool::new(false)),
            acq: None,
        }
    }

    fn require_open(&self) -> Result<&Value, RpcError> {
        self.idn.as_ref().ok_or_else(|| dev_err("no device open".into()))
    }

    // ------------------------------------------------------------------
    // scan / open / close
    // ------------------------------------------------------------------
    fn scan(&mut self, params: &Value, ctx: &Arc<Ctx>) -> Result<Value, RpcError> {
        let mut devices = Vec::new();
        let addr = params.get("hints")
            .and_then(|h| h.get("address"))
            .and_then(Value::as_str);
        if let Some(addr) = addr {
            let (host, port) = match addr.split_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (addr, None),
            };
            // VXI-11 first: the X-E's raw-socket service (5025) is fragile
            // and once crashed stays down until reboot; VXI-11 is the path
            // vendor software uses.
            let mut probes = Vec::new();
            if port.is_none() {
                probes.push(format!("vxi11://{host}"));
            }
            probes.push(format!("tcp://{host}:{}", port.unwrap_or("5025")));
            for conn in probes {
                let idn = open_transport(&conn).and_then(|mut t| t.query("*IDN?"));
                match idn {
                    Ok(idn) => {
                        devices.push(device_entry(&conn, &idn));
                        break;
                    }
                    Err(e) => ctx.log("warning", &format!("scan {conn}: {e}")),
                }
            }
        }
        for path in usbtmc_paths() {
            let conn = format!("usbtmc://{path}");
            match open_transport(&conn).and_then(|mut t| t.query("*IDN?")) {
                Ok(idn) => {
                    if idn.contains("SDS1") || idn.contains("SDS2") {
                        devices.push(device_entry(&conn, &idn));
                    }
                }
                Err(e) if e.0.contains("ermission denied") => {
                    ctx.log("warning", &format!(
                        "{path}: no permission (install udev rule, see \
                         plugins/siglent-sds1000xe/99-openmso-usbtmc.rules)"));
                }
                Err(e) => ctx.log("warning", &format!("scan {path}: {e}")),
            }
        }
        Ok(json!({"devices": devices}))
    }

    fn open(&mut self, params: &Value) -> Result<Value, RpcError> {
        if self.dev.lock().unwrap().is_some() {
            return Err(RpcError::new(BUSY, "device already open"));
        }
        let connection = params.get("device_id").and_then(Value::as_str)
            .ok_or_else(|| invalid("device_id required"))?;
        let (dev, idn) = (|| {
            let mut dev = open_transport(connection)?;
            let idn = dev.query("*IDN?")?;
            dev.command("CHDR OFF")?; // numeric-only replies from here on
            Ok::<_, crate::scpi::ScpiError>((dev, idn))
        })().map_err(|e| RpcError::new(DEVICE_ERROR, format!("open failed: {e}")))?;
        *self.dev.lock().unwrap() = Some(dev);
        let entry = device_entry(connection, &idn);
        self.idn = Some(entry.clone());
        Ok(json!({"device": entry}))
    }

    fn release(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.acq.take() {
            let _ = handle.join();
        }
        self.dev.lock().unwrap().take();
        self.idn = None;
    }

    // ------------------------------------------------------------------
    // describe / config
    // ------------------------------------------------------------------
    fn describe(&self) -> Result<Value, RpcError> {
        let device = self.require_open()?;
        let mut channels: Vec<Value> = ANALOG_CHANNELS.iter().enumerate()
            .map(|(i, ch)| json!({"id": ch, "kind": "analog",
                                  "name": format!("CH{}", i + 1), "index": i}))
            .collect();
        channels.extend((0..16).map(|i| {
            json!({"id": format!("D{i}"), "kind": "logic",
                   "name": format!("D{i}"), "index": i, "untested": true})
        }));
        let config = json!({
            "timebase": {"scope": "device", "type": "number", "unit": "s/div",
                         "choices": TDIVS.to_vec(), "get": true, "set": true},
            "samplerate": {"scope": "device", "type": "number", "unit": "Sa/s",
                           "get": true, "set": false},
            "memory_depth": {"scope": "device", "type": "string",
                             "choices": MEMORY_DEPTHS.to_vec(), "get": true, "set": true},
            "trigger": {"scope": "device", "type": "object",
                        "get": true, "set": true},
            "enabled": {"scope": "analog", "type": "bool",
                        "get": true, "set": true},
            "vdiv": {"scope": "analog", "type": "number", "unit": "V/div",
                     "choices": VDIVS.to_vec(), "get": true, "set": true},
            "offset": {"scope": "analog", "type": "number", "unit": "V",
                       "get": true, "set": true},
            "coupling": {"scope": "analog", "type": "string",
                         "choices": ["ac", "dc", "gnd"], "get": true, "set": true},
            "probe_factor": {"scope": "analog", "type": "number",
                             "choices": PROBE_FACTORS.to_vec(), "get": true, "set": true},
        });
        Ok(json!({"device": device, "channels": channels, "config": config}))
    }

    fn config_get(&mut self, params: &Value) -> Result<Value, RpcError> {
        self.require_open()?;
        let dev = &self.dev;
        let channel = params.get("channel").and_then(Value::as_str);
        let requested: Option<Vec<String>> = params.get("keys")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect());
        let default_keys: &[&str] = if channel.is_some() {
            &["enabled", "vdiv", "offset", "coupling", "probe_factor"]
        } else {
            &["timebase", "samplerate", "memory_depth", "trigger"]
        };
        let keys: Vec<String> = requested
            .unwrap_or_else(|| default_keys.iter().map(|s| s.to_string()).collect());

        if let Some(ch) = channel {
            if !ANALOG_CHANNELS.contains(&ch) {
                return Err(invalid(format!("unknown channel {ch}")));
            }
            let mut values = Map::new();
            for key in keys {
                let v = match key.as_str() {
                    "enabled" => json!(q(dev, &format!("{ch}:TRA?"))
                        .map_err(dev_err)?.to_uppercase().ends_with("ON")),
                    "vdiv" => json!(query_float(dev, &format!("{ch}:VDIV?"))?),
                    "offset" => json!(query_float(dev, &format!("{ch}:OFST?"))?),
                    "coupling" => json!(scpi_to_coupling(
                        q(dev, &format!("{ch}:CPL?")).map_err(dev_err)?.trim())),
                    "probe_factor" => json!(query_float(dev, &format!("{ch}:ATTN?"))?),
                    _ => return Err(invalid(format!("unknown key {key:?} in scope"))),
                };
                values.insert(key, v);
            }
            return Ok(json!({"values": values}));
        }
        let mut values = Map::new();
        for key in keys {
            let v = match key.as_str() {
                "timebase" => json!(query_float(dev, "TDIV?")?),
                "samplerate" => json!(query_float(dev, "SARA?")?),
                "memory_depth" => json!(q(dev, "MSIZ?").map_err(dev_err)?.trim()),
                "trigger" => trigger_get(dev).map_err(dev_err)?,
                _ => return Err(invalid(format!("unknown key {key:?} in scope"))),
            };
            values.insert(key, v);
        }
        Ok(json!({"values": values}))
    }

    fn config_set(&mut self, params: &Value) -> Result<Value, RpcError> {
        self.require_open()?;
        let channel = params.get("channel").and_then(Value::as_str);
        let values = params.get("values").and_then(Value::as_object)
            .cloned().unwrap_or_default();
        let mut applied = Map::new();
        for (key, value) in values {
            applied.insert(key.clone(), self.set_one(channel, &key, &value)?);
        }
        Ok(json!({"applied": applied}))
    }

    fn set_one(&mut self, channel: Option<&str>, key: &str, value: &Value)
               -> Result<Value, RpcError> {
        let dev = &self.dev;
        if let Some(ch) = channel {
            if !ANALOG_CHANNELS.contains(&ch) {
                return Err(invalid(format!("unknown channel {ch}")));
            }
            match key {
                "enabled" => {
                    let on = value.as_bool()
                        .ok_or_else(|| invalid("enabled must be a bool"))?;
                    c(dev, &format!("{ch}:TRA {}", if on { "ON" } else { "OFF" }))
                        .map_err(dev_err)?;
                    return Ok(json!(q(dev, &format!("{ch}:TRA?"))
                        .map_err(dev_err)?.to_uppercase().ends_with("ON")));
                }
                "vdiv" => {
                    let v = value.as_f64().ok_or_else(|| invalid("vdiv must be a number"))?;
                    c(dev, &format!("{ch}:VDIV {}", fmt_scpi(v))).map_err(dev_err)?;
                    return Ok(json!(query_float(dev, &format!("{ch}:VDIV?"))?));
                }
                "offset" => {
                    let v = value.as_f64().ok_or_else(|| invalid("offset must be a number"))?;
                    c(dev, &format!("{ch}:OFST {}", fmt_scpi(v))).map_err(dev_err)?;
                    return Ok(json!(query_float(dev, &format!("{ch}:OFST?"))?));
                }
                "coupling" => {
                    let name = value.as_str().unwrap_or("");
                    let scpi = coupling_to_scpi(name)
                        .ok_or_else(|| invalid(format!("coupling {value}")))?;
                    c(dev, &format!("{ch}:CPL {scpi}")).map_err(dev_err)?;
                    return Ok(json!(scpi_to_coupling(
                        q(dev, &format!("{ch}:CPL?")).map_err(dev_err)?.trim())));
                }
                "probe_factor" => {
                    let v = value.as_f64()
                        .ok_or_else(|| invalid("probe_factor must be a number"))?;
                    c(dev, &format!("{ch}:ATTN {v}")).map_err(dev_err)?;
                    return Ok(json!(query_float(dev, &format!("{ch}:ATTN?"))?));
                }
                _ => {}
            }
        } else {
            match key {
                "timebase" => {
                    let v = value.as_f64()
                        .ok_or_else(|| invalid("timebase must be a number"))?;
                    c(dev, &format!("TDIV {}", fmt_scpi(v))).map_err(dev_err)?;
                    return Ok(json!(query_float(dev, "TDIV?")?));
                }
                "memory_depth" => {
                    let depth = value.as_str().unwrap_or("");
                    if !MEMORY_DEPTHS.contains(&depth) {
                        return Err(invalid(format!(
                            "memory_depth must be one of {MEMORY_DEPTHS:?}")));
                    }
                    c(dev, &format!("MSIZ {depth}")).map_err(dev_err)?;
                    return Ok(json!(q(dev, "MSIZ?").map_err(dev_err)?.trim()));
                }
                "trigger" => return trigger_set(dev, value),
                _ => {}
            }
        }
        Err(invalid(format!("unknown key {key:?} in scope")))
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
        if mode != "single" && mode != "snapshot" {
            return Err(RpcError::new(UNSUPPORTED, format!("mode {mode:?} not supported")));
        }
        let timeout = params.get("timeout").and_then(Value::as_f64).unwrap_or(30.0);
        self.capture_id += 1;
        let cid = self.capture_id;
        self.stop.store(false, Ordering::SeqCst);
        let (dev, stop, ctx, mode) =
            (self.dev.clone(), self.stop.clone(), ctx.clone(), mode.to_string());
        self.acq = Some(thread::spawn(move || {
            if let Err(e) = acquire_inner(&dev, &stop, &ctx, cid, &mode, timeout) {
                ctx.notify("capture.end",
                           json!({"capture_id": cid, "ok": false, "error": e}), None);
            }
        }));
        Ok(json!({"capture_id": cid}))
    }

    fn device_raw(&mut self, params: &Value) -> Result<Value, RpcError> {
        self.require_open()?;
        let cmd = params.get("command").and_then(Value::as_str).unwrap_or("");
        if params.get("binary").and_then(Value::as_bool).unwrap_or(false) {
            let data = with_dev(&self.dev, |d| d.query_block(cmd, None))
                .map_err(dev_err)?;
            return Ok(json!({"length": data.len()}));
        }
        let is_query = params.get("query").and_then(Value::as_bool)
            .unwrap_or_else(|| cmd.trim_end().ends_with('?'));
        if is_query {
            return Ok(json!({"response": q(&self.dev, cmd).map_err(dev_err)?}));
        }
        c(&self.dev, cmd).map_err(dev_err)?;
        Ok(json!({}))
    }
}

fn usbtmc_paths() -> Vec<String> {
    let mut paths: Vec<String> = std::fs::read_dir("/dev").ok()
        .map(|entries| {
            entries.flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|name| name.starts_with("usbtmc"))
                .map(|name| format!("/dev/{name}"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths
}

fn query_float(dev: &Dev, cmd: &str) -> Result<f64, RpcError> {
    let reply = q(dev, cmd).map_err(dev_err)?;
    scpi_float(&reply).map_err(|e| dev_err(e.to_string()))
}

// ------------------------------------------------------------------
// trigger
// ------------------------------------------------------------------
fn trigger_get(dev: &Dev) -> Result<Value, String> {
    let trse = q(dev, "TRSE?")?; // e.g. "EDGE,SR,C1,HT,OFF"
    let parts: Vec<String> = trse.split(',').map(|p| p.trim().to_string()).collect();
    let source = parts.iter().position(|p| p == "SR")
        .and_then(|i| parts.get(i + 1).cloned())
        .unwrap_or_else(|| "C1".to_string());
    let mut trig = Map::new();
    trig.insert("type".into(),
                json!(parts.first().map(|p| p.to_lowercase()).unwrap_or("edge".into())));
    trig.insert("source".into(), json!(source));
    if ANALOG_CHANNELS.contains(&source.as_str()) || source == "EX" || source == "EX5" {
        let slope = q(dev, &format!("{source}:TRSL?"))?.trim().to_uppercase();
        trig.insert("slope".into(), json!(match slope.as_str() {
            "POS" => "rising".to_string(),
            "NEG" => "falling".to_string(),
            other => other.to_lowercase(),
        }));
        if let Ok(reply) = q(dev, &format!("{source}:TRLV?")) {
            if let Ok(level) = scpi_float(&reply) {
                trig.insert("level".into(), json!(level));
            }
        }
    }
    let mode = q(dev, "TRMD?")?.trim().to_uppercase();
    trig.insert("mode".into(), json!(mode.to_lowercase()));
    Ok(Value::Object(trig))
}

fn trigger_set(dev: &Dev, value: &Value) -> Result<Value, RpcError> {
    let obj = value.as_object()
        .ok_or_else(|| invalid("trigger must be an object"))?;
    if obj.get("type").and_then(Value::as_str).unwrap_or("edge") != "edge" {
        return Err(RpcError::new(UNSUPPORTED, "only edge trigger supported for now"));
    }
    let source = obj.get("source").and_then(Value::as_str).unwrap_or("C1");
    if !ANALOG_CHANNELS.contains(&source)
        && !["EX", "EX5", "LINE"].contains(&source) {
        return Err(invalid(format!("trigger source {source:?}")));
    }
    c(dev, &format!("TRSE EDGE,SR,{source},HT,OFF")).map_err(dev_err)?;
    if let Some(slope) = obj.get("slope") {
        let scpi_slope = match slope.as_str() {
            Some("rising") => "POS",
            Some("falling") => "NEG",
            _ => return Err(invalid(format!("slope {slope}"))),
        };
        c(dev, &format!("{source}:TRSL {scpi_slope}")).map_err(dev_err)?;
    }
    if let Some(level) = obj.get("level").and_then(Value::as_f64) {
        if source != "LINE" {
            c(dev, &format!("{source}:TRLV {}", fmt_volts(level))).map_err(dev_err)?;
        }
    }
    if let Some(mode) = obj.get("mode").and_then(Value::as_str) {
        let mode = mode.to_uppercase();
        if !["AUTO", "NORM", "SINGLE"].contains(&mode.as_str()) {
            return Err(invalid(format!("mode {mode:?}")));
        }
        c(dev, &format!("TRMD {mode}")).map_err(dev_err)?;
    }
    trigger_get(dev).map_err(dev_err)
}

// ------------------------------------------------------------------
// acquisition worker
// ------------------------------------------------------------------

/// Poll SAST? until the scope reports Stop (single-shot complete).
fn wait_stopped(dev: &Dev, stop: &AtomicBool, timeout: f64) -> Result<bool, String> {
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    while Instant::now() < deadline {
        if stop.load(Ordering::SeqCst) {
            c(dev, "STOP")?;
            return Ok(true);
        }
        if q(dev, "SAST?")?.trim().eq_ignore_ascii_case("stop") {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(50));
    }
    Ok(false)
}

fn acquire_inner(dev: &Dev, stop: &AtomicBool, ctx: &Ctx, cid: u64, mode: &str,
                 timeout: f64) -> Result<(), String> {
    let prev_trmd = q(dev, "TRMD?")?.trim().to_uppercase();
    if mode == "single" {
        ctx.notify("event.status", json!({"state": "armed"}), None);
        c(dev, "TRMD SINGLE")?;
        if !wait_stopped(dev, stop, timeout)? {
            c(dev, "STOP")?;
            return Err(format!("no trigger within {timeout:.0}s"));
        }
        ctx.notify("event.status", json!({"state": "triggered"}), None);
    } else {
        // snapshot: freeze whatever is on screen
        c(dev, "STOP")?;
    }

    let mut enabled = Vec::new();
    for ch in ANALOG_CHANNELS {
        if q(dev, &format!("{ch}:TRA?"))?.to_uppercase().ends_with("ON") {
            enabled.push(ch);
        }
    }
    if enabled.is_empty() {
        return Err("no channels enabled".into());
    }
    let float = |cmd: &str| -> Result<f64, String> {
        scpi_float(&q(dev, cmd)?).map_err(|e| e.to_string())
    };
    let samplerate = float("SARA?")?;
    let tdiv = float("TDIV?")?;
    let sample_count = float(&format!("SANU? {}", enabled[0]))? as usize;

    let mut streams = Vec::new();
    for (si, ch) in enabled.iter().enumerate() {
        let vdiv = float(&format!("{ch}:VDIV?"))?;
        let ofst = float(&format!("{ch}:OFST?"))?;
        streams.push(json!({
            "stream": si, "kind": "analog", "channels": [ch],
            "sample_count": sample_count,
            "encoding": {"dtype": "int8",
                         "scale": vdiv / CODES_PER_DIV,
                         "offset": -ofst, "unit": "V",
                         "quantity": "voltage", "digits": 3}}));
    }
    ctx.notify("capture.begin", json!({
        "capture_id": cid, "samplerate": samplerate,
        "t0": -(tdiv * HORIZ_DIVS / 2.0), "timebase": tdiv,
        "streams": streams}), None);

    ctx.notify("event.status", json!({"state": "transferring"}), None);
    for (si, ch) in enabled.iter().enumerate() {
        read_channel(dev, stop, ctx, cid, si, ch, sample_count)?;
        if stop.load(Ordering::SeqCst) {
            break;
        }
    }

    // Restore free-running state and full-transfer setup.
    with_dev(dev, |d| {
        d.command("WFSU SP,0,NP,0,FP,0")?;
        if mode == "snapshot" && (prev_trmd == "AUTO" || prev_trmd == "NORM") {
            d.command(&format!("TRMD {prev_trmd}"))?;
        }
        Ok(())
    })?;
    let aborted = stop.load(Ordering::SeqCst);
    let mut end = json!({"capture_id": cid, "ok": !aborted});
    if aborted {
        end["error"] = json!("aborted");
    }
    ctx.notify("capture.end", end, None);
    ctx.notify("event.status", json!({"state": "idle"}), None);
    Ok(())
}

fn read_channel(dev: &Dev, stop: &AtomicBool, ctx: &Ctx, cid: u64, stream: usize,
                ch: &str, sample_count: usize) -> Result<(), String> {
    let block_timeout = Some(Duration::from_secs(30));
    let mut seq = 0u64;
    if sample_count <= PAGE_SAMPLES {
        let mut data = with_dev(dev, |d| {
            d.command("WFSU SP,0,NP,0,FP,0")?;
            d.query_block(&format!("{ch}:WF? DAT2"), block_timeout)
        })?;
        data.truncate(sample_count);
        let mut off = 0;
        while off < data.len() {
            let part = &data[off..data.len().min(off + DATA_FRAME_BYTES)];
            ctx.notify("capture.data",
                       json!({"capture_id": cid, "stream": stream, "seq": seq,
                              "first_sample": off, "nsamples": part.len()}),
                       Some(part));
            seq += 1;
            off += part.len();
        }
        return Ok(());
    }
    let mut first = 0;
    while first < sample_count && !stop.load(Ordering::SeqCst) {
        let npoints = PAGE_SAMPLES.min(sample_count - first);
        let mut data = with_dev(dev, |d| {
            d.command(&format!("WFSU SP,0,NP,{npoints},FP,{first}"))?;
            d.query_block(&format!("{ch}:WF? DAT2"), block_timeout)
        })?;
        if data.is_empty() {
            return Err(format!("empty page at FP={first} for {ch}"));
        }
        data.truncate(npoints);
        ctx.notify("capture.data",
                   json!({"capture_id": cid, "stream": stream, "seq": seq,
                          "first_sample": first, "nsamples": data.len()}),
                   Some(&data));
        seq += 1;
        first += data.len();
    }
    Ok(())
}

impl CaptureServer for SdsPlugin {
    fn info(&self) -> Value {
        json!({"name": "siglent-sds1000xe", "version": "0.2.0", "vendor": "OpenMSO",
               "description": "Siglent SDS1000X-E series oscilloscopes"})
    }

    fn capabilities(&self) -> Value {
        json!({"scan": true, "modes": ["single", "snapshot"],
               "raw": true, "trigger_forms": ["edge"]})
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
            "device.raw" => self.device_raw(params),
            _ => Err(RpcError::method_not_found(method)),
        }
    }

    fn on_disconnect(&mut self) {
        self.release();
    }
}

fn main() {
    let mut plugin = SdsPlugin::new();
    server::run_from_argv(&mut plugin);
}
