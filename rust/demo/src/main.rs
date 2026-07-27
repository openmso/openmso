// SPDX-License-Identifier: Apache-2.0
//! OpenMSO demo/simulation plugin.
//!
//! Synthesizes a mixed-signal capture with no hardware: two analog channels
//! (sine + square with noise) and eight logic channels (a binary counter on
//! D0-D6, plus a 10-bit-frame 115200-baud UART pattern on D7 sending
//! "OpenMSO! "). Useful for exercising frontends, the protocol, decoders and
//! file writers.

use std::f64::consts::TAU;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use openmso::server::{self, CaptureServer, Ctx, RpcError, BUSY, INVALID_PARAMS};
use serde_json::{json, Map, Value};

const UART_BAUD: f64 = 115200.0;
const UART_TEXT: &[u8] = b"OpenMSO! ";
/// Recommended max OCP payload is 4 MiB; 1 MiB keeps notifications small.
const CHUNK: usize = 1 << 20;
/// Fixed, so a demo capture is byte-identical from run to run.
const NOISE_SEED: u64 = 0x0DE0_0DE0_0DE0_0DE0;

// --- config ---------------------------------------------------------------

#[derive(Clone, Copy)]
struct Cfg {
    samplerate: f64,
    sample_count: i64,
    frequency: f64,
    amplitude: f64,
    noise: f64,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg { samplerate: 1_000_000.0, sample_count: 100_000, frequency: 1000.0,
              amplitude: 1.0, noise: 0.02 }
    }
}

impl Cfg {
    const KEYS: [&'static str; 5] =
        ["samplerate", "sample_count", "frequency", "amplitude", "noise"];

    fn get(&self, key: &str) -> Option<Value> {
        Some(match key {
            "samplerate" => json!(self.samplerate),
            "sample_count" => json!(self.sample_count),
            "frequency" => json!(self.frequency),
            "amplitude" => json!(self.amplitude),
            "noise" => json!(self.noise),
            _ => return None,
        })
    }

    fn set(&mut self, key: &str, v: &Value) -> Result<Value, RpcError> {
        let n = v.as_f64().ok_or_else(|| {
            RpcError::new(INVALID_PARAMS, format!("{key:?} must be a number"))
        })?;
        match key {
            "samplerate" => self.samplerate = n,
            "sample_count" => self.sample_count = n as i64,
            "frequency" => self.frequency = n,
            "amplitude" => self.amplitude = n,
            "noise" => self.noise = n,
            _ => return Err(RpcError::new(INVALID_PARAMS, format!("unknown key {key:?}"))),
        }
        Ok(self.get(key).expect("key matched above"))
    }
}

// --- deterministic noise --------------------------------------------------

/// xorshift64* plus Box-Muller. A dependency on `rand` would dwarf the ten
/// lines it saves, and a fixed seed keeps captures reproducible across runs.
struct Rng {
    state: u64,
    spare: Option<f64>,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Rng { state: seed | 1, spare: None }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in (0, 1) — never exactly 0, so `ln` below stays finite.
    fn next_f64(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) * (1.0 / (1u64 << 53) as f64)
    }

    /// Standard normal. Box-Muller yields two values per pass; keep the spare.
    fn normal(&mut self) -> f64 {
        if let Some(v) = self.spare.take() {
            return v;
        }
        let (u1, u2) = (self.next_f64(), self.next_f64());
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = TAU * u2;
        self.spare = Some(r * theta.sin());
        r * theta.cos()
    }
}

// --- signal synthesis -----------------------------------------------------

/// Clamp to the int8 code range and truncate toward zero, then reinterpret as
/// a byte. Storing codes as `u8` avoids an unsafe cast when we send the buffer.
fn clip_code(x: f64) -> u8 {
    (x.clamp(-127.0, 127.0) as i8) as u8
}

/// Idle-high async serial bit pattern: one 10-bit 8N1 frame per byte
/// (start, 8 data bits LSB first, stop), then an inter-message idle gap.
fn uart_bits(text: &[u8]) -> Vec<u8> {
    let mut bits = Vec::with_capacity(text.len() * 10 + 20);
    for &byte in text {
        bits.push(0);
        for i in 0..8 {
            bits.push((byte >> i) & 1);
        }
        bits.push(1);
    }
    bits.extend(std::iter::repeat(1).take(20));
    bits
}

struct Synth {
    sine: Vec<u8>,
    square: Vec<u8>,
    logic: Vec<u8>,
    scale: f64,
}

