// SPDX-License-Identifier: Apache-2.0
//! OpenMSO demo/simulation plugin, and the reference OCP v1 plugin.
//!
//! Synthesizes a mixed-signal capture with no hardware: two analog channels
//! (sine + square with noise) and eight logic channels (a binary counter on
//! D0-D6, plus a 10-bit-frame 115200-baud UART pattern on D7 sending
//! "OpenMSO! "). Useful for exercising frontends, the protocol, decoders and
//! file writers.

use std::collections::HashMap;
use std::f64::consts::TAU;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use openmso::encoding::accepts;
use openmso::proto::{
    event, stream, trigger, AcquireMode, AcquireStart, AcquireStop,
    AcquisitionBegin, AcquisitionEnd, AnalogFormat, AnalogLimits, Capabilities, CaptureBegin,
    CaptureEnd, CaptureTrigger, Channel, ChannelConfig, ChannelKind, Config, Coupling,
    DeviceConfig, DeviceInfo, DeviceLimits, Description, DoubleSet, Empty, Hello, HelloResult,
    LogicFormat, LogicLimits, PluginInfo, SampleEncoding, SampleType, State, Stream, TriggerKind,
    UintRange, UintSet, VendorOption,
};
use openmso::server::{self, Args, CaptureServer, Events, StreamSender};
use openmso::{proto, Reply};

const UART_BAUD: f64 = 115200.0;
const UART_TEXT: &[u8] = b"OpenMSO! ";
/// Fixed, so a demo capture is byte-identical from run to run.
const NOISE_SEED: u64 = 0x0DE0_0DE0_0DE0_0DE0;
/// Guards against a frontend asking for a capture that would not fit in RAM.
const MAX_SAMPLE_DEPTH: u64 = 100_000_000;

const ANALOG: [(&str, &str); 2] = [("A0", "sine"), ("A1", "square")];
const LOGIC_CHANNELS: usize = 8;

// --- config ---------------------------------------------------------------

#[derive(Clone, Copy)]
struct Cfg {
    samplerate: f64,
    sample_depth: u64,
    frequency: f64,
    amplitude: f64,
    noise: f64,
    enabled: [bool; ANALOG.len()],
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg { samplerate: 1_000_000.0, sample_depth: 100_000, frequency: 1000.0,
              amplitude: 1.0, noise: 0.02, enabled: [true; ANALOG.len()] }
    }
}

impl Cfg {
    /// Settings with no field of their own in `Config`. A frontend may show
    /// these generically and must work correctly while ignoring them.
    const VENDOR_KEYS: [(&'static str, &'static str); 3] = [
        ("frequency", "signal frequency, Hz"),
        ("amplitude", "signal amplitude, volts peak"),
        ("noise", "gaussian noise added to each sample, volts RMS"),
    ];

    fn to_config(self) -> Config {
        let channels = ANALOG
            .iter()
            .zip(self.enabled)
            .map(|((id, _), enabled)| ChannelConfig {
                id: id.to_string(),
                enabled: Some(enabled),
                ..Default::default()
            })
            .chain((0..LOGIC_CHANNELS).map(|i| ChannelConfig {
                id: format!("D{i}"),
                enabled: Some(true),
                ..Default::default()
            }))
            .collect();
        let vendor = HashMap::from([
            ("frequency".to_string(), self.frequency.to_string()),
            ("amplitude".to_string(), self.amplitude.to_string()),
            ("noise".to_string(), self.noise.to_string()),
        ]);
        Config {
            device: Some(DeviceConfig {
                samplerate: Some(self.samplerate),
                sample_depth: Some(self.sample_depth),
                trigger: Some(proto::Trigger {
                    trigger: Some(trigger::Trigger::None(Empty {})),
                    position: 0.0,
                }),
                averaging: Some(1),
                capture_span: Some(self.sample_depth as f64 / self.samplerate),
            }),
            channels,
            vendor,
        }
    }

    /// Apply the fields present in `config`; the caller reports back what this
    /// left the device on, which is not always what was asked for.
    fn apply(&mut self, config: &Config) -> Reply<()> {
        if let Some(device) = &config.device {
            self.apply_device(device)?;
        }
        for channel in &config.channels {
            self.apply_channel(channel)?;
        }
        for (key, value) in &config.vendor {
            let slot = match key.as_str() {
                "frequency" => &mut self.frequency,
                "amplitude" => &mut self.amplitude,
                "noise" => &mut self.noise,
                _ => return Err(proto::Error::invalid(format!("no vendor option {key:?}"))),
            };
            *slot = value.parse().map_err(|_| {
                proto::Error::invalid(format!("vendor option {key:?} is not a number: {value:?}"))
            })?;
        }
        Ok(())
    }

