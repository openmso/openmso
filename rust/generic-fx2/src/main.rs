// SPDX-License-Identifier: Apache-2.0
//! OpenMSO capture plugin for Cypress FX2 (fx2lafw) logic analyzers.
//!
//! Written from scratch against the fx2lafw wire-protocol description and
//! live observation of a Saleae Logic clone (0925:3881). libsigrok's
//! `fx2lafw.c` was consulted as a behavioral reference only; no GPL code is
//! included. See `docs/fx2-plan/README.md` §3 for the clean-room discipline.
//!
//! The fx2lafw firmware blob (GPL-2.0+) is NOT vendored: the user's
//! system-installed blob is read at runtime and uploaded via the Cypress 0xA0
//! bootloader while answering `Hello`.

mod firmware;
mod fx2;

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use openmso::encoding::accepts;
use openmso::proto::{
    event, stream, trigger, AcquireMode, AcquireStart, AcquireStop, AcquisitionBegin,
    AcquisitionEnd, Capabilities, CaptureBegin, CaptureEnd, CaptureTrigger, Channel, ChannelConfig,
    ChannelKind, Config, Description, DeviceConfig, DeviceInfo, DeviceLimits, DoubleSet, Hello,
    HelloResult, LogicFormat, LogicLimits, PluginInfo, SampleEncoding, State, Stream, TriggerKind,
    UintRange, UintSet,
};
use openmso::server::{self, Args, CaptureServer, Events, StreamSender};
use openmso::{proto, Reply};

use fx2::{Fx2, ReadResult, Target, DEFAULT_LIMIT_SAMPLES, DEFAULT_SAMPLERATE, SAMPLE_RATES};

const LOGIC_CHANNELS: usize = 8;
/// One byte per sample, bit *i* = D*i* — byte-identical to OCP logic encoding.
const UNITSIZE: usize = 1;
const BULK_BUF_SIZE: usize = 4096;
const BULK_TIMEOUT: Duration = Duration::from_millis(1000);
/// The device streams without bound, so a single capture needs a ceiling
/// somewhere; 24 MSa/s fills this in about 20 s.
const MAX_SAMPLE_DEPTH: u64 = 500_000_000;

#[derive(Clone, Copy)]
struct Cfg {
    samplerate: u32,
    sample_depth: u64,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg { samplerate: DEFAULT_SAMPLERATE, sample_depth: DEFAULT_LIMIT_SAMPLES }
    }
}

impl Cfg {
    fn to_config(self) -> Config {
        Config {
            device: Some(DeviceConfig {
                samplerate: Some(self.samplerate as f64),
                sample_depth: Some(self.sample_depth),
                trigger: Some(proto::Trigger {
                    trigger: Some(trigger::Trigger::None(proto::Empty {})),
                    position: 0.0,
                }),
                averaging: Some(1),
                capture_span: Some(self.sample_depth as f64 / self.samplerate as f64),
            }),
            // The device always samples all eight bits into one packed stream,
            // so a channel cannot be switched off without renumbering the bits
            // underneath a decoder.
            channels: (0..LOGIC_CHANNELS)
                .map(|i| ChannelConfig {
                    id: format!("D{i}"),
                    enabled: Some(true),
                    ..Default::default()
                })
                .collect(),
            vendor: Default::default(),
        }
    }

    fn apply(&mut self, config: &Config) -> Reply<()> {
        if let Some(device) = &config.device {
            self.apply_device(device)?;
        }
        for channel in &config.channels {
            let index = channel
                .id
                .strip_prefix('D')
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|i| *i < LOGIC_CHANNELS);
            if index.is_none() {
                return Err(proto::Error::invalid(format!("no channel {:?}", channel.id)));
            }
        }
        if let Some((key, _)) = config.vendor.iter().next() {
            return Err(proto::Error::invalid(format!("no vendor option {key:?}")));
        }
        Ok(())
    }

    fn apply_device(&mut self, device: &DeviceConfig) -> Reply<()> {
        if let Some(rate) = device.samplerate {
            if !(rate.is_finite() && rate > 0.0) {
                return Err(proto::Error::invalid("samplerate must be positive"));
            }
            // Snapped rather than refused: the ladder is what the device's
            // IFCLK divider can actually produce, and set_config reports back
            // what it settled on.
            self.samplerate = snap_samplerate(rate);
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
                return Err(proto::Error::unsupported("fx2lafw has no hardware trigger"));
            }
        }
        match device.averaging {
            None | Some(1) => Ok(()),
            Some(_) => Err(proto::Error::unsupported("averaging")),
        }
    }
}

