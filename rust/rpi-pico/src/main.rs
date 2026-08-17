// SPDX-License-Identifier: Apache-2.0
//! OpenMSO capture plugin for the Pico family.
//!
//! The openmso-pico firmware answers OCP itself, as JSON lines over USB-CDC,
//! so this is a relay: requests go out re-encoded, events come back re-stamped.
//! Sample payloads cross untouched, run-length encoding included, unless the
//! frontend cannot decode what the device produced.

mod link;

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use openmso::encoding::{accepts, decode_transition, negotiate_codecs, negotiate_encodings, CODECS};
use openmso::proto::{
    event, stream, trigger, AcquireStart, AcquireStop, AcquisitionBegin, AcquisitionEnd,
    AnalogFormat, AnalogLimits, Capabilities, CaptureBegin, CaptureData, CaptureEnd, CaptureTrigger,
    Channel, ChannelKind, Codec, Config, Coupling, Description, DeviceConfig, DeviceInfo,
    DeviceLimits, DeviceLost, DoubleRange, DoubleSet, EdgeTrigger, Empty, ErrorCode, Hello,
    HelloResult, Log, LogLevel, LogicFormat, LogicLimits, PluginInfo, SampleEncoding, SampleType,
    Slope, State, Status, Stream, TriggerKind, UintRange, UintSet, VendorOption,
};
use openmso::server::{Args, CaptureServer, Events};
use openmso::{proto, Reply};
use serde_json::{json, Value};

use link::{Frame, Link, LinkError};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const BYID_DIR: &str = "/dev/serial/by-id";
/// by-id names encode the flash unique ID:
/// `usb-OpenMSO_Pico_MSO_E6616407E39C7C2B-if00`.
const BYID_MARK: &str = "OpenMSO_Pico_MSO_";

// --- device discovery -----------------------------------------------------

struct Target {
    path: String,
    serial: String,
}

fn discover() -> Vec<Target> {
    let Ok(entries) = std::fs::read_dir(BYID_DIR) else { return Vec::new() };
    let mut found: Vec<Target> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let serial = name.split(BYID_MARK).nth(1)?.split("-if").next()?.to_string();
            let path = std::fs::canonicalize(entry.path()).ok()?;
            Some(Target { path: path.to_string_lossy().into_owned(), serial })
        })
        .collect();
    found.sort_by(|a, b| a.serial.cmp(&b.serial));
    found
}

/// `serial:///dev/ttyACM0` names a port; `serial://E661...` and `usb://VID:PID`
/// pick one out of what is attached.
fn resolve(url: &str) -> Result<Target, String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("{url:?} is not a device URL"))?;
    match scheme {
        "serial" if rest.starts_with('/') => {
            let serial = discover()
                .into_iter()
                .find(|t| t.path == rest)
                .map(|t| t.serial)
                .unwrap_or_default();
            Ok(Target { path: rest.to_string(), serial })
        }
        "serial" | "usb" => {
            // usb:// carries the VID:PID the frontend matched on, which says
            // nothing about which of several Picos is meant; a serial does.
            let wanted = match scheme {
                "serial" => Some(rest),
                _ => rest.split_once('/').map(|(_, serial)| serial),
            }
            .filter(|s| !s.is_empty());

            let mut devices = discover();
            if let Some(wanted) = wanted {
                devices.retain(|t| t.serial == wanted);
            }
            devices.into_iter().next().ok_or_else(|| match wanted {
                Some(serial) => format!("no OpenMSO Pico with serial {serial:?}"),
                None => format!("no OpenMSO Pico found under {BYID_DIR}"),
            })
        }
        other => Err(format!("{other:?} is not a scheme this plugin handles")),
    }
}

// --- vocabulary -----------------------------------------------------------

fn encoding_name(encoding: SampleEncoding) -> &'static str {
    match encoding {
        SampleEncoding::Transition => "transition",
        _ => "packed",
    }
}

fn encoding_from(name: &str) -> SampleEncoding {
    match name {
        "transition" => SampleEncoding::Transition,
        _ => SampleEncoding::Packed,
    }
}

fn sample_type(name: &str) -> SampleType {
    match name {
        "int8" => SampleType::SampleInt8,
        "int16" => SampleType::SampleInt16,
        "uint16" => SampleType::SampleUint16,
        "float32" => SampleType::SampleFloat32,
        "float64" => SampleType::SampleFloat64,
        _ => SampleType::SampleUint8,
    }
}

/// Bytes one sample of `type` occupies, which run-length decoding needs.
fn sample_width(sample_type: SampleType) -> usize {
    match sample_type {
        SampleType::SampleInt16 | SampleType::SampleUint16 => 2,
        SampleType::SampleFloat32 => 4,
        SampleType::SampleFloat64 => 8,
        _ => 1,
    }
}

