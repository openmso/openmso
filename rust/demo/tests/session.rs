// SPDX-License-Identifier: Apache-2.0
//! The demo plugin driven as a real subprocess over OCP v1, which is also the
//! worked example of what a frontend does.

use std::collections::HashMap;
use std::time::Duration;

use openmso::client::CaptureClient;
use openmso::encoding::decode_payload;
use openmso::proto::{
    event, stream, AcquireMode, Config, DeviceConfig, Event, LogicFormat, SampleEncoding,
};
use openmso::Error;

const DEPTH: u64 = 5000;

fn launch() -> CaptureClient {
    let client =
        CaptureClient::launch(&[env!("CARGO_BIN_EXE_demo").to_string()], "demo://0").unwrap();
    client.set_event_timeout(Some(Duration::from_secs(20))).unwrap();
    client
}

/// Collect one capture, returning each stream's samples in packed form.
fn drain(client: &CaptureClient, unitsize: &HashMap<u32, usize>) -> HashMap<u32, Vec<u8>> {
    let mut streams: HashMap<u32, Vec<u8>> = HashMap::new();
    loop {
        let event: Event = client.next_event().unwrap();
        match event.event.expect("event with no arm") {
            event::Event::Data(data) => {
                let unit = unitsize.get(&data.stream).copied().unwrap_or(1);
                let packed = decode_payload(&data, unit).unwrap();
                assert_eq!(packed.len() as u64, data.sample_count * unit as u64);
                streams.entry(data.stream).or_default().extend_from_slice(&packed);
            }
            event::Event::CaptureEnd(end) => {
                assert_eq!(end.error, None, "capture failed");
                return streams;
            }
            _ => {}
        }
    }
}

#[test]
fn a_single_capture_arrives_with_the_shape_describe_promised() {
    let mut client = launch();

    let hello = client.hello("session-test", "0").unwrap();
    assert_eq!(hello.protocol, openmso::PROTOCOL_VERSION);
    assert_eq!(hello.device.unwrap().model, "Demo MSO");
    assert_eq!(hello.plugin.unwrap().name, "demo");
    // The client offers run-length encoding, so the plugin should take it.
    assert!(hello.encodings.contains(&(SampleEncoding::Transition as i32)));

    let description = client.describe().unwrap();
    assert_eq!(description.channels.len(), 10, "two analog and eight logic");
    assert_eq!(description.vendor_options.len(), 3);

    let settled = client
        .set_config(Config {
            device: Some(DeviceConfig { sample_depth: Some(DEPTH), ..Default::default() }),
            vendor: HashMap::from([("frequency".to_string(), "2000".to_string())]),
            ..Default::default()
        })
        .unwrap();
    let device = settled.device.unwrap();
    assert_eq!(device.sample_depth, Some(DEPTH));
    assert_eq!(device.capture_span, Some(DEPTH as f64 / 1e6), "derived, not set");
    assert_eq!(settled.vendor["frequency"], "2000");

    let capture_id = client.next_capture_id();
    client.acquire_start(capture_id, AcquireMode::AcquireSingle).unwrap();

    let begin = loop {
        match client.next_event().unwrap().event.unwrap() {
            event::Event::CaptureBegin(begin) => break begin,
            _ => continue,
        }
    };
    assert_eq!(begin.capture_id, capture_id);
    assert_eq!(begin.samplerate, 1e6);
    assert_eq!(begin.streams.len(), 3);

    let unitsize: HashMap<u32, usize> = begin
        .streams
        .iter()
        .map(|s| {
            let unit = match &s.format {
                Some(stream::Format::Logic(LogicFormat { unitsize })) => *unitsize as usize,
                // Every analog stream here is int8.
                _ => 1,
            };
            (s.id, unit)
        })
        .collect();

    let streams = drain(&client, &unitsize);
    assert_eq!(streams.len(), 3);
    for (id, samples) in &streams {
        assert_eq!(samples.len() as u64, DEPTH, "stream {id} is short");
    }

    // The logic stream is the last one, and carries the counter and the UART.
    let logic = &streams[&2];
    assert!(logic.iter().any(|b| b & 0x80 != 0) && logic.iter().any(|b| b & 0x80 == 0));
    assert!(logic.iter().map(|b| b & 0x7f).max().unwrap() > 100, "counter should run");

    client.shutdown().unwrap();
}

#[test]
fn a_continuous_capture_repeats_until_it_is_stopped() {
    let mut client = launch();
    client.hello("session-test", "0").unwrap();
    client
        .set_config(Config {
            device: Some(DeviceConfig { sample_depth: Some(DEPTH), ..Default::default() }),
            ..Default::default()
        })
        .unwrap();

    let capture_id = client.next_capture_id();
    client.acquire_start(capture_id, AcquireMode::AcquireContinuous).unwrap();

    // Wait for the second frame, so it is definitely re-arming rather than
    // running once, then stop it while data is still in flight.
    let mut acquisitions = 0;
    while acquisitions < 2 {
        if let event::Event::AcquisitionBegin(begin) = client.next_event().unwrap().event.unwrap() {
            assert_eq!(begin.capture_id, capture_id);
            assert_eq!(begin.sample_count, DEPTH);
            acquisitions = begin.acquisition + 1;
        }
    }
    client.acquire_stop(capture_id).unwrap();

    loop {
        if let event::Event::CaptureEnd(end) = client.next_event().unwrap().event.unwrap() {
            assert_eq!(end.error, None);
            break;
        }
    }

    // Back in READY, so the device is configurable again.
    client.get_config().unwrap();
    client.shutdown().unwrap();
}

#[test]
fn a_device_url_the_plugin_does_not_drive_fails_on_a_live_connection() {
    let mut client =
        CaptureClient::launch(&[env!("CARGO_BIN_EXE_demo").to_string()], "usb://04b4:8613").unwrap();
    // Not a dead process and a line on stderr: a normal Error, in reply.
    match client.hello("session-test", "0") {
        Err(Error::Remote(e)) => {
            assert_eq!(e.code, openmso::proto::ErrorCode::ErrorDevice as i32);
        }
        other => panic!("expected a device error, got {other:?}"),
    }
}