fn snap_samplerate(rate: f64) -> u32 {
    SAMPLE_RATES
        .iter()
        .copied()
        .min_by(|a, b| {
            let (da, db) = ((*a as f64 - rate).abs(), (*b as f64 - rate).abs());
            da.partial_cmp(&db).expect("rate ladder is finite")
        })
        .unwrap_or(DEFAULT_SAMPLERATE)
}

type Dev = Arc<Mutex<Option<Fx2>>>;

struct Fx2Plugin {
    device_url: String,
    dev: Dev,
    cfg: Cfg,
    encodings: Vec<i32>,
    stop: Arc<AtomicBool>,
    acquisition: Option<JoinHandle<()>>,
}

impl Fx2Plugin {
    fn new(device_url: String) -> Self {
        Fx2Plugin {
            device_url,
            dev: Arc::new(Mutex::new(None)),
            cfg: Cfg::default(),
            encodings: Vec::new(),
            stop: Arc::new(AtomicBool::new(false)),
            acquisition: None,
        }
    }

    fn join_previous(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.acquisition.take() {
            handle.join().ok();
        }
    }

    /// The work `Hello` does, minus the socket a live plugin has by then.
    fn connect(&mut self, req: &Hello) -> Reply<HelloResult> {
        let target = Target::parse(&self.device_url).map_err(proto::Error::device)?;
        // Where the firmware upload happens, so a device that is missing or
        // has no blob installed comes back as an Error on a live connection.
        let device = Fx2::open(&target).map_err(proto::Error::device)?;

        let (major, minor) = device.fw_version;
        let info = DeviceInfo {
            vendor: format!("{:04x}", target.vid),
            model: "fx2lafw".to_string(),
            serial: device.serial.clone(),
            firmware_version: format!("{major}.{minor}"),
        };
        *self.dev.lock().unwrap() = Some(device);

        let result = server::hello_result(
            req,
            PluginInfo {
                name: "generic-fx2".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                vendor: "OpenMSO".to_string(),
                description: "Cypress FX2 (fx2lafw) logic analyzers".to_string(),
            },
            Capabilities {
                // No snapshot: fx2lafw streams and has no on-device memory to
                // hand over.
                modes: vec![
                    AcquireMode::AcquireSingle as i32,
                    AcquireMode::AcquireContinuous as i32,
                ],
                trigger_kinds: vec![TriggerKind::TriggerNone as i32],
            },
            info,
        );
        self.encodings = result.encodings.clone();
        Ok(result)
    }
}

impl CaptureServer for Fx2Plugin {
    fn hello(&mut self, req: &Hello, _events: &Arc<Events>) -> Reply<HelloResult> {
        self.connect(req)
    }