    fn apply_device(&mut self, device: &DeviceConfig) -> Reply<()> {
        if let Some(rate) = device.samplerate {
            if !(rate.is_finite() && rate > 0.0) {
                return Err(proto::Error::invalid("samplerate must be positive"));
            }
            self.samplerate = rate;
        }
        if let Some(depth) = device.sample_depth {
            if depth == 0 || depth > MAX_SAMPLE_DEPTH {
                return Err(proto::Error::invalid(format!(
                    "sample_depth must be 1..{MAX_SAMPLE_DEPTH}"
                )));
            }
            self.sample_depth = depth;
        }
        if let Some(trigger) = &device.trigger {
            if !matches!(trigger.trigger, None | Some(trigger::Trigger::None(_))) {
                return Err(proto::Error::unsupported("the demo device free-runs"));
            }
        }
        match device.averaging {
            None | Some(1) => Ok(()),
            Some(_) => Err(proto::Error::unsupported("averaging")),
        }
    }

    fn apply_channel(&mut self, channel: &ChannelConfig) -> Reply<()> {
        // Logic channels are always on: they share one packed stream, and
        // dropping one would renumber the bits underneath a decoder.
        if let Some(index) = logic_index(&channel.id) {
            if index < LOGIC_CHANNELS {
                return Ok(());
            }
        }
        let index = ANALOG
            .iter()
            .position(|(id, _)| *id == channel.id)
            .ok_or_else(|| proto::Error::invalid(format!("no channel {:?}", channel.id)))?;
        if let Some(enabled) = channel.enabled {
            self.enabled[index] = enabled;
        }
        if channel.coupling.is_some_and(|c| c != Coupling::Dc as i32) {
            return Err(proto::Error::unsupported("the demo device is DC coupled"));
        }
        Ok(())
    }
}

fn logic_index(id: &str) -> Option<usize> {
    id.strip_prefix('D').and_then(|n| n.parse().ok())
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
}

/// Volts per code: int8 codes at 25 per "division", like an 8-bit scope.
fn code_scale(cfg: &Cfg) -> f64 {
    cfg.amplitude / 100.0
}

fn synthesize(cfg: &Cfg) -> Synth {
    let n = cfg.sample_depth as usize;
    let (sr, f, a) = (cfg.samplerate, cfg.frequency, cfg.amplitude);
    let scale = code_scale(cfg);

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

    Synth { sine, square, logic }
}

// --- plugin ---------------------------------------------------------------

struct DemoPlugin {
    device: String,
    cfg: Cfg,
    /// What the frontend agreed to decode, from the Hello negotiation.
    encodings: Vec<i32>,
    stop: Arc<AtomicBool>,
    acquisition: Option<JoinHandle<()>>,
}

impl DemoPlugin {
    fn new(device: String) -> Self {
        DemoPlugin {
            device,
            cfg: Cfg::default(),
            encodings: vec![SampleEncoding::Packed as i32],
            stop: Arc::new(AtomicBool::new(false)),
            acquisition: None,
        }
    }

    /// The streams a capture will carry, in the order their ids are assigned.
    fn streams(&self, scale: f64) -> Vec<Stream> {
        let mut streams: Vec<Stream> = ANALOG
            .iter()
            .zip(self.cfg.enabled)
            .filter(|(_, enabled)| *enabled)
            .map(|((id, _), _)| Stream {
                channels: vec![id.to_string()],
                format: Some(stream::Format::Analog(AnalogFormat {
                    r#type: SampleType::SampleInt8 as i32,
                    scale,
                    offset: 0.0,
                    unit: "V".to_string(),
                    digits: 3,
                })),
                ..Default::default()
            })
            .collect();
        streams.push(Stream {
            channels: (0..LOGIC_CHANNELS).map(|i| format!("D{i}")).collect(),
            format: Some(stream::Format::Logic(LogicFormat { unitsize: 1 })),
            ..Default::default()
        });
        for (id, stream) in streams.iter_mut().enumerate() {
            stream.id = id as u32;
        }
        streams
    }

    fn join_previous(&mut self) {
        if let Some(handle) = self.acquisition.take() {
            self.stop.store(true, Ordering::SeqCst);
            handle.join().ok();
        }
    }
}