fn error_code(name: &str) -> ErrorCode {
    match name {
        "invalid_request" => ErrorCode::ErrorInvalidRequest,
        "unsupported" => ErrorCode::ErrorUnsupported,
        "device_disconnected" => ErrorCode::ErrorDeviceDisconnected,
        "busy" => ErrorCode::ErrorBusy,
        "timeout" => ErrorCode::ErrorTimeout,
        "internal" => ErrorCode::ErrorInternal,
        _ => ErrorCode::ErrorDevice,
    }
}

fn device_error(e: LinkError) -> proto::Error {
    let code = match e.code.as_str() {
        "" if e.message.contains("disconnected") => ErrorCode::ErrorDeviceDisconnected,
        "" => ErrorCode::ErrorDevice,
        name => error_code(name),
    };
    proto::Error::new(code, e.message)
}

fn double_set(value: &Value) -> Option<DoubleSet> {
    let values: Vec<f64> = value["values"].as_array()?.iter().filter_map(Value::as_f64).collect();
    let range = value.get("range").map(|r| DoubleRange {
        min: r["min"].as_f64().unwrap_or(0.0),
        max: r["max"].as_f64().unwrap_or(0.0),
        step: r["step"].as_f64().unwrap_or(0.0),
    });
    Some(DoubleSet { values, range })
}