    fn describe(&mut self) -> Reply<Description> {
        Ok(Description {
            channels: (0..LOGIC_CHANNELS)
                .map(|i| Channel {
                    id: format!("D{i}"),
                    name: format!("D{i}"),
                    kind: ChannelKind::ChannelLogic as i32,
                    index: i as u32,
                    ..Default::default()
                })
                .collect(),
            limits: Some(DeviceLimits {
                // Discrete steps, not a range: these are the only rates the
                // IFCLK divider produces.
                samplerate: Some(DoubleSet {
                    values: SAMPLE_RATES.iter().map(|r| *r as f64).collect(),
                    ..Default::default()
                }),
                sample_depth: Some(UintSet {
                    range: Some(UintRange { min: 1, max: MAX_SAMPLE_DEPTH, step: 1 }),
                    ..Default::default()
                }),
                samplerate_settable: true,
                sample_depth_settable: true,
                max_enabled_channels: LOGIC_CHANNELS as u32,
                ..Default::default()
            }),
            analog: None,
            // Fixed threshold, which an empty set is how to say.
            logic: Some(LogicLimits::default()),
            vendor_options: vec![],
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
        match mode {
            AcquireMode::AcquireSingle | AcquireMode::AcquireContinuous => {}
            AcquireMode::AcquireSnapshot => {
                return Err(proto::Error::unsupported(
                    "snapshot: fx2lafw has no on-device memory",
                ))
            }
            AcquireMode::Unspecified => return Err(proto::Error::invalid("no acquire mode")),
        }

        // An idle bus costs almost nothing run-length encoded, and a logic
        // analyzer's bus is mostly idle.
        let encoding = if accepts(&self.encodings, SampleEncoding::Transition) {
            SampleEncoding::Transition
        } else {
            SampleEncoding::Packed
        };

        // Configure and start the device on the control loop, so a device
        // that refuses fails the request rather than a CaptureEnd.
        {
            let mut guard = self.dev.lock().unwrap();
            let device = guard
                .as_mut()
                .ok_or_else(|| proto::Error::device("no device open"))?;
            device.start(self.cfg.samplerate).map_err(proto::Error::device)?;
        }

        let stop = Arc::new(AtomicBool::new(false));
        self.stop = stop.clone();
        let capture =
            Capture { capture_id: req.capture_id, mode, cfg: self.cfg, encoding };
        let (dev, events) = (self.dev.clone(), events.clone());
        self.acquisition = Some(thread::spawn(move || {
            let error = capture
                .run(&dev, &events, &stop)
                .err()
                .map(|e| proto::Error::device(e.to_string()));
            // Whatever happened, the device stops streaming and the frontend
            // gets its CaptureEnd.
            if let Some(device) = dev.lock().unwrap().as_mut() {
                device.stop().ok();
            }
            events
                .send(event::Event::CaptureEnd(CaptureEnd {
                    capture_id: capture.capture_id,
                    error,
                }))
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
        if let Some(device) = self.dev.lock().unwrap().as_mut() {
            device.stop().ok();
        }
    }
}

struct Capture {
    capture_id: u64,
    mode: AcquireMode,
    cfg: Cfg,
    encoding: SampleEncoding,
}

impl Capture {
    fn run(&self, dev: &Dev, events: &Arc<Events>, stop: &AtomicBool) -> openmso::Result<()> {
        events.send(event::Event::CaptureBegin(CaptureBegin {
            capture_id: self.capture_id,
            samplerate: self.cfg.samplerate as f64,
            streams: vec![Stream {
                id: 0,
                channels: (0..LOGIC_CHANNELS).map(|i| format!("D{i}")).collect(),
                format: Some(stream::Format::Logic(LogicFormat { unitsize: UNITSIZE as u32 })),
            }],
        }))?;
        events.status(State::Armed, "")?;

        // One acquisition either way: continuous is a streaming device's single
        // unbounded run, not a series of frames, because there is nothing to
        // re-arm.
        let sample_count = match self.mode {
            AcquireMode::AcquireContinuous => 0,
            _ => self.cfg.sample_depth,
        };
        events.send(event::Event::AcquisitionBegin(AcquisitionBegin {
            capture_id: self.capture_id,
            acquisition: 0,
            t0: 0.0,
            sample_count,
        }))?;
        // A free-running device triggers on sample 0 the moment it is armed.
        events.status(State::Triggered, "")?;
        events.send(event::Event::Trigger(CaptureTrigger {
            capture_id: self.capture_id,
            acquisition: 0,
            sample: 0,
        }))?;
        events.status(State::Transferring, "")?;

        let collected = self.transfer(dev, events, stop)?;

        events.send(event::Event::AcquisitionEnd(AcquisitionEnd {
            capture_id: self.capture_id,
            acquisition: 0,
            // fx2lafw gives the host no overrun signal, so a gap would be a
            // guess. Reporting none is the honest answer.
            dropped_samples: 0,
        }))?;
        let _ = collected;
        events.status(State::Idle, "")
    }

    fn transfer(&self, dev: &Dev, events: &Arc<Events>, stop: &AtomicBool) -> openmso::Result<u64> {
        let mut sender = StreamSender::new(self.capture_id, 0, 0, UNITSIZE);
        if self.encoding == SampleEncoding::Transition {
            sender = sender.transition();
        }

        let mps = dev
            .lock()
            .unwrap()
            .as_ref()
            .map(|d| d.max_packet_size())
            .unwrap_or(512);
        // nusb requires IN buffers to be a multiple of the max packet size.
        let buf_size = BULK_BUF_SIZE.max(mps).div_ceil(mps) * mps;

        let mut collected: u64 = 0;
        let single = self.mode != AcquireMode::AcquireContinuous;
        while !stop.load(Ordering::SeqCst) {
            let read = {
                let mut guard = dev.lock().unwrap();
                let device = guard.as_mut().ok_or_else(|| {
                    openmso::Error::Protocol("device closed during acquisition".into())
                })?;
                device
                    .read_blocking(buf_size, BULK_TIMEOUT)
                    .map_err(openmso::Error::Protocol)?
            };

            let mut data = match read {
                ReadResult::Data(data) => data,
                ReadResult::Timeout | ReadResult::Stall => continue,
            };
            if data.is_empty() {
                continue;
            }

            if single && collected + data.len() as u64 >= self.cfg.sample_depth {
                data.truncate((self.cfg.sample_depth - collected) as usize);
            }
            collected += data.len() as u64;
            sender.send(events, &data)?;

            if single && collected >= self.cfg.sample_depth {
                break;
            }
        }
        Ok(collected)
    }
}

fn main() -> ExitCode {
    let args = match Args::from_env() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("generic-fx2: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Ties this process's life to the frontend's: its death closes our stdin.
    server::exit_on_stdin_eof();
    let mut plugin = Fx2Plugin::new(args.device.clone());
    match server::serve(&args, &mut plugin) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("generic-fx2: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_snap_to_the_ladder_the_divider_can_produce() {
        assert_eq!(snap_samplerate(1e6), 1_000_000);
        assert_eq!(snap_samplerate(1.1e6), 1_000_000);
        assert_eq!(snap_samplerate(1e9), 24_000_000, "clamps to the top step");
        assert_eq!(snap_samplerate(1.0), 20_000, "clamps to the bottom step");
    }

    #[test]
    fn config_round_trips_and_derives_capture_span() {
        let cfg = Cfg { samplerate: 1_000_000, sample_depth: 500_000 };
        let device = cfg.to_config().device.unwrap();
        assert_eq!(device.samplerate, Some(1e6));
        assert_eq!(device.sample_depth, Some(500_000));
        assert_eq!(device.capture_span, Some(0.5));
    }

    #[test]
    fn a_sparse_set_config_leaves_everything_else_alone() {
        let mut cfg = Cfg::default();
        cfg.apply(&Config {
            device: Some(DeviceConfig { sample_depth: Some(4096), ..Default::default() }),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(cfg.sample_depth, 4096);
        assert_eq!(cfg.samplerate, Cfg::default().samplerate);
    }

    #[test]
    fn an_unachievable_rate_is_snapped_rather_than_refused() {
        let mut cfg = Cfg::default();
        cfg.apply(&Config {
            device: Some(DeviceConfig { samplerate: Some(7.0), ..Default::default() }),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(cfg.samplerate, 20_000);
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
            refuse(device(DeviceConfig {
                sample_depth: Some(MAX_SAMPLE_DEPTH + 1),
                ..Default::default()
            })),
            proto::ErrorCode::ErrorInvalidRequest
        );
        assert_eq!(
            refuse(device(DeviceConfig { averaging: Some(8), ..Default::default() })),
            proto::ErrorCode::ErrorUnsupported
        );
        // No hardware trigger, so asking for one is refused rather than
        // silently free-running.
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
                channels: vec![ChannelConfig { id: "D9".into(), ..Default::default() }],
                ..Default::default()
            }),
            proto::ErrorCode::ErrorInvalidRequest
        );
    }

    #[test]
    fn a_device_url_for_another_plugin_fails_the_handshake() {
        for url in ["demo://0", "tcp://192.168.1.155:5025"] {
            let code = Fx2Plugin::new(url.to_string())
                .connect(&Hello::default())
                .unwrap_err()
                .code;
            assert_eq!(
                proto::ErrorCode::try_from(code).unwrap(),
                proto::ErrorCode::ErrorDevice,
                "{url}"
            );
        }
    }

    #[test]
    fn a_usb_url_naming_absent_hardware_fails_on_a_live_connection() {
        // Not a dead process and a line on stderr: a normal Error, in reply.
        let code = Fx2Plugin::new("usb://dead:beef".to_string())
            .connect(&Hello::default())
            .unwrap_err()
            .code;
        assert_eq!(proto::ErrorCode::try_from(code).unwrap(), proto::ErrorCode::ErrorDevice);
    }
}
