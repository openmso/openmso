// SPDX-License-Identifier: Apache-2.0
//! omso-cli — OpenMSO command-line frontend, and the reference OCP v1 client.
//!
//! Launches a capture plugin as a subprocess, speaks OCP over the two nng
//! sockets it created for it, and writes captures as sigrok-compatible .sr
//! files (plus optional CSV).
//!
//! Examples:
//!   omso-cli --plugin demo --device demo://0 capture -o demo.sr --csv demo.csv
//!   omso-cli --plugin demo --device demo://0 info
//!   omso-cli --plugin siglent-sds1000xe --device tcp://192.168.1.155:5025 \
//!            capture --channels C1,C3 --set C1@full_scale=8 -o cal.sr

mod capture;
mod config;
mod manifest;
mod srzip;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use openmso::client::CaptureClient;
use openmso::proto::{AcquireMode, ChannelKind, Config, Description};

use capture::Capture;
use srzip::SrZipWriter;

#[derive(Parser)]
#[command(name = "omso-cli", version, about = "OpenMSO command-line frontend")]
struct Cli {
    /// Plugin name under plugins/
    #[arg(long, global = true, default_value = "demo")]
    plugin: String,

    /// Device URL, e.g. demo://0, usb://04b4:8613, tcp://192.168.1.155:5025
    #[arg(long, global = true, default_value = "demo://0")]
    device: String,

    /// Directory holding plugin manifests (overrides $OPENMSO_PLUGINS_DIR)
    #[arg(long, global = true, value_name = "DIR")]
    plugins_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show what the plugin found, and what it will accept
    Info,
    /// Perform a capture
    Capture(CaptureArgs),
}

#[derive(clap::Args)]
struct CaptureArgs {
    /// Comma-separated, e.g. C1,C3 (default: leave device selection as-is)
    #[arg(long)]
    channels: Option<String>,

    #[arg(long, default_value = "single", value_parser = ["single", "snapshot", "continuous"])]
    mode: String,

    /// Config to apply, e.g. C1@full_scale=8 or sample_depth=14M (repeatable)
    #[arg(long, value_name = "[CHANNEL@]KEY=VALUE")]
    set: Vec<String>,

    /// Plugin-specific option from `info` (repeatable)
    #[arg(long, value_name = "KEY=VALUE")]
    vendor: Vec<String>,

    /// Write sigrok .sr file
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Write CSV file
    #[arg(long)]
    csv: Option<PathBuf>,

    /// Write every Nth sample to CSV
    #[arg(long, default_value_t = 1)]
    csv_decimate: usize,

    #[arg(short, long)]
    quiet: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("omso-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), String> {
    let dir = manifest::plugins_dir(cli.plugins_dir.clone())?;
    let plugin = manifest::find_plugin(&dir, &cli.plugin)?;
    manifest::check_scheme(&plugin.manifest, &cli.device)?;

    let mut client = CaptureClient::launch(&plugin.argv, &cli.device)
        .map_err(|e| format!("cannot launch plugin {:?}: {e}", cli.plugin))?;
    let hello = client
        .hello("omso-cli", env!("CARGO_PKG_VERSION"))
        .map_err(|e| format!("handshake failed: {e}"))?;

    let result = match &cli.command {
        Command::Info => cmd_info(&mut client, &hello),
        Command::Capture(args) => cmd_capture(&mut client, args),
    };
    // A plugin that is already wedged should not stop us reporting why.
    client.shutdown().ok();
    result
}

fn json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| e.to_string())
}

fn cmd_info(client: &mut CaptureClient, hello: &openmso::proto::HelloResult) -> Result<(), String> {
    println!("{}", json(hello)?);
    println!("{}", json(&client.describe().map_err(|e| e.to_string())?)?);
    println!("{}", json(&client.get_config().map_err(|e| e.to_string())?)?);
    Ok(())
}

fn channels_of(description: &Description, kind: ChannelKind) -> Vec<String> {
    description
        .channels
        .iter()
        .filter(|c| c.kind == kind as i32)
        .map(|c| c.id.clone())
        .collect()
}

fn cmd_capture(client: &mut CaptureClient, args: &CaptureArgs) -> Result<(), String> {
    let description = client.describe().map_err(|e| e.to_string())?;

    let mut config = Config::default();
    if let Some(list) = &args.channels {
        let wanted: Vec<&str> = list.split(',').collect();
        config::select_channels(&mut config, &wanted,
                                &channels_of(&description, ChannelKind::ChannelAnalog))?;
    }
    for spec in &args.set {
        config::apply_spec(&mut config, spec)?;
    }
    for spec in &args.vendor {
        let (key, value) = spec
            .split_once('=')
            .ok_or_else(|| format!("--vendor {spec:?} is not KEY=VALUE"))?;
        config.vendor.insert(key.to_string(), value.to_string());
    }
    if config != Config::default() {
        let settled = client.set_config(config).map_err(|e| e.to_string())?;
        if !args.quiet {
            eprintln!("configured: {}", json(&settled)?);
        }
    }

    let mode = match args.mode.as_str() {
        "snapshot" => AcquireMode::AcquireSnapshot,
        "continuous" => AcquireMode::AcquireContinuous,
        _ => AcquireMode::AcquireSingle,
    };
    let capture_id = client.next_capture_id();
    client.acquire_start(capture_id, mode).map_err(|e| e.to_string())?;
    if !args.quiet {
        eprintln!("capture {capture_id} started ({})", args.mode);
    }

    let capture = capture::collect(client, capture_id, !args.quiet)?;
    report(&capture);
    write_outputs(&capture, args)
}

