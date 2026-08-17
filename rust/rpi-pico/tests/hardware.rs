// SPDX-License-Identifier: Apache-2.0
//! The plugin driven against a real Pico, skipped when none is attached.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use openmso::client::CaptureClient;
use openmso::encoding::decode_payload;
use openmso::proto::{
    event, stream, AcquireMode, ChannelConfig, ChannelKind, Config, DeviceConfig, SampleEncoding,
};

const DEPTH: u64 = 8192;
const SAMPLERATE: f64 = 12e6;

/// The device serves one connection at a time, and cargo runs tests in
/// parallel by default.
static PORT: Mutex<()> = Mutex::new(());

fn exclusive() -> MutexGuard<'static, ()> {
    // A test that panicked while holding it left the device idle, not broken.
    PORT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The by-id link a Pico running this firmware shows up as.
fn attached() -> Option<String> {
    std::fs::read_dir("/dev/serial/by-id").ok()?.flatten().find_map(|entry| {
        let name = entry.file_name().into_string().ok()?;
        name.contains("OpenMSO_Pico_MSO_").then(|| format!("usb://1209:0001"))
    })
}

fn launch(device: &str) -> CaptureClient {
    let client =
        CaptureClient::launch(&[env!("CARGO_BIN_EXE_rpi-pico").to_string()], device).unwrap();
    client.set_event_timeout(Some(Duration::from_secs(30))).unwrap();
    client
}

/// Every logic channel off, so the device takes an analog-only capture; it
/// refuses a mix of the two.
fn logic_off(description: &openmso::proto::Description) -> Vec<ChannelConfig> {
    description
        .channels
        .iter()
        .filter(|c| c.kind == ChannelKind::ChannelLogic as i32)
        .map(|c| ChannelConfig { id: c.id.clone(), enabled: Some(false), ..Default::default() })
        .collect()
}

fn collect(client: &CaptureClient) -> (HashMap<u32, Vec<u8>>, HashMap<u32, usize>) {
    let mut unitsize: HashMap<u32, usize> = HashMap::new();
    let mut streams: HashMap<u32, Vec<u8>> = HashMap::new();
    loop {
        match client.next_event().unwrap().event.expect("event with no arm") {
            event::Event::CaptureBegin(begin) => {
                for stream in &begin.streams {
                    let unit = match stream.format {
                        Some(stream::Format::Logic(logic)) => logic.unitsize as usize,
                        _ => 1,
                    };
                    unitsize.insert(stream.id, unit);
                }
            }
            event::Event::Data(data) => {
                let unit = unitsize.get(&data.stream).copied().unwrap_or(1);
                let packed = decode_payload(&data, unit).unwrap();
                assert_eq!(packed.len() as u64, data.sample_count * unit as u64);
                streams.entry(data.stream).or_default().extend_from_slice(&packed);
            }
            event::Event::CaptureEnd(end) => {
                assert_eq!(end.error, None, "capture failed");
                return (streams, unitsize);
            }
            _ => {}
        }
    }
}