fn uint_set(value: &Value) -> Option<UintSet> {
    let values: Vec<u64> =
        value["values"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
    let range = value.get("range").map(|r| UintRange {
        min: r["min"].as_u64().unwrap_or(0),
        max: r["max"].as_u64().unwrap_or(0),
        step: r["step"].as_u64().unwrap_or(0),
    });
    Some(UintSet { values, range })
}

// --- channels -------------------------------------------------------------

/// The channel list, which fixes what the device's enable masks mean.
struct Channels {
    all: Vec<Channel>,
    logic: Vec<String>,
    analog: Vec<String>,
}

impl Channels {
    fn parse(describe: &Value) -> Channels {
        let mut all = Vec::new();
        let (mut logic, mut analog) = (Vec::new(), Vec::new());
        for channel in describe["channels"].as_array().into_iter().flatten() {
            let id = channel["id"].as_str().unwrap_or_default().to_string();
            let kind = match channel["kind"].as_str() {
                Some("analog") => ChannelKind::ChannelAnalog,
                _ => ChannelKind::ChannelLogic,
            };
            match kind {
                ChannelKind::ChannelAnalog => analog.push(id.clone()),
                _ => logic.push(id.clone()),
            }
            all.push(Channel {
                name: channel["name"].as_str().unwrap_or(&id).to_string(),
                id,
                kind: kind as i32,
                index: channel["index"].as_u64().unwrap_or(0) as u32,
                ..Default::default()
            });
        }
        Channels { all, logic, analog }
    }

    /// (mask, bit) the device uses for `id`.
    fn slot(&self, id: &str) -> Option<(&'static str, u64)> {
        if let Some(bit) = self.logic.iter().position(|c| c == id) {
            return Some(("logic_mask", 1 << bit));
        }
        let bit = self.analog.iter().position(|c| c == id)?;
        Some(("analog_mask", 1 << bit))
    }

    fn enabled(&self, id: &str, values: &Value) -> bool {
        match self.slot(id) {
            Some((mask, bit)) => values[mask].as_u64().unwrap_or(0) & bit != 0,
            None => false,
        }
    }
}

// --- config ---------------------------------------------------------------

fn to_config(values: &Value, channels: &Channels) -> Config {
    let samplerate = values["samplerate"].as_f64().unwrap_or(0.0);
    let sample_depth = values["sample_depth"].as_u64().unwrap_or(0);
    Config {
        device: Some(DeviceConfig {
            samplerate: Some(samplerate),
            sample_depth: Some(sample_depth),
            trigger: Some(to_trigger(&values["trigger"])),
            averaging: Some(1),
            capture_span: (samplerate > 0.0).then(|| sample_depth as f64 / samplerate),
        }),
        channels: channels
            .all
            .iter()
            .map(|channel| proto::ChannelConfig {
                id: channel.id.clone(),
                enabled: Some(channels.enabled(&channel.id, values)),
                ..Default::default()
            })
            .collect(),
        vendor: HashMap::from([(
            "cal_freq".to_string(),
            values["cal_freq"].as_f64().unwrap_or(0.0).to_string(),
        )]),
    }
}

fn to_trigger(value: &Value) -> proto::Trigger {
    let position = value["position"].as_f64().unwrap_or(0.0);
    let trigger = match value["kind"].as_str() {
        Some("edge") => trigger::Trigger::Edge(EdgeTrigger {
            channel: value["channel"].as_str().unwrap_or_default().to_string(),
            slope: match value["slope"].as_str() {
                Some("falling") => Slope::Falling as i32,
                Some("either") => Slope::Either as i32,
                _ => Slope::Rising as i32,
            },
            level: value["level"].as_f64().unwrap_or(0.0),
        }),
        _ => trigger::Trigger::None(Empty {}),
    };
    proto::Trigger { trigger: Some(trigger), position }
}

/// One `config.set` object carrying every field the request touches. The
/// device takes whole enable masks, so they are edited against the values it
/// currently reports.
fn set_params(config: &Config, current: &Value, channels: &Channels) -> Reply<Value> {
    let mut params = serde_json::Map::new();

    if let Some(device) = &config.device {
        if let Some(rate) = device.samplerate {
            if !(rate.is_finite() && rate > 0.0) {
                return Err(proto::Error::invalid("samplerate must be positive"));
            }
            params.insert("samplerate".into(), json!(rate));
        }
        if let Some(depth) = device.sample_depth {
            if depth == 0 {
                return Err(proto::Error::invalid("sample_depth must be positive"));
            }
            params.insert("sample_depth".into(), json!(depth));
        }
        if let Some(trigger) = &device.trigger {
            params.insert("trigger".into(), trigger_params(trigger, channels)?);
        }
        match device.averaging {
            None | Some(1) => {}
            Some(_) => return Err(proto::Error::unsupported("averaging")),
        }
    }

    let mut masks: HashMap<&str, u64> = HashMap::from([
        ("logic_mask", current["logic_mask"].as_u64().unwrap_or(0)),
        ("analog_mask", current["analog_mask"].as_u64().unwrap_or(0)),
    ]);
    let mut touched = false;
    for channel in &config.channels {
        let (mask, bit) = channels
            .slot(&channel.id)
            .ok_or_else(|| proto::Error::invalid(format!("no channel {:?}", channel.id)))?;
        if channel.coupling.is_some_and(|c| c != Coupling::Dc as i32) {
            return Err(proto::Error::unsupported("the Pico ADC is DC coupled"));
        }
        for (name, set) in [
            ("probe_factor", channel.probe_factor.is_some()),
            ("full_scale", channel.full_scale.is_some()),
            ("offset", channel.offset.is_some()),
            ("impedance", channel.impedance.is_some()),
            ("bandwidth_limit", channel.bandwidth_limit.is_some()),
            ("invert", channel.invert.is_some()),
            ("threshold", channel.threshold.is_some()),
        ] {
            if set {
                return Err(proto::Error::unsupported(format!("per-channel {name}")));
            }
        }
        if let Some(enabled) = channel.enabled {
            let slot = masks.get_mut(mask).expect("slot names a mask");
            *slot = if enabled { *slot | bit } else { *slot & !bit };
            touched = true;
        }
    }
    if touched {
        for (mask, value) in masks {
            params.insert(mask.into(), json!(value));
        }
    }

    for (key, value) in &config.vendor {
        if key != "cal_freq" {
            return Err(proto::Error::invalid(format!("no vendor option {key:?}")));
        }
        let hz: f64 = value
            .parse()
            .map_err(|_| proto::Error::invalid(format!("cal_freq {value:?} is not a number")))?;
        params.insert("cal_freq".into(), json!(hz));
    }
    Ok(Value::Object(params))
}

fn trigger_params(trigger: &proto::Trigger, channels: &Channels) -> Reply<Value> {
    if !(0.0..=1.0).contains(&trigger.position) {
        return Err(proto::Error::invalid("trigger position must be 0.0..1.0"));
    }
    match &trigger.trigger {
        None | Some(trigger::Trigger::None(_)) => Ok(json!({ "kind": "none" })),
        Some(trigger::Trigger::Edge(edge)) => {
            if channels.slot(&edge.channel).is_none() {
                return Err(proto::Error::invalid(format!(
                    "no trigger channel {:?}",
                    edge.channel
                )));
            }
            let slope = match Slope::try_from(edge.slope).unwrap_or(Slope::Unspecified) {
                Slope::Falling => "falling",
                Slope::Either => return Err(proto::Error::unsupported("either-edge trigger")),
                _ => "rising",
            };
            Ok(json!({
                "kind": "edge",
                "channel": edge.channel,
                "slope": slope,
                "level": edge.level,
                "position": trigger.position,
            }))
        }
        Some(_) => Err(proto::Error::unsupported("only edge triggers")),
    }
}

// --- plugin ---------------------------------------------------------------

/// What the pump needs while a capture is running.
struct Running {
    /// The frontend's id for this capture; the device numbers its own.
    capture_id: u64,
    transition: bool,
    unitsize: HashMap<u32, usize>,
}

type Shared = Arc<Mutex<Option<Running>>>;

struct PicoPlugin {
    device_url: String,
    link: Option<Arc<Link>>,
    channels: Option<Channels>,
    capture: Shared,
    encodings: Vec<i32>,
}

impl PicoPlugin {
    fn new(device_url: String) -> Self {
        PicoPlugin {
            device_url,
            link: None,
            channels: None,
            capture: Arc::new(Mutex::new(None)),
            encodings: Vec::new(),
        }
    }

    fn link(&self) -> Reply<&Arc<Link>> {
        self.link.as_ref().ok_or_else(|| proto::Error::device("no device open"))
    }

    fn channels(&self) -> Reply<&Channels> {
        self.channels.as_ref().ok_or_else(|| proto::Error::device("no device open"))
    }

    fn request(&self, method: &str, params: Value) -> Reply<Value> {
        self.link()?.request(method, params, REQUEST_TIMEOUT).map_err(device_error)
    }
}

impl CaptureServer for PicoPlugin {
    fn hello(&mut self, req: &Hello, events: &Arc<Events>) -> Reply<HelloResult> {
        let target = resolve(&self.device_url).map_err(proto::Error::device)?;
        let (link, notifications) = Link::open(&target.path)
            .map_err(|e| proto::Error::device(format!("{}: {e}", target.path)))?;
        self.link = Some(link);

        let accept: Vec<&str> = req
            .accept_encodings
            .iter()
            .filter_map(|e| SampleEncoding::try_from(*e).ok())
            .map(encoding_name)
            .collect();
        let hello = self.request(
            "hello",
            json!({
                "protocol": openmso::PROTOCOL_VERSION,
                "client_name": "rpi-pico",
                "client_version": env!("CARGO_PKG_VERSION"),
                "accept_encodings": accept,
                "accept_codecs": ["none"],
            }),
        )?;

        // The device reports what it will emit; the frontend is only offered
        // what both ends can handle.
        let offered: Vec<SampleEncoding> = hello["encodings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(encoding_from)
            .collect();
        self.encodings = negotiate_encodings(&req.accept_encodings, &offered);

        let channels = Channels::parse(&self.request("describe", json!({}))?);
        self.channels = Some(channels);

        let device = &hello["device"];
        let result = HelloResult {
            protocol: openmso::PROTOCOL_VERSION,
            plugin: Some(PluginInfo {
                name: "rpi-pico".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                vendor: "OpenMSO".to_string(),
                description: "Raspberry Pi Pico running the OpenMSO Pico firmware".to_string(),
            }),
            capabilities: Some(Capabilities {
                modes: hello["capabilities"]["modes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|mode| match mode.as_str() {
                        Some("single") => Some(proto::AcquireMode::AcquireSingle as i32),
                        Some("continuous") => Some(proto::AcquireMode::AcquireContinuous as i32),
                        Some("snapshot") => Some(proto::AcquireMode::AcquireSnapshot as i32),
                        _ => None,
                    })
                    .collect(),
                trigger_kinds: hello["capabilities"]["trigger_kinds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|kind| match kind.as_str() {
                        Some("none") => Some(TriggerKind::TriggerNone as i32),
                        Some("edge") => Some(TriggerKind::TriggerEdge as i32),
                        Some("pulse") => Some(TriggerKind::TriggerPulse as i32),
                        Some("pattern") => Some(TriggerKind::TriggerPattern as i32),
                        _ => None,
                    })
                    .collect(),
            }),
            device: Some(DeviceInfo {
                vendor: device["vendor"].as_str().unwrap_or("Raspberry Pi").to_string(),
                model: device["model"].as_str().unwrap_or("Pico MSO").to_string(),
                serial: match device["serial"].as_str() {
                    Some(serial) if !serial.is_empty() => serial.to_string(),
                    _ => target.serial,
                },
                firmware_version: device["firmware_version"].as_str().unwrap_or("").to_string(),
            }),
            encodings: self.encodings.clone(),
            codecs: negotiate_codecs(&req.accept_codecs, &CODECS),
        };

        let (events, capture) = (events.clone(), self.capture.clone());
        thread::spawn(move || pump(notifications, &events, &capture));
        Ok(result)
    }

    fn describe(&mut self) -> Reply<Description> {
        let describe = self.request("describe", json!({}))?;
        let channels = Channels::parse(&describe);
        let limits = &describe["limits"];
        let analog = &describe["analog"];
        let description = Description {
            channels: channels.all.clone(),
            limits: Some(DeviceLimits {
                samplerate: double_set(&limits["samplerate"]),
                sample_depth: uint_set(&limits["sample_depth"]),
                samplerate_settable: limits["samplerate_settable"].as_bool().unwrap_or(false),
                sample_depth_settable: limits["sample_depth_settable"].as_bool().unwrap_or(false),
                averaging: None,
                trigger_position: limits["trigger_position"].as_bool().unwrap_or(false),
                max_enabled_channels: limits["max_enabled_channels"].as_u64().unwrap_or(0) as u32,
            }),
            analog: analog.is_object().then(|| AnalogLimits {
                full_scale: double_set(&analog["full_scale"]),
                probe_factors: double_set(&analog["probe_factors"]),
                couplings: analog["couplings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|coupling| match coupling.as_str() {
                        Some("dc") => Some(Coupling::Dc as i32),
                        Some("ac") => Some(Coupling::Ac as i32),
                        Some("gnd") => Some(Coupling::Gnd as i32),
                        _ => None,
                    })
                    .collect(),
                vertical_divisions: analog["vertical_divisions"].as_u64().unwrap_or(0) as u32,
                ..Default::default()
            }),
            logic: describe["logic"].is_object().then(LogicLimits::default),
            vendor_options: describe["vendor_options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|option| VendorOption {
                    key: option["key"].as_str().unwrap_or_default().to_string(),
                    description: option["description"].as_str().unwrap_or_default().to_string(),
                    values: option["values"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                })
                .collect(),
        };
        self.channels = Some(channels);
        Ok(description)
    }

    fn get_config(&mut self) -> Reply<Config> {
        let values = self.request("config.get", json!({}))?;
        Ok(to_config(&values, self.channels()?))
    }

    fn set_config(&mut self, config: &Config) -> Reply<Config> {
        let current = self.request("config.get", json!({}))?;
        let params = set_params(config, &current, self.channels()?)?;
        // config.set answers with the whole config, snapped to what the
        // hardware could take.
        let values = self.request("config.set", params)?;
        Ok(to_config(&values, self.channels()?))
    }

    fn acquire_start(&mut self, req: &AcquireStart, _events: &Arc<Events>) -> Reply<()> {
        let mode = match proto::AcquireMode::try_from(req.mode).unwrap_or_default() {
            proto::AcquireMode::AcquireSingle => "single",
            proto::AcquireMode::AcquireContinuous => "continuous",
            proto::AcquireMode::AcquireSnapshot => "snapshot",
            proto::AcquireMode::Unspecified => {
                return Err(proto::Error::invalid("no acquire mode"))
            }
        };
        let transition = accepts(&self.encodings, SampleEncoding::Transition);
        let encoding = match transition {
            true => "transition",
            false => "packed",
        };

        *self.capture.lock().unwrap() =
            Some(Running { capture_id: req.capture_id, transition, unitsize: HashMap::new() });
        let started = self.request(
            "acquire.start",
            json!({ "capture_id": req.capture_id, "mode": mode, "encoding": encoding }),
        );
        if let Err(e) = started {
            *self.capture.lock().unwrap() = None;
            return Err(e);
        }
        Ok(())
    }

    fn acquire_stop(&mut self, _req: &AcquireStop) -> Reply<()> {
        // Not a blocking request: while armed the firmware answers nothing
        // else, so waiting here would deadlock against the capture being
        // stopped.
        self.link()?.send_nowait("acquire.stop", json!({})).map_err(device_error)
    }

    fn reset(&mut self) -> Reply<()> {
        let link = self.link()?;
        link.send_nowait("acquire.stop", json!({})).map_err(device_error)?;
        link.send_nowait("reset", json!({})).map_err(device_error)?;
        *self.capture.lock().unwrap() = None;
        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(link) = self.link.take() {
            link.send_nowait("acquire.stop", json!({})).ok();
            link.request("shutdown", json!({}), SHUTDOWN_TIMEOUT).ok();
        }
    }
}

