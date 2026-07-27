// SPDX-License-Identifier: Apache-2.0
//! Independent writer for sigrok's .sr (srzip) session files.
//!
//! Layout (established from libsigrok's src/output/srzip.c, format version 2):
//!
//! - `version`: the ASCII string `2`.
//! - `metadata`: INI. `[device 1]` carries `samplerate` (human string like
//!   "1 MHz"), logic channels as `total probes` + `probe<n>` names (1-based by
//!   channel index) + `unitsize` + `capturefile = logic-1`, analog channels as
//!   `total analog` + `analog<n>` names where numbering starts at
//!   `total probes` + 1.
//! - Logic chunks `logic-1-<n>` (n from 1): bit-packed samples, `unitsize`
//!   bytes/sample, channel i = bit i.
//! - Analog chunks `analog-1-<ch>-<n>`: native-endian float32 samples.
//!
//! This module contains no sigrok code; the layout above is interface fact.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const CHUNK_SAMPLES: usize = 4 * 1024 * 1024;

/// Format a samplerate the way sigrok's metadata expects ("1 MHz").
pub fn samplerate_string(rate: f64) -> String {
    let rate = rate.round() as i64;
    for (mult, suffix) in [(1_000_000_000i64, "GHz"), (1_000_000, "MHz"), (1_000, "kHz")] {
        if rate >= mult && rate % mult == 0 {
            return format!("{} {}", rate / mult, suffix);
        }
    }
    format!("{rate} Hz")
}

pub struct SrZipWriter {
    samplerate: f64,
    logic_channels: Vec<String>,
    analog_channels: Vec<String>,
    unitsize: usize,
    logic: Vec<u8>,
    analog: Vec<Vec<u8>>,
}

impl SrZipWriter {
    /// `logic_channels` are named by bit position; `analog_channels` by index.
    pub fn new(samplerate: f64, logic_channels: Vec<String>,
               analog_channels: Vec<String>) -> Self {
        let unitsize = if logic_channels.is_empty() {
            0
        } else {
            logic_channels.len().div_ceil(8)
        };
        let analog = vec![Vec::new(); analog_channels.len()];
        SrZipWriter { samplerate, logic_channels, analog_channels, unitsize,
                      logic: Vec::new(), analog }
    }

    /// `data`: bit-packed samples, `unitsize` bytes per sample.
    pub fn add_logic(&mut self, data: &[u8]) {
        self.logic.extend_from_slice(data);
    }

    /// `samples`: already scaled to real units.
    pub fn add_analog(&mut self, channel_index: usize, samples: &[f64]) {
        let buf = &mut self.analog[channel_index];
        buf.reserve(samples.len() * 4);
        for &v in samples {
            // srzip stores native-endian float32. Every platform we target is
            // little-endian; being explicit beats relying on the host.
            buf.extend_from_slice(&(v as f32).to_le_bytes());
        }
    }

    fn metadata(&self) -> String {
        let mut lines = vec![
            "[global]".to_string(),
            "sigrok version = 0.5.2 (openmso)".to_string(),
            String::new(),
            "[device 1]".to_string(),
        ];
        if !self.logic_channels.is_empty() {
            lines.push("capturefile = logic-1".to_string());
            lines.push(format!("total probes = {}", self.logic_channels.len()));
        }
        lines.push(format!("samplerate = {}", samplerate_string(self.samplerate)));
        if !self.analog_channels.is_empty() {
            lines.push(format!("total analog = {}", self.analog_channels.len()));
        }
        if !self.logic_channels.is_empty() {
            lines.push(format!("unitsize = {}", self.unitsize));
            for (i, name) in self.logic_channels.iter().enumerate() {
                lines.push(format!("probe{} = {}", i + 1, name));
            }
        }
        let base = self.logic_channels.len();
        for (i, name) in self.analog_channels.iter().enumerate() {
            lines.push(format!("analog{} = {}", base + i + 1, name));
        }
        lines.join("\n") + "\n"
    }

    pub fn write(self, path: &Path) -> io::Result<()> {
        let mut z = ZipWriter::new(File::create(path)?);
        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated);

        z.start_file("version", opts)?;
        z.write_all(b"2")?;
        z.start_file("metadata", opts)?;
        z.write_all(self.metadata().as_bytes())?;