#[test]
fn a_logic_capture_matches_the_config_it_was_given() {
    let _port = exclusive();
    let Some(device) = attached() else {
        eprintln!("no Pico attached; skipping");
        return;
    };
    let mut client = launch(&device);

    let hello = client.hello("hardware-test", "0").unwrap();
    assert_eq!(hello.protocol, openmso::PROTOCOL_VERSION);
    assert_eq!(hello.plugin.unwrap().name, "rpi-pico");
    let info = hello.device.unwrap();
    assert!(info.model.contains("Pico"), "model was {:?}", info.model);
    assert!(!info.serial.is_empty(), "the by-id name carries a serial");
    // The device run-length encodes, and the client offered to decode it.
    assert!(hello.encodings.contains(&(SampleEncoding::Transition as i32)));

    let description = client.describe().unwrap();
    assert!(description.channels.iter().any(|c| c.id == "D0"));
    let limits = description.limits.unwrap();
    assert!(!limits.samplerate.unwrap().values.is_empty(), "a rate ladder, not a range");
    assert!(limits.sample_depth.unwrap().range.unwrap().max >= DEPTH);

    let settled = client
        .set_config(Config {
            device: Some(DeviceConfig {
                samplerate: Some(SAMPLERATE),
                sample_depth: Some(DEPTH),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
    let device_config = settled.device.unwrap();
    assert_eq!(device_config.samplerate, Some(SAMPLERATE));
    assert_eq!(device_config.sample_depth, Some(DEPTH));
    assert!(settled.channels.iter().any(|c| c.id == "D0" && c.enabled == Some(true)));

    let capture_id = client.next_capture_id();
    client.acquire_start(capture_id, AcquireMode::AcquireSingle).unwrap();
    let (streams, unitsize) = collect(&client);

    assert_eq!(streams.len(), 1, "logic channels share one packed stream");
    let (id, samples) = streams.iter().next().unwrap();
    assert_eq!(samples.len() as u64, DEPTH * unitsize[id] as u64);
    client.shutdown().unwrap();
}

#[test]
fn an_analog_capture_carries_volts_within_the_adc_range() {
    let _port = exclusive();
    let Some(device) = attached() else {
        eprintln!("no Pico attached; skipping");
        return;
    };
    let mut client = launch(&device);
    client.hello("hardware-test", "0").unwrap();
    let description = client.describe().unwrap();

    let mut channels = logic_off(&description);
    channels.push(ChannelConfig {
        id: "A0".to_string(),
        enabled: Some(true),
        ..Default::default()
    });
    let settled = client
        .set_config(Config {
            device: Some(DeviceConfig { sample_depth: Some(4096), ..Default::default() }),
            channels,
            ..Default::default()
        })
        .unwrap();
    assert!(settled.channels.iter().any(|c| c.id == "A0" && c.enabled == Some(true)));
    assert!(settled.channels.iter().all(|c| !c.id.starts_with('D') || c.enabled == Some(false)));

    let capture_id = client.next_capture_id();
    client.acquire_start(capture_id, AcquireMode::AcquireSingle).unwrap();

    let mut scale = 0.0;
    let mut codes = Vec::new();
    loop {
        match client.next_event().unwrap().event.expect("event with no arm") {
            event::Event::CaptureBegin(begin) => {
                let format = begin.streams.first().and_then(|s| s.format.clone());
                let Some(stream::Format::Analog(analog)) = format else {
                    panic!("analog capture without an analog stream");
                };
                scale = analog.scale;
                assert_eq!(analog.unit, "V");
            }
            event::Event::Data(data) => codes.extend_from_slice(&decode_payload(&data, 1).unwrap()),
            event::Event::CaptureEnd(end) => {
                assert_eq!(end.error, None);
                break;
            }
            _ => {}
        }
    }

    assert_eq!(codes.len(), 4096);
    // 8-bit codes across the 3.3 V reference, so every sample is in range.
    assert!((scale - 3.3 / 256.0).abs() < 1e-9, "scale was {scale}");
    let volts: Vec<f64> = codes.iter().map(|c| *c as f64 * scale).collect();
    assert!(volts.iter().all(|v| (0.0..=3.3).contains(v)));
    client.shutdown().unwrap();
}

#[test]
fn a_capture_the_device_refuses_comes_back_as_an_error() {
    let _port = exclusive();
    let Some(device) = attached() else {
        eprintln!("no Pico attached; skipping");
        return;
    };
    let mut client = launch(&device);
    client.hello("hardware-test", "0").unwrap();
    let description = client.describe().unwrap();

    // Logic and analog at once is the one combination the firmware rejects.
    let mut channels = logic_off(&description);
    for id in ["D0", "A0"] {
        channels.push(ChannelConfig {
            id: id.to_string(),
            enabled: Some(true),
            ..Default::default()
        });
    }
    client.set_config(Config { channels, ..Default::default() }).unwrap();

    let capture_id = client.next_capture_id();
    let refused = client.acquire_start(capture_id, AcquireMode::AcquireSingle);
    let Err(openmso::Error::Remote(error)) = refused else {
        panic!("expected the device's refusal, got {refused:?}");
    };
    assert!(!error.message.is_empty(), "the device's own words reach the frontend");
    client.shutdown().unwrap();
}

#[test]
fn an_unknown_serial_fails_the_handshake() {
    if attached().is_none() {
        eprintln!("no Pico attached; skipping");
        return;
    }
    let mut client = launch("serial://NOSUCHDEVICE");
    let Err(openmso::Error::Remote(error)) = client.hello("hardware-test", "0") else {
        panic!("a device that is not there must fail Hello");
    };
    assert!(error.message.contains("NOSUCHDEVICE"), "message was {:?}", error.message);
}