impl DemoPlugin {
    /// The work `Hello` does, minus the socket a live plugin has by then.
    fn connect(&mut self, req: &Hello) -> Reply<HelloResult> {
        // Nothing to open, but the URL still has to name something this plugin
        // drives — a real plugin connects to the device here, and reports the
        // failure as an Error on a live connection rather than dying.
        if !self.device.starts_with("demo://") {
            return Err(proto::Error::device(format!(
                "{:?} is not a demo:// device URL",
                self.device
            )));
        }
        let result = server::hello_result(
            req,
            PluginInfo {
                name: "demo".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                vendor: "OpenMSO".to_string(),
                description: "Simulated mixed-signal device".to_string(),
            },
            Capabilities {
                modes: vec![
                    AcquireMode::AcquireSingle as i32,
                    AcquireMode::AcquireContinuous as i32,
                    AcquireMode::AcquireSnapshot as i32,
                ],
                trigger_kinds: vec![TriggerKind::TriggerNone as i32],
            },
            DeviceInfo {
                vendor: "OpenMSO".to_string(),
                model: "Demo MSO".to_string(),
                serial: "DEMO0001".to_string(),
                firmware_version: env!("CARGO_PKG_VERSION").to_string(),
            },
        );
        self.encodings = result.encodings.clone();
        Ok(result)
    }
}

impl CaptureServer for DemoPlugin {
    fn hello(&mut self, req: &Hello, _events: &Arc<Events>) -> Reply<HelloResult> {
        self.connect(req)
    }

    fn describe(&mut self) -> Reply<Description> {
        let channels = ANALOG
            .iter()
            .enumerate()
            .map(|(i, (id, name))| Channel {
                id: id.to_string(),
                name: name.to_string(),
                kind: ChannelKind::ChannelAnalog as i32,
                index: i as u32,
                ..Default::default()
            })
            .chain((0..LOGIC_CHANNELS).map(|i| Channel {
                id: format!("D{i}"),
                name: format!("D{i}"),
                kind: ChannelKind::ChannelLogic as i32,
                index: i as u32,
                ..Default::default()
            }))
            .collect();
        Ok(Description {
            channels,
            limits: Some(DeviceLimits {
                samplerate: Some(DoubleSet {
                    range: Some(proto::DoubleRange { min: 1.0, max: 1e9, step: 0.0 }),
                    ..Default::default()
                }),
                sample_depth: Some(UintSet {
                    range: Some(UintRange { min: 1, max: MAX_SAMPLE_DEPTH, step: 1 }),
                    ..Default::default()
                }),
                samplerate_settable: true,
                sample_depth_settable: true,
                ..Default::default()
            }),
            // Everything a real scope adjusts is fixed here, which an empty
            // set is exactly how to say.
            analog: Some(AnalogLimits {
                couplings: vec![Coupling::Dc as i32],
                vertical_divisions: 8,
                ..Default::default()
            }),
            logic: Some(LogicLimits::default()),
            vendor_options: Cfg::VENDOR_KEYS
                .iter()
                .map(|(key, description)| VendorOption {
                    key: key.to_string(),
                    description: description.to_string(),
                    values: vec![],
                })
                .collect(),
        })
    }

    fn get_config(&mut self) -> Reply<Config> {
        Ok(self.cfg.to_config())
    }

    fn set_config(&mut self, config: &Config) -> Reply<Config> {
        // Applied to a copy, so a request that fails half way leaves the
        // device on the settings it had.
        let mut cfg = self.cfg;
        cfg.apply(config)?;
        self.cfg = cfg;
        Ok(cfg.to_config())
    }

    fn acquire_start(&mut self, req: &AcquireStart, events: &Arc<Events>) -> Reply<()> {
        self.join_previous();
        let mode = AcquireMode::try_from(req.mode).unwrap_or(AcquireMode::Unspecified);
        if mode == AcquireMode::Unspecified {
            return Err(proto::Error::invalid("no acquire mode"));
        }

        let streams = self.streams(code_scale(&self.cfg));
        let logic_encoding = if accepts(&self.encodings, SampleEncoding::Transition) {
            // An idle counter bit costs almost nothing run-length encoded, and
            // the frontend said it can expand it.
            SampleEncoding::Transition
        } else {
            SampleEncoding::Packed
        };

        let stop = Arc::new(AtomicBool::new(false));
        self.stop = stop.clone();
        let (capture_id, cfg) = (req.capture_id, self.cfg);
        let events = events.clone();
        // Everything from here on — synthesis included — happens off the
        // control loop, which must stay free to answer AcquireStop.
        self.acquisition = Some(thread::spawn(move || {
            let capture = Capture { capture_id, mode, cfg, streams, logic_encoding };
            let error = capture.run(&events, &stop).err().map(|e| proto::Error::device(e.to_string()));
            // The frontend is entitled to a CaptureEnd however this went; a
            // failure to send it means the socket is gone anyway.
            events
                .send(event::Event::CaptureEnd(CaptureEnd { capture_id, error }))
                .ok();
        }));
        Ok(())
    }

