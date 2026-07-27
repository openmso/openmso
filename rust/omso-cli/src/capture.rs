// SPDX-License-Identifier: Apache-2.0
//! Collect one acquisition off the event socket and turn it into real units.

use std::collections::HashMap;
use std::time::Instant;

use openmso::client::CaptureClient;
use openmso::encoding::decode_payload;
use openmso::proto::{
    event, stream, AnalogFormat, CaptureBegin, LogLevel, SampleType, State, Stream,
};

pub struct AnalogStream {
    pub name: String,
    pub volts: Vec<f64>,
}

pub struct Capture {
    pub samplerate: f64,
    pub t0: f64,
    pub trigger_sample: Option<u64>,
    pub dropped_samples: u64,
    pub analog: Vec<AnalogStream>,
    pub logic_channels: Vec<String>,
    pub logic: Vec<u8>,
}

/// Read events until the first acquisition is complete, stopping the capture
/// if the device would otherwise keep going, then drain to `CaptureEnd`.
///
/// The control socket stays usable throughout — that separation is the reason
/// there are two sockets.
pub fn collect(
    client: &mut CaptureClient,
    capture_id: u64,
    verbose: bool,
) -> Result<Capture, String> {
    let started = Instant::now();
    let mut begin: Option<CaptureBegin> = None;
    let mut payloads: HashMap<u32, Vec<u8>> = HashMap::new();
    let mut t0 = 0.0;
    let mut trigger_sample = None;
    let mut dropped_samples = 0;
    let mut bytes = 0;
    let mut collecting = true;

    loop {
        let event = client.next_event().map_err(|e| format!("event stream: {e}"))?;
        let Some(event) = event.event else {
            continue;
        };
        match event {
            event::Event::CaptureBegin(b) if b.capture_id == capture_id => begin = Some(b),
            event::Event::AcquisitionBegin(b) if collecting => t0 = b.t0,
            event::Event::Trigger(t) if collecting => trigger_sample = Some(t.sample),
            event::Event::Data(data) if collecting && data.acquisition == 0 => {
                let streams = begin.as_ref().map(|b| &b.streams[..]).unwrap_or_default();
                let unitsize = unitsize(streams, data.stream)?;
                let packed = decode_payload(&data, unitsize).map_err(|e| e.to_string())?;
                bytes += packed.len();
                place(payloads.entry(data.stream).or_default(),
                      data.first_sample as usize * unitsize, &packed);
            }
            event::Event::AcquisitionEnd(end) if collecting && end.acquisition == 0 => {
                dropped_samples = end.dropped_samples;
                collecting = false;
                // One acquisition is all a file can hold, so wind up whatever
                // is still running.
                client.acquire_stop(capture_id).map_err(|e| e.to_string())?;
            }
            event::Event::CaptureEnd(end) => {
                if let Some(error) = end.error {
                    return Err(format!("capture failed: {}", error.message));
                }
                let begin = begin.ok_or("capture ended without a CaptureBegin")?;
                if verbose {
                    let (mb, secs) = (bytes as f64 / 1e6, started.elapsed().as_secs_f64());
                    eprintln!("[transfer: {mb:.2} MB in {secs:.2}s = {:.2} MB/s]", mb / secs);
                }
                return assemble(begin, payloads, t0, trigger_sample, dropped_samples);
            }
            event::Event::DeviceLost(lost) => {
                return Err(format!("device lost: {}", lost.reason))
            }
            event::Event::Log(log) => eprintln!("[plugin:{}] {}", level_name(log.level), log.message),
            event::Event::Status(status) if verbose => {
                eprintln!("[{}]", state_name(status.state))
            }
            _ => {}
        }
    }
}

/// Write a chunk at its absolute offset: chunks are allowed to arrive in any
/// order within a stream, and `first_sample` is what says where they belong.
fn place(buffer: &mut Vec<u8>, offset: usize, data: &[u8]) {
    let end = offset + data.len();
    if buffer.len() < end {
        buffer.resize(end, 0);
    }
    buffer[offset..end].copy_from_slice(data);
}