// --- notification pump ----------------------------------------------------

/// Relays the device's notifications as events until the link dies.
fn pump(notifications: Receiver<Frame>, events: &Arc<Events>, capture: &Shared) {
    for frame in notifications {
        let mut guard = capture.lock().unwrap();
        let sent = match (frame.method(), guard.as_mut()) {
            ("event.status", _) => status(frame.params(), events),
            ("event.log", _) => log(frame.params(), events),
            ("capture.begin", Some(running)) => begin(frame.params(), events, running),
            ("acquisition.begin", Some(running)) => {
                let params = frame.params();
                events.send(event::Event::AcquisitionBegin(AcquisitionBegin {
                    capture_id: running.capture_id,
                    acquisition: params["acquisition"].as_u64().unwrap_or(0),
                    t0: params["t0"].as_f64().unwrap_or(0.0),
                    sample_count: params["sample_count"].as_u64().unwrap_or(0),
                }))
            }
            ("capture.trigger", Some(running)) => {
                let params = frame.params();
                events.send(event::Event::Trigger(CaptureTrigger {
                    capture_id: running.capture_id,
                    acquisition: params["acquisition"].as_u64().unwrap_or(0),
                    sample: params["sample"].as_u64().unwrap_or(0),
                }))
            }
            ("capture.data", Some(running)) => data(&frame, events, running),
            ("acquisition.end", Some(running)) => {
                let params = frame.params();
                events.send(event::Event::AcquisitionEnd(AcquisitionEnd {
                    capture_id: running.capture_id,
                    acquisition: params["acquisition"].as_u64().unwrap_or(0),
                    dropped_samples: params["dropped_samples"].as_u64().unwrap_or(0),
                }))
            }
            ("capture.end", Some(_)) => {
                let running = guard.take().expect("matched Some");
                end(frame.params(), events, &running)
            }
            _ => Ok(()),
        };
        drop(guard);
        if sent.is_err() {
            return;
        }
    }

    // The reader thread ended: the port is gone, so anything still running is
    // over whether the device said so or not.
    if let Some(running) = capture.lock().unwrap().take() {
        events
            .send(event::Event::CaptureEnd(CaptureEnd {
                capture_id: running.capture_id,
                error: Some(proto::Error::new(
                    ErrorCode::ErrorDeviceDisconnected,
                    "device disconnected during capture",
                )),
            }))
            .ok();
    }
    events
        .send(event::Event::DeviceLost(DeviceLost { reason: "USB-CDC port closed".to_string() }))
        .ok();
}