    fn acquire_stop(&mut self, _req: &AcquireStop) -> Reply<()> {
        self.stop.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn reset(&mut self) -> Reply<()> {
        self.join_previous();
        Ok(())
    }

    fn shutdown(&mut self) {
        self.join_previous();
    }
}

// --- acquisition ----------------------------------------------------------

struct Capture {
    capture_id: u64,
    mode: AcquireMode,
    cfg: Cfg,
    streams: Vec<Stream>,
    logic_encoding: SampleEncoding,
}

impl Capture {
    fn run(&self, events: &Events, stop: &AtomicBool) -> openmso::Result<()> {
        let synth = synthesize(&self.cfg);
        events.send(event::Event::CaptureBegin(CaptureBegin {
            capture_id: self.capture_id,
            samplerate: self.cfg.samplerate,
            streams: self.streams.clone(),
        }))?;

        let mut acquisition = 0;
        loop {
            self.acquire(events, &synth, acquisition)?;
            acquisition += 1;
            if self.mode != AcquireMode::AcquireContinuous || stop.load(Ordering::SeqCst) {
                break;
            }
        }
        events.status(State::Idle, "")
    }

    fn acquire(&self, events: &Events, synth: &Synth, acquisition: u64) -> openmso::Result<()> {
        let samples = synth.logic.len() as u64;
        events.status(State::Armed, "")?;
        events.send(event::Event::AcquisitionBegin(AcquisitionBegin {
            capture_id: self.capture_id,
            acquisition,
            t0: 0.0,
            sample_count: samples,
        }))?;
        // A free-running device triggers on sample 0 the moment it is armed.
        events.status(State::Triggered, "")?;
        events.send(event::Event::Trigger(CaptureTrigger {
            capture_id: self.capture_id,
            acquisition,
            sample: 0,
        }))?;
        events.status(State::Transferring, "")?;

        for stream in &self.streams {
            let (data, unitsize, encoding) = match &stream.format {
                Some(stream::Format::Analog(_)) => {
                    let data = match stream.channels.first().map(String::as_str) {
                        Some("A0") => &synth.sine,
                        _ => &synth.square,
                    };
                    (data, 1, SampleEncoding::Packed)
                }
                _ => (&synth.logic, 1, self.logic_encoding),
            };
            let mut sender = StreamSender::new(self.capture_id, acquisition, stream.id, unitsize);
            if encoding == SampleEncoding::Transition {
                sender = sender.transition();
            }
            sender.send(events, data)?;
        }

        events.send(event::Event::AcquisitionEnd(AcquisitionEnd {
            capture_id: self.capture_id,
            acquisition,
            dropped_samples: 0,
        }))
    }
}

fn main() -> ExitCode {
    let args = match Args::from_env() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("demo: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Ties this process's life to the frontend's: its death closes our stdin.
    server::exit_on_stdin_eof();
    let mut plugin = DemoPlugin::new(args.device.clone());
    match server::serve(&args, &mut plugin) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("demo: {e}");
            ExitCode::FAILURE
        }
    }
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
        let cfg = Cfg { sample_depth: 4096, ..Cfg::default() };
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
        let cfg = Cfg { sample_depth: 8192, ..Cfg::default() };
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
        let cfg = Cfg { sample_depth: 512, ..Cfg::default() };
        assert_eq!(synthesize(&cfg).sine, synthesize(&cfg).sine);
    }

    #[test]
    fn config_round_trips_through_the_wire_shape() {
        let mut cfg = Cfg::default();
        let wire = cfg.to_config();
        let device = wire.device.as_ref().unwrap();
        assert_eq!(device.samplerate, Some(1e6));
        assert_eq!(device.sample_depth, Some(100_000));
        // capture_span is derived, never set: depth over rate.
        assert_eq!(device.capture_span, Some(0.1));

        cfg.apply(&wire).unwrap();
        assert_eq!(cfg.samplerate, 1e6);
        assert_eq!(cfg.frequency, 1000.0);
    }

    #[test]
    fn a_sparse_set_config_leaves_everything_else_alone() {
        let mut cfg = Cfg::default();
        cfg.apply(&Config {
            device: Some(DeviceConfig { samplerate: Some(2e6), ..Default::default() }),
            vendor: HashMap::from([("noise".to_string(), "0.5".to_string())]),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(cfg.samplerate, 2e6);
        assert_eq!(cfg.noise, 0.5);
        assert_eq!(cfg.sample_depth, Cfg::default().sample_depth);
        assert_eq!(cfg.amplitude, Cfg::default().amplitude);
    }

    #[test]
    fn what_the_device_cannot_do_is_refused_by_code() {
        let refuse = |config: Config| {
            let code = Cfg::default().apply(&config).unwrap_err().code;
            proto::ErrorCode::try_from(code).unwrap()
        };
        let device = |device: DeviceConfig| Config { device: Some(device), ..Default::default() };

        assert_eq!(
            refuse(device(DeviceConfig { samplerate: Some(0.0), ..Default::default() })),
            proto::ErrorCode::ErrorInvalidRequest
        );
        assert_eq!(
            refuse(device(DeviceConfig { sample_depth: Some(0), ..Default::default() })),
            proto::ErrorCode::ErrorInvalidRequest
        );
        assert_eq!(
            refuse(device(DeviceConfig { averaging: Some(8), ..Default::default() })),
            proto::ErrorCode::ErrorUnsupported
        );
        assert_eq!(
            refuse(device(DeviceConfig {
                trigger: Some(proto::Trigger {
                    trigger: Some(trigger::Trigger::Edge(proto::EdgeTrigger::default())),
                    position: 0.5,
                }),
                ..Default::default()
            })),
            proto::ErrorCode::ErrorUnsupported
        );
        assert_eq!(
            refuse(Config {
                vendor: HashMap::from([("frequency".to_string(), "loud".to_string())]),
                ..Default::default()
            }),
            proto::ErrorCode::ErrorInvalidRequest
        );
        assert_eq!(
            refuse(Config {
                channels: vec![ChannelConfig { id: "C9".into(), ..Default::default() }],
                ..Default::default()
            }),
            proto::ErrorCode::ErrorInvalidRequest
        );
    }

    #[test]
    fn disabling_a_logic_channel_is_accepted_but_reported_as_still_on() {
        let mut cfg = Cfg::default();
        cfg.apply(&Config {
            channels: vec![
                ChannelConfig { id: "D3".into(), enabled: Some(false), ..Default::default() },
                ChannelConfig { id: "A1".into(), enabled: Some(false), ..Default::default() },
            ],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(cfg.enabled, [true, false], "analog channels do switch off");

        let reported = cfg.to_config();
        let d3 = reported.channels.iter().find(|c| c.id == "D3").unwrap();
        assert_eq!(d3.enabled, Some(true), "logic channels share a packed stream");
    }

    #[test]
    fn a_disabled_analog_channel_leaves_the_capture() {
        let mut plugin = DemoPlugin::new("demo://0".to_string());
        assert_eq!(plugin.streams(0.01).len(), 3, "two analog plus one logic");

        plugin.cfg.enabled = [false, true];
        let streams = plugin.streams(0.01);
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].channels, ["A1"]);
        // Stream ids stay dense, so a frontend can index by them.
        assert_eq!(streams.iter().map(|s| s.id).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn a_non_demo_device_url_fails_the_handshake() {
        let mut plugin = DemoPlugin::new("usb://04b4:8613".to_string());
        let code = plugin.connect(&Hello::default()).unwrap_err().code;
        assert_eq!(proto::ErrorCode::try_from(code).unwrap(), proto::ErrorCode::ErrorDevice);
    }

    #[test]
    fn negotiation_picks_run_length_only_when_offered() {
        let mut plugin = DemoPlugin::new("demo://0".to_string());

        let packed_only = Hello {
            accept_encodings: vec![SampleEncoding::Packed as i32],
            ..Default::default()
        };
        let result = plugin.connect(&packed_only).unwrap();
        assert_eq!(result.encodings, [SampleEncoding::Packed as i32]);
        assert!(!accepts(&plugin.encodings, SampleEncoding::Transition));

        let both = Hello {
            accept_encodings: openmso::encoding::ENCODINGS.iter().map(|e| *e as i32).collect(),
            ..Default::default()
        };
        plugin.connect(&both).unwrap();
        assert!(accepts(&plugin.encodings, SampleEncoding::Transition));
    }
}