fn report(capture: &Capture) {
    let samples = capture.analog.iter().map(|a| a.volts.len()).max().unwrap_or(0);
    println!("capture: {} analog channel(s), {samples} samples @ {} Sa/s",
             capture.analog.len(), fmt_rate(capture.samplerate));
    if let Some(sample) = capture.trigger_sample {
        let t = sample as f64 / capture.samplerate + capture.t0;
        println!("  trigger at sample {sample} (t = {t:+.6} s)");
    }
    if capture.dropped_samples > 0 {
        println!("  warning: {} samples dropped by the device", capture.dropped_samples);
    }
    for a in &capture.analog {
        if a.volts.is_empty() {
            println!("  {}: no samples", a.name);
            continue;
        }
        let (min, max) = a.volts.iter().fold((f64::INFINITY, f64::NEG_INFINITY),
                                             |(lo, hi), &v| (lo.min(v), hi.max(v)));
        let mean = a.volts.iter().sum::<f64>() / a.volts.len() as f64;
        println!("  {}: min {min:+.4} V  max {max:+.4} V  mean {mean:+.4} V  \
                  pkpk {:.4} V", a.name, max - min);
    }
}

fn write_outputs(capture: &Capture, args: &CaptureArgs) -> Result<(), String> {
    if let Some(path) = &args.output {
        let mut w = SrZipWriter::new(capture.samplerate, capture.logic_channels.clone(),
                                     capture.analog.iter().map(|a| a.name.clone()).collect());
        for (i, a) in capture.analog.iter().enumerate() {
            w.add_analog(i, &a.volts);
        }
        w.add_logic(&capture.logic);
        w.write(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!("wrote {}", path.display());
    }

    if let Some(path) = &args.csv {
        let step = args.csv_decimate.max(1);
        write_csv(path, capture, step)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!("wrote {}{}", path.display(),
                 if step > 1 { format!(" (1:{step} decimated)") } else { String::new() });
    }
    Ok(())
}

fn write_csv(path: &Path, capture: &Capture, step: usize) -> std::io::Result<()> {
    let samples = capture.analog.iter().map(|a| a.volts.len()).max().unwrap_or(0);
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(w, "time")?;
    for a in &capture.analog {
        write!(w, ",{}", a.name)?;
    }
    writeln!(w)?;
    for i in (0..samples).step_by(step) {
        write!(w, "{:.12e}", i as f64 / capture.samplerate + capture.t0)?;
        for a in &capture.analog {
            match a.volts.get(i) {
                Some(v) => write!(w, ",{v:.12e}")?,
                None => write!(w, ",")?,
            }
        }
        writeln!(w)?;
    }
    w.flush()
}

/// Whole rates print as integers ("1000000 Sa/s"), which reads better in a
/// terminal than the reference implementation's `%.4g` ("1e+06").
fn fmt_rate(sr: f64) -> String {
    if sr.fract() == 0.0 && sr.abs() < 1e15 {
        format!("{}", sr as i64)
    } else {
        format!("{sr}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmso::proto::Channel;

    #[test]
    fn rates_print_without_exponent_notation() {
        assert_eq!(fmt_rate(1e6), "1000000");
        assert_eq!(fmt_rate(1.25e6), "1250000");
        assert_eq!(fmt_rate(2.5), "2.5");
    }

    #[test]
    fn analog_channel_ids_come_from_describe() {
        let channel = |id: &str, kind: ChannelKind| Channel {
            id: id.to_string(),
            kind: kind as i32,
            ..Default::default()
        };
        let description = Description {
            channels: vec![
                channel("A0", ChannelKind::ChannelAnalog),
                channel("D0", ChannelKind::ChannelLogic),
                channel("A1", ChannelKind::ChannelAnalog),
            ],
            ..Default::default()
        };
        assert_eq!(channels_of(&description, ChannelKind::ChannelAnalog), ["A0", "A1"]);
        assert_eq!(channels_of(&description, ChannelKind::ChannelLogic), ["D0"]);
        assert!(channels_of(&Description::default(), ChannelKind::ChannelAnalog).is_empty());
    }

    #[test]
    fn cli_accepts_the_documented_invocations() {
        use clap::CommandFactory;
        Cli::command().debug_assert();

        let cli = Cli::try_parse_from(
            ["omso-cli", "--plugin", "demo", "--device", "demo://0", "capture", "-o", "x.sr"])
            .unwrap();
        assert_eq!(cli.plugin, "demo");
        assert_eq!(cli.device, "demo://0");
        let Command::Capture(a) = &cli.command else { panic!("expected capture") };
        assert_eq!(a.output.as_deref(), Some(Path::new("x.sr")));
        assert_eq!(a.mode, "single");

        let cli = Cli::try_parse_from(["omso-cli", "info"]).unwrap();
        assert_eq!(cli.plugin, "demo");

        assert!(Cli::try_parse_from(["omso-cli", "capture", "--mode", "sometimes"]).is_err());
        // scan and raw are gone: the frontend enumerates, and device-native
        // command passthrough left the protocol.
        assert!(Cli::try_parse_from(["omso-cli", "scan"]).is_err());
        assert!(Cli::try_parse_from(["omso-cli", "raw", "SARA?"]).is_err());
    }
}