        if !self.logic.is_empty() {
            // max(1) so a caller that adds logic data without declaring logic
            // channels gets a file rather than a divide-by-zero panic.
            let step = (CHUNK_SAMPLES * self.unitsize).max(1);
            for (n, chunk) in self.logic.chunks(step).enumerate() {
                z.start_file(format!("logic-1-{}", n + 1), opts)?;
                z.write_all(chunk)?;
            }
        }

        let base = self.logic_channels.len();
        for (i, blob) in self.analog.iter().enumerate() {
            if blob.is_empty() {
                continue;
            }
            for (n, chunk) in blob.chunks(CHUNK_SAMPLES * 4).enumerate() {
                z.start_file(format!("analog-1-{}-{}", base + i + 1, n + 1), opts)?;
                z.write_all(chunk)?;
            }
        }
        z.finish()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samplerate_strings_match_sigrok_conventions() {
        assert_eq!(samplerate_string(1e9), "1 GHz");
        assert_eq!(samplerate_string(1e6), "1 MHz");
        assert_eq!(samplerate_string(1000.0), "1 kHz");
        // Not a whole number of MHz, so it degrades to the next unit down.
        assert_eq!(samplerate_string(2.5e6), "2500 kHz");
        assert_eq!(samplerate_string(1500.0), "1500 Hz");
        assert_eq!(samplerate_string(48.0), "48 Hz");
    }

    /// Golden: byte-for-byte the metadata the Python writer produced for a
    /// demo capture (8 logic + 2 analog channels at 1 MHz).
    #[test]
    fn metadata_matches_python_reference_output() {
        let logic: Vec<String> = (0..8).map(|i| format!("D{i}")).collect();
        let analog = vec!["A0".to_string(), "A1".to_string()];
        let w = SrZipWriter::new(1e6, logic, analog);
        assert_eq!(w.metadata(), "\
[global]
sigrok version = 0.5.2 (openmso)

[device 1]
capturefile = logic-1
total probes = 8
samplerate = 1 MHz
total analog = 2
unitsize = 1
probe1 = D0
probe2 = D1
probe3 = D2
probe4 = D3
probe5 = D4
probe6 = D5
probe7 = D6
probe8 = D7
analog9 = A0
analog10 = A1
");
    }

    #[test]
    fn analog_only_metadata_omits_logic_keys() {
        let w = SrZipWriter::new(1e6, vec![], vec!["C1".to_string()]);
        let m = w.metadata();
        assert!(!m.contains("capturefile"), "{m}");
        assert!(!m.contains("unitsize"), "{m}");
        assert!(!m.contains("total probes"), "{m}");
        // With no logic channels the analog numbering starts at 1.
        assert!(m.contains("analog1 = C1"), "{m}");
    }

    #[test]
    fn unitsize_covers_all_logic_channels() {
        let ch = |n: usize| (0..n).map(|i| format!("D{i}")).collect::<Vec<_>>();
        assert_eq!(SrZipWriter::new(1e6, ch(8), vec![]).unitsize, 1);
        assert_eq!(SrZipWriter::new(1e6, ch(9), vec![]).unitsize, 2);
        assert_eq!(SrZipWriter::new(1e6, ch(16), vec![]).unitsize, 2);
        assert_eq!(SrZipWriter::new(1e6, ch(17), vec![]).unitsize, 3);
    }

    #[test]
    fn written_archive_has_the_expected_entries_and_payloads() {
        let dir = std::env::temp_dir().join(format!("srzip-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.sr");

        let mut w = SrZipWriter::new(1e6, vec!["D0".into()], vec!["A0".into()]);
        w.add_logic(&[0, 1, 0, 1]);
        w.add_analog(0, &[0.0, 1.0, -1.0]);
        w.write(&path).unwrap();

        let mut z = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let names: Vec<String> = z.file_names().map(String::from).collect();
        for want in ["version", "metadata", "logic-1-1", "analog-1-2-1"] {
            assert!(names.contains(&want.to_string()), "missing {want} in {names:?}");
        }

        let mut buf = Vec::new();
        io::Read::read_to_end(&mut z.by_name("version").unwrap(), &mut buf).unwrap();
        assert_eq!(buf, b"2");

        buf.clear();
        io::Read::read_to_end(&mut z.by_name("analog-1-2-1").unwrap(), &mut buf).unwrap();
        assert_eq!(buf.len(), 3 * 4, "three float32 samples");
        assert_eq!(f32::from_le_bytes(buf[4..8].try_into().unwrap()), 1.0);

        std::fs::remove_dir_all(&dir).ok();
    }
}