fn status(params: &Value, events: &Arc<Events>) -> openmso::Result<()> {
    let state = match params["state"].as_str() {
        Some("idle") => State::Idle,
        Some("armed") => State::Armed,
        Some("triggered") => State::Triggered,
        Some("transferring") => State::Transferring,
        Some("stopping") => State::Stopping,
        // Anything else is a state this plugin does not know; the detail
        // still reaches the frontend.
        _ => State::Unspecified,
    };
    events.send(event::Event::Status(Status {
        state: state as i32,
        detail: params["detail"].as_str().unwrap_or_default().to_string(),
    }))
}

fn log(params: &Value, events: &Arc<Events>) -> openmso::Result<()> {
    let level = match params["level"].as_str() {
        Some("debug") => LogLevel::LogDebug,
        Some("warning") => LogLevel::LogWarning,
        Some("error") => LogLevel::LogError,
        _ => LogLevel::LogInfo,
    };
    events.send(event::Event::Log(Log {
        level: level as i32,
        message: params["message"].as_str().unwrap_or_default().to_string(),
    }))
}

fn begin(params: &Value, events: &Arc<Events>, running: &mut Running) -> openmso::Result<()> {
    let mut streams = Vec::new();
    for stream in params["streams"].as_array().into_iter().flatten() {
        let id = stream["id"].as_u64().unwrap_or(0) as u32;
        let format = if let Some(analog) = stream.get("analog") {
            let sample_type = sample_type(analog["type"].as_str().unwrap_or("uint8"));
            running.unitsize.insert(id, sample_width(sample_type));
            stream::Format::Analog(AnalogFormat {
                r#type: sample_type as i32,
                scale: analog["scale"].as_f64().unwrap_or(1.0),
                offset: analog["offset"].as_f64().unwrap_or(0.0),
                unit: analog["unit"].as_str().unwrap_or("V").to_string(),
                digits: analog["digits"].as_u64().unwrap_or(3) as u32,
            })
        } else {
            let unitsize = stream["logic"]["unitsize"].as_u64().unwrap_or(1).max(1);
            running.unitsize.insert(id, unitsize as usize);
            stream::Format::Logic(LogicFormat { unitsize: unitsize as u32 })
        };
        streams.push(Stream {
            id,
            channels: stream["channels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            format: Some(format),
        });
    }

    events.send(event::Event::CaptureBegin(CaptureBegin {
        capture_id: running.capture_id,
        samplerate: params["samplerate"].as_f64().unwrap_or(0.0),
        streams,
    }))
}

fn data(frame: &Frame, events: &Arc<Events>, running: &Running) -> openmso::Result<()> {
    let params = frame.params();
    let stream = params["stream"].as_u64().unwrap_or(0) as u32;
    let samples = params["sample_count"].as_u64().unwrap_or(0);
    let unitsize = running.unitsize.get(&stream).copied().unwrap_or(1);
    let encoding = encoding_from(params["encoding"].as_str().unwrap_or("packed"));

    // The device only run-length encodes when asked, but a frontend that
    // cannot decode it must never see it.
    let (encoding, payload) = match encoding {
        SampleEncoding::Transition if !running.transition => (
            SampleEncoding::Packed,
            decode_transition(&frame.payload, unitsize, samples as usize)?,
        ),
        encoding => (encoding, frame.payload.clone()),
    };

    events.send(event::Event::Data(CaptureData {
        capture_id: running.capture_id,
        acquisition: params["acquisition"].as_u64().unwrap_or(0),
        stream,
        seq: params["seq"].as_u64().unwrap_or(0),
        first_sample: params["first_sample"].as_u64().unwrap_or(0),
        sample_count: samples,
        encoding: encoding as i32,
        codec: Codec::None as i32,
        decoded_len: payload.len() as u64,
        payload: payload.into(),
    }))
}

fn end(params: &Value, events: &Arc<Events>, running: &Running) -> openmso::Result<()> {
    let error = params.get("error").filter(|e| e.is_object()).map(|error| {
        proto::Error::new(
            error_code(error["code"].as_str().unwrap_or_default()),
            error["message"].as_str().unwrap_or("capture failed").to_string(),
        )
    });
    events.send(event::Event::CaptureEnd(CaptureEnd { capture_id: running.capture_id, error }))
}

fn main() -> ExitCode {
    let args = match Args::from_env() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("rpi-pico: {e}");
            return ExitCode::FAILURE;
        }
    };
    openmso::server::exit_on_stdin_eof();
    let mut plugin = PicoPlugin::new(args.device.clone());
    match openmso::server::serve(&args, &mut plugin) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rpi-pico: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmso::proto::ChannelConfig;

    /// A describe reply in the shape the firmware sends, trimmed to two logic
    /// and two analog channels.
    fn describe() -> Value {
        json!({
            "channels": [
                {"id": "A0", "name": "A0", "kind": "analog", "index": 0},
                {"id": "A1", "name": "A1", "kind": "analog", "index": 1},
                {"id": "D0", "name": "D0", "kind": "logic", "index": 0},
                {"id": "D1", "name": "D1", "kind": "logic", "index": 1}
            ]
        })
    }

    fn values() -> Value {
        json!({
            "samplerate": 12000000, "sample_depth": 4096, "memory_depth": 200000,
            "logic_mask": 0b11, "analog_mask": 0b01, "cal_freq": 1000,
            "trigger": {"kind": "none", "position": 0.0}
        })
    }

    #[test]
    fn enable_masks_expand_to_per_channel_flags() {
        let channels = Channels::parse(&describe());
        let config = to_config(&values(), &channels);

        let enabled = |id: &str| {
            config.channels.iter().find(|c| c.id == id).and_then(|c| c.enabled).unwrap()
        };
        assert!(enabled("A0"));
        assert!(!enabled("A1"), "analog_mask bit 1 is clear");
        assert!(enabled("D0") && enabled("D1"));
        // capture_span is derived, never reported by the device.
        assert_eq!(config.device.unwrap().capture_span, Some(4096.0 / 12e6));
    }

    #[test]
    fn a_channel_switch_edits_the_mask_it_belongs_to() {
        let channels = Channels::parse(&describe());
        let config = Config {
            channels: vec![
                ChannelConfig { id: "D1".into(), enabled: Some(false), ..Default::default() },
                ChannelConfig { id: "A1".into(), enabled: Some(true), ..Default::default() },
            ],
            ..Default::default()
        };
        let params = set_params(&config, &values(), &channels).unwrap();
        assert_eq!(params["logic_mask"], json!(0b01));
        assert_eq!(params["analog_mask"], json!(0b11));
    }

    #[test]
    fn masks_are_left_alone_when_no_channel_is_touched() {
        let channels = Channels::parse(&describe());
        let config = Config {
            device: Some(DeviceConfig { samplerate: Some(24e6), ..Default::default() }),
            ..Default::default()
        };
        let params = set_params(&config, &values(), &channels).unwrap();
        assert_eq!(params["samplerate"], json!(24e6));
        assert!(params.get("logic_mask").is_none(), "an untouched mask is not resent");
    }

    #[test]
    fn triggers_round_trip_through_the_device_shape() {
        let channels = Channels::parse(&describe());
        let edge = proto::Trigger {
            trigger: Some(trigger::Trigger::Edge(EdgeTrigger {
                channel: "D1".into(),
                slope: Slope::Falling as i32,
                level: 1.5,
            })),
            position: 0.25,
        };
        let params = trigger_params(&edge, &channels).unwrap();
        assert_eq!(params["kind"], "edge");
        assert_eq!(params["channel"], "D1");
        assert_eq!(params["slope"], "falling");
        assert_eq!(params["position"], json!(0.25));

        let back = to_trigger(&params);
        assert_eq!(back, edge);

        let free_run = to_trigger(&json!({"kind": "none", "position": 0.0}));
        assert!(matches!(free_run.trigger, Some(trigger::Trigger::None(_))));
    }

    #[test]
    fn what_the_device_cannot_do_is_refused_by_code() {
        let channels = Channels::parse(&describe());
        let refuse = |config: Config| {
            let code = set_params(&config, &values(), &channels).unwrap_err().code;
            ErrorCode::try_from(code).unwrap()
        };
        let device = |device: DeviceConfig| Config { device: Some(device), ..Default::default() };
        let channel = |channel: ChannelConfig| Config {
            channels: vec![channel],
            ..Default::default()
        };

        assert_eq!(
            refuse(device(DeviceConfig { samplerate: Some(0.0), ..Default::default() })),
            ErrorCode::ErrorInvalidRequest
        );
        assert_eq!(
            refuse(device(DeviceConfig { sample_depth: Some(0), ..Default::default() })),
            ErrorCode::ErrorInvalidRequest
        );
        assert_eq!(
            refuse(device(DeviceConfig { averaging: Some(8), ..Default::default() })),
            ErrorCode::ErrorUnsupported
        );
        assert_eq!(
            refuse(channel(ChannelConfig { id: "D9".into(), ..Default::default() })),
            ErrorCode::ErrorInvalidRequest
        );
        assert_eq!(
            refuse(channel(ChannelConfig {
                id: "A0".into(),
                coupling: Some(Coupling::Ac as i32),
                ..Default::default()
            })),
            ErrorCode::ErrorUnsupported
        );
        assert_eq!(
            refuse(channel(ChannelConfig {
                id: "A0".into(),
                full_scale: Some(5.0),
                ..Default::default()
            })),
            ErrorCode::ErrorUnsupported
        );
        assert_eq!(
            refuse(Config {
                vendor: HashMap::from([("wat".to_string(), "1".to_string())]),
                ..Default::default()
            }),
            ErrorCode::ErrorInvalidRequest
        );

        let either = proto::Trigger {
            trigger: Some(trigger::Trigger::Edge(EdgeTrigger {
                channel: "D0".into(),
                slope: Slope::Either as i32,
                level: 0.0,
            })),
            position: 0.0,
        };
        let code = trigger_params(&either, &channels).unwrap_err().code;
        assert_eq!(ErrorCode::try_from(code).unwrap(), ErrorCode::ErrorUnsupported);
    }

    #[test]
    fn device_error_codes_survive_the_hop() {
        let refused = LinkError {
            code: "unsupported".to_string(),
            message: "acquire.start: enable logic or analog channels, not both".to_string(),
        };
        assert_eq!(device_error(refused).code, ErrorCode::ErrorUnsupported as i32);

        // A link failure has no device code of its own.
        let gone = LinkError { code: String::new(), message: "device disconnected".to_string() };
        assert_eq!(device_error(gone).code, ErrorCode::ErrorDeviceDisconnected as i32);
    }

    #[test]
    fn stream_formats_carry_the_widths_decoding_needs() {
        let mut running =
            Running { capture_id: 7, transition: true, unitsize: HashMap::new() };
        let params = json!({
            "capture_id": 1, "samplerate": 12e6,
            "streams": [
                {"id": 0, "channels": ["D0", "D1"], "logic": {"unitsize": 2}},
                {"id": 1, "channels": ["A0"],
                 "analog": {"type": "uint8", "scale": 0.0129, "offset": 0.0,
                            "unit": "V", "digits": 3}}
            ]
        });
        // Parsed for its side effect on `running`; sending needs a live socket.
        let streams: Vec<Stream> = params["streams"]
            .as_array()
            .unwrap()
            .iter()
            .map(|stream| {
                let id = stream["id"].as_u64().unwrap() as u32;
                if let Some(analog) = stream.get("analog") {
                    let t = sample_type(analog["type"].as_str().unwrap());
                    running.unitsize.insert(id, sample_width(t));
                } else {
                    let unitsize = stream["logic"]["unitsize"].as_u64().unwrap() as usize;
                    running.unitsize.insert(id, unitsize);
                }
                Stream { id, ..Default::default() }
            })
            .collect();

        assert_eq!(streams.len(), 2);
        assert_eq!(running.unitsize[&0], 2, "16 logic channels pack into 2 bytes");
        assert_eq!(running.unitsize[&1], 1, "uint8 analog codes are one byte");
    }

    #[test]
    fn url_schemes_this_plugin_does_not_handle_are_refused() {
        assert!(resolve("usb:04b4:8613").is_err(), "no scheme separator");
        assert!(resolve("tcp://192.168.1.155:5025").is_err());
    }
}