fn synthesize(cfg: &Cfg) -> Synth {
    let n = cfg.sample_count.max(0) as usize;
    let (sr, f, a) = (cfg.samplerate, cfg.frequency, cfg.amplitude);
    // int8 codes at 25 codes per "division", like a real 8-bit scope.
    let scale = a / 100.0;

    let mut rng = Rng::new(NOISE_SEED);
    let mut sine = Vec::with_capacity(n);
    let mut square = Vec::with_capacity(n);
    for i in 0..n {
        let phase = TAU * f * (i as f64 / sr);
        sine.push(clip_code((a * phase.sin() + rng.normal() * cfg.noise) / scale));
    }
    for i in 0..n {
        let phase = TAU * f * (i as f64 / sr);
        let s = phase.sin();
        let level = if s > 0.0 { 1.0 } else if s < 0.0 { -1.0 } else { 0.0 };
        square.push(clip_code((a * level + rng.normal() * cfg.noise) / scale));
    }

    // D0-D6: binary counter at f*32. D7: the UART pattern.
    let bits = uart_bits(UART_TEXT);
    let mut logic = Vec::with_capacity(n);
    for i in 0..n {
        let counter = ((i as f64) * (f * 32.0 / sr)) as i64 % 128;
        let idx = ((i as f64) * (UART_BAUD / sr)) as usize % bits.len();
        logic.push((counter as u8 & 0x7F) | (bits[idx] << 7));
    }

    Synth { sine, square, logic, scale }
}

// --- plugin ---------------------------------------------------------------

struct DemoPlugin {
    open: bool,
    cfg: Cfg,
    capture_id: i64,
    acq: Option<JoinHandle<()>>,
}

impl DemoPlugin {
    fn new() -> Self {
        DemoPlugin { open: false, cfg: Cfg::default(), capture_id: 0, acq: None }
    }

    fn describe(&self) -> Value {
        let mut channels = vec![
            json!({"id": "A0", "kind": "analog", "name": "sine", "index": 0}),
            json!({"id": "A1", "kind": "analog", "name": "square", "index": 1}),
        ];
        for i in 0..8 {
            channels.push(json!({"id": format!("D{i}"), "kind": "logic",
                                 "name": format!("D{i}"), "index": i}));
        }
        let mut config = Map::new();
        for key in Cfg::KEYS {
            config.insert(key.to_string(), json!({"scope": "device", "type": "number",
                                                  "get": true, "set": true}));
        }
        json!({"device": {"vendor": "OpenMSO", "model": "Demo MSO"},
               "channels": channels, "config": config})
    }

    fn config_get(&self, params: &Value) -> Result<Value, RpcError> {
        let mut values = Map::new();
        match params.get("keys").and_then(Value::as_array) {
            Some(keys) => {
                for k in keys.iter().filter_map(Value::as_str) {
                    if let Some(v) = self.cfg.get(k) {
                        values.insert(k.to_string(), v);
                    }
                }
            }
            None => {
                for k in Cfg::KEYS {
                    values.insert(k.to_string(), self.cfg.get(k).expect("known key"));
                }
            }
        }
        Ok(json!({"values": values}))
    }

    fn config_set(&mut self, params: &Value) -> Result<Value, RpcError> {
        let mut applied = Map::new();
        if let Some(values) = params.get("values").and_then(Value::as_object) {
            for (k, v) in values {
                applied.insert(k.clone(), self.cfg.set(k, v)?);
            }
        }
        Ok(json!({"applied": applied}))
    }

    fn acquire_start(&mut self, ctx: &Arc<Ctx>) -> Result<Value, RpcError> {
        if self.acq.as_ref().is_some_and(|h| !h.is_finished()) {
            return Err(RpcError::new(BUSY, "acquisition already running"));
        }
        if self.cfg.sample_count <= 0 {
            return Err(RpcError::new(INVALID_PARAMS, "sample_count must be positive"));
        }
        if self.cfg.samplerate <= 0.0 {
            return Err(RpcError::new(INVALID_PARAMS, "samplerate must be positive"));
        }
        self.capture_id += 1;
        let cid = self.capture_id;
        let (cfg, ctx) = (self.cfg, ctx.clone());
        self.acq = Some(thread::spawn(move || acquire(&ctx, cid, cfg)));
        Ok(json!({"capture_id": cid}))
    }
}

fn acquire(ctx: &Arc<Ctx>, cid: i64, cfg: Cfg) {
    let s = synthesize(&cfg);
    let n = s.sine.len();
    let encoding = json!({"dtype": "int8", "scale": s.scale, "offset": 0.0,
                          "unit": "V", "quantity": "voltage", "digits": 3});
    let logic_channels: Vec<String> = (0..8).map(|i| format!("D{i}")).collect();

    ctx.notify("capture.begin", json!({
        "capture_id": cid, "samplerate": cfg.samplerate, "t0": 0.0,
        "streams": [
            {"stream": 0, "kind": "analog", "channels": ["A0"],
             "sample_count": n, "encoding": encoding},
            {"stream": 1, "kind": "analog", "channels": ["A1"],
             "sample_count": n, "encoding": encoding},
            {"stream": 2, "kind": "logic", "channels": logic_channels,
             "sample_count": n, "encoding": {"unitsize": 1}},
        ]}), None);
    ctx.notify("capture.trigger", json!({"capture_id": cid, "sample": 0}), None);

    for (stream, data) in [(0, &s.sine), (1, &s.square), (2, &s.logic)] {
        // One byte per sample in every stream here, so a byte offset is also
        // the absolute sample index.
        for (seq, off) in (0..data.len()).step_by(CHUNK).enumerate() {
            let part = &data[off..(off + CHUNK).min(data.len())];
            ctx.notify("capture.data", json!({
                "capture_id": cid, "stream": stream, "seq": seq,
                "first_sample": off, "nsamples": part.len()}), Some(part));
        }
    }
    ctx.notify("capture.end", json!({"capture_id": cid, "ok": true}), None);
}