fn unitsize(streams: &[Stream], id: u32) -> Result<usize, String> {
    let stream = streams
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("data for stream {id}, which CaptureBegin did not declare"))?;
    match &stream.format {
        Some(stream::Format::Logic(logic)) => Ok(logic.unitsize.max(1) as usize),
        Some(stream::Format::Analog(analog)) => Ok(sample_width(analog.r#type)?),
        None => Err(format!("stream {id} has no sample format")),
    }
}

fn assemble(
    begin: CaptureBegin,
    mut payloads: HashMap<u32, Vec<u8>>,
    t0: f64,
    trigger_sample: Option<u64>,
    dropped_samples: u64,
) -> Result<Capture, String> {
    let mut capture = Capture {
        samplerate: begin.samplerate,
        t0,
        trigger_sample,
        dropped_samples,
        analog: Vec::new(),
        logic_channels: Vec::new(),
        logic: Vec::new(),
    };
    for stream in &begin.streams {
        let payload = payloads.remove(&stream.id).unwrap_or_default();
        match &stream.format {
            Some(stream::Format::Analog(format)) => {
                let name = stream.channels.first().cloned()
                    .unwrap_or_else(|| format!("stream{}", stream.id));
                capture.analog.push(AnalogStream { name, volts: decode(&payload, format)? });
            }
            Some(stream::Format::Logic(_)) if !capture.logic_channels.is_empty() => {
                eprintln!("omso-cli: ignoring extra logic stream {}", stream.id);
            }
            Some(stream::Format::Logic(_)) => {
                capture.logic_channels = stream.channels.clone();
                capture.logic = payload;
            }
            None => return Err(format!("stream {} has no sample format", stream.id)),
        }
    }
    Ok(capture)
}

/// Raw device codes to real units: `value = code * scale + offset`.
fn decode(raw: &[u8], format: &AnalogFormat) -> Result<Vec<f64>, String> {
    macro_rules! decode_le {
        ($ty:ty, $width:expr) => {{
            raw.chunks_exact($width)
                .map(|c| <$ty>::from_le_bytes(c.try_into().expect("chunks_exact width")) as f64)
                .collect::<Vec<f64>>()
        }};
    }
    let codes: Vec<f64> = match SampleType::try_from(format.r#type) {
        Ok(SampleType::SampleInt8) => raw.iter().map(|&b| b as i8 as f64).collect(),
        Ok(SampleType::SampleUint8) => raw.iter().map(|&b| b as f64).collect(),
        Ok(SampleType::SampleInt16) => decode_le!(i16, 2),
        Ok(SampleType::SampleUint16) => decode_le!(u16, 2),
        Ok(SampleType::SampleFloat32) => decode_le!(f32, 4),
        Ok(SampleType::SampleFloat64) => decode_le!(f64, 8),
        _ => return Err(format!("unsupported sample type {}", format.r#type)),
    };
    Ok(codes.into_iter().map(|c| c * format.scale + format.offset).collect())
}

fn sample_width(sample_type: i32) -> Result<usize, String> {
    match SampleType::try_from(sample_type) {
        Ok(SampleType::SampleInt8) | Ok(SampleType::SampleUint8) => Ok(1),
        Ok(SampleType::SampleInt16) | Ok(SampleType::SampleUint16) => Ok(2),
        Ok(SampleType::SampleFloat32) => Ok(4),
        Ok(SampleType::SampleFloat64) => Ok(8),
        _ => Err(format!("unsupported sample type {sample_type}")),
    }
}

fn level_name(level: i32) -> &'static str {
    match LogLevel::try_from(level) {
        Ok(LogLevel::LogDebug) => "debug",
        Ok(LogLevel::LogInfo) => "info",
        Ok(LogLevel::LogWarning) => "warning",
        Ok(LogLevel::LogError) => "error",
        _ => "?",
    }
}

fn state_name(state: i32) -> &'static str {
    match State::try_from(state) {
        Ok(State::Idle) => "idle",
        Ok(State::Armed) => "armed",
        Ok(State::Triggered) => "triggered",
        Ok(State::Transferring) => "transferring",
        Ok(State::Stopping) => "stopping",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmso::proto::LogicFormat;

    fn analog(sample_type: SampleType, scale: f64, offset: f64) -> AnalogFormat {
        AnalogFormat { r#type: sample_type as i32, scale, offset, unit: "V".into(), digits: 3 }
    }

    #[test]
    fn int8_codes_scale_to_volts() {
        let v = decode(&[0, 100, (-100i8) as u8], &analog(SampleType::SampleInt8, 0.01, 0.0))
            .unwrap();
        assert_eq!(v.len(), 3);
        assert!((v[0]).abs() < 1e-12);
        assert!((v[1] - 1.0).abs() < 1e-12);
        assert!((v[2] + 1.0).abs() < 1e-12);
    }

    #[test]
    fn offset_is_applied_after_scaling() {
        let v = decode(&[2], &analog(SampleType::SampleUint8, 0.5, 10.0)).unwrap();
        assert!((v[0] - 11.0).abs() < 1e-12);
    }

    #[test]
    fn wide_types_decode_little_endian() {
        let raw = (-2i16).to_le_bytes();
        let v = decode(&raw, &analog(SampleType::SampleInt16, 1.0, 0.0)).unwrap();
        assert!((v[0] + 2.0).abs() < 1e-12);
        let raw = 1.5f32.to_le_bytes();
        let v = decode(&raw, &analog(SampleType::SampleFloat32, 1.0, 0.0)).unwrap();
        assert!((v[0] - 1.5).abs() < 1e-12);
    }

    #[test]
    fn an_unknown_sample_type_is_an_error_not_a_panic() {
        assert!(decode(&[0], &AnalogFormat { r#type: 99, ..Default::default() }).is_err());
        assert!(sample_width(99).is_err());
    }

    #[test]
    fn chunks_land_where_first_sample_says_regardless_of_arrival() {
        let mut buffer = Vec::new();
        place(&mut buffer, 2, b"c");
        place(&mut buffer, 0, b"ab");
        assert_eq!(buffer, b"abc");
    }

    #[test]
    fn unitsize_comes_from_the_stream_it_belongs_to() {
        let streams = vec![
            Stream {
                id: 0,
                format: Some(stream::Format::Analog(analog(SampleType::SampleInt16, 1.0, 0.0))),
                ..Default::default()
            },
            Stream {
                id: 4,
                format: Some(stream::Format::Logic(LogicFormat { unitsize: 2 })),
                ..Default::default()
            },
        ];
        assert_eq!(unitsize(&streams, 0).unwrap(), 2);
        assert_eq!(unitsize(&streams, 4).unwrap(), 2);
        assert!(unitsize(&streams, 9).is_err(), "undeclared stream");
    }
}