impl CaptureServer for DemoPlugin {
    fn info(&self) -> Value {
        json!({"name": "demo", "version": env!("CARGO_PKG_VERSION"),
               "vendor": "OpenMSO", "description": "Simulated mixed-signal device"})
    }

    fn capabilities(&self) -> Value {
        json!({"scan": true, "modes": ["single"], "raw": false,
               "trigger_forms": []})
    }

    fn handle(&mut self, method: &str, params: &Value, _payload: Option<Vec<u8>>,
              ctx: &Arc<Ctx>) -> Result<Value, RpcError> {
        match method {
            "scan" => Ok(json!({"devices": [
                {"device_id": "demo0", "vendor": "OpenMSO", "model": "Demo MSO",
                 "serial": "DEMO0001", "connection": "demo://0"}]})),
            "open" => {
                self.open = true;
                Ok(json!({}))
            }
            "close" => {
                self.open = false;
                Ok(json!({}))
            }
            "describe" => Ok(self.describe()),
            "config.get" => self.config_get(params),
            "config.set" => self.config_set(params),
            "acquire.start" => self.acquire_start(ctx),
            "acquire.stop" => Ok(json!({})),
            _ => Err(RpcError::method_not_found(method)),
        }
    }
}

fn main() {
    let mut plugin = DemoPlugin::new();
    server::run_from_argv(&mut plugin);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uart_frames_are_8n1_lsb_first() {
        let bits = uart_bits(b"O");
        assert_eq!(bits.len(), 10 + 20, "one frame plus the idle gap");
        assert_eq!(bits[0], 0, "start bit is low");
        assert_eq!(bits[9], 1, "stop bit is high");
        let byte: u8 = (0..8).map(|i| bits[1 + i] << i).sum();
        assert_eq!(byte, b'O');
        assert!(bits[10..].iter().all(|&b| b == 1), "line idles high");
    }

    #[test]
    fn codes_saturate_without_wrapping() {
        assert_eq!(clip_code(0.0) as i8, 0);
        assert_eq!(clip_code(1e9) as i8, 127);
        assert_eq!(clip_code(-1e9) as i8, -127);
        // Truncation toward zero, matching the reference implementation.
        assert_eq!(clip_code(-1.9) as i8, -1);
    }

    #[test]
    fn logic_carries_counter_below_uart() {
        let cfg = Cfg { sample_count: 4096, ..Cfg::default() };
        let s = synthesize(&cfg);
        assert_eq!(s.logic.len(), 4096);
        // D0-D6 is a counter, so the low 7 bits must cover the full range.
        let lo: std::collections::HashSet<u8> =
            s.logic.iter().map(|b| b & 0x7F).collect();
        assert_eq!(lo.len(), 128, "counter should wrap through all 128 values");
        // D7 is the UART line: both levels must appear.
        assert!(s.logic.iter().any(|b| b & 0x80 != 0));
        assert!(s.logic.iter().any(|b| b & 0x80 == 0));
    }

    #[test]
    fn analog_channels_span_the_code_range() {
        let cfg = Cfg { sample_count: 8192, ..Cfg::default() };
        let s = synthesize(&cfg);
        let peak = |v: &[u8]| v.iter().map(|&b| (b as i8) as i32).max().unwrap();
        let trough = |v: &[u8]| v.iter().map(|&b| (b as i8) as i32).min().unwrap();
        // Amplitude 1.0 at scale 0.01 puts the peaks near +/-100 codes.
        assert!((95..=110).contains(&peak(&s.sine)), "sine peak {}", peak(&s.sine));
        assert!((-110..=-95).contains(&trough(&s.sine)));
        // The square wave sits at its rails, so it has far fewer distinct codes.
        let distinct: std::collections::HashSet<u8> = s.square.iter().copied().collect();
        assert!(distinct.len() < 40, "square should be two-valued plus noise");
    }

    #[test]
    fn noise_is_deterministic_across_runs() {
        let cfg = Cfg { sample_count: 512, ..Cfg::default() };
        assert_eq!(synthesize(&cfg).sine, synthesize(&cfg).sine);
    }
}
