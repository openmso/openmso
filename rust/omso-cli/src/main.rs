// SPDX-License-Identifier: Apache-2.0
//! omso-cli — OpenMSO command-line frontend.
//!
//! Launches a capture plugin as a subprocess, speaks OCP to it over stdio, and
//! writes captures as sigrok-compatible .sr files (plus optional CSV).
//!
//! Examples:
//!   omso-cli --plugin demo capture -o demo.sr --csv demo.csv
//!   omso-cli --plugin siglent-sds1000xe --address 192.168.1.155 scan
//!   omso-cli --plugin siglent-sds1000xe --address 192.168.1.155 capture \
//!            --channels C1,C3 --mode single -o cal.sr
//!   omso-cli --plugin siglent-sds1000xe --address 192.168.1.155 raw "SARA?"

mod manifest;
mod sink;
mod srzip;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use openmso::client::CaptureClient;
use serde_json::{json, Value};

use sink::Sink;
use srzip::SrZipWriter;

const RPC_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Parser)]
#[command(name = "omso-cli", version, about = "OpenMSO command-line frontend")]
struct Cli {
    /// Plugin name under plugins/
    #[arg(long, global = true, default_value = "demo")]
    plugin: String,

    /// Network address hint for scan
    #[arg(long, global = true)]
    address: Option<String>,

    /// Substring to select among scan results
    #[arg(long, global = true)]
    device: Option<String>,

    /// Directory holding plugin manifests (overrides $OPENMSO_PLUGINS_DIR)
    #[arg(long, global = true, value_name = "DIR")]
    plugins_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List devices
    Scan,
    /// Show device description and config
    Info,
    /// Send raw device commands (debug)
    Raw {
        #[arg(required = true)]
        commands: Vec<String>,
    },
    /// Perform a capture
    Capture(CaptureArgs),
}

#[derive(clap::Args)]
struct CaptureArgs {
    /// Comma-separated, e.g. C1,C3 (default: leave device selection as-is)
    #[arg(long)]
    channels: Option<String>,

    #[arg(long, default_value = "single", value_parser = ["single", "snapshot"])]
    mode: String,

    /// Trigger wait timeout, seconds
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,

    /// Config to apply, e.g. C1@vdiv=0.5 or memory_depth=14M (repeatable)
    #[arg(long, value_name = "[CH@]KEY=VALUE")]
    set: Vec<String>,

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
    let m = manifest::find_plugin(&dir, &cli.plugin)?;

    let verbose = !matches!(&cli.command, Command::Capture(a) if a.quiet);
    let collector = Sink::new(verbose);
    let handler = {
        let collector = collector.clone();
        Box::new(move |method: &str, params: &Value, payload: Option<&[u8]>| {
            collector.handle(method, params, payload);
        })
    };

    let mut client = CaptureClient::launch(&m.argv, Some(handler))
        .map_err(|e| format!("cannot launch plugin {:?}: {e}", cli.plugin))?;
    client.initialize("omso-cli").map_err(|e| e.to_string())?;

    let result = match &cli.command {
        Command::Scan => cmd_scan(&client, cli),
        Command::Info => cmd_info(&client, cli),
        Command::Raw { commands } => cmd_raw(&client, cli, commands),
        Command::Capture(args) => cmd_capture(&client, cli, args, &collector),
    };
    client.close();
    result
}

fn request(client: &CaptureClient, method: &str, params: Value) -> Result<Value, String> {
    client.request(method, params, RPC_TIMEOUT)
        .map_err(|e| format!("plugin error: {e}"))
}

fn scan_hints(cli: &Cli) -> Value {
    match &cli.address {
        Some(a) => json!({"address": a}),
        None => json!({}),
    }
}

fn pick_device(client: &CaptureClient, cli: &Cli) -> Result<Value, String> {
    let r = request(client, "scan", json!({"hints": scan_hints(cli)}))?;
    let devices = r.get("devices").and_then(Value::as_array).cloned().unwrap_or_default();
    if devices.is_empty() {
        return Err("no devices found".to_string());
    }
    match &cli.device {
        Some(want) => devices.into_iter()
            .find(|d| d.get("device_id").and_then(Value::as_str)
                       .is_some_and(|id| id.contains(want.as_str())))
            .ok_or_else(|| format!("no device matching {want:?}")),
        None => Ok(devices.into_iter().next().expect("non-empty")),
    }
}

fn field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("?")
}

fn cmd_scan(client: &CaptureClient, cli: &Cli) -> Result<(), String> {
    let r = request(client, "scan", json!({"hints": scan_hints(cli)}))?;
    let devices = r.get("devices").and_then(Value::as_array).cloned().unwrap_or_default();
    if devices.is_empty() {
        println!("no devices found");
    }
    for d in &devices {
        println!("{}: {} {} (serial {})", field(d, "device_id"), field(d, "vendor"),
                 field(d, "model"), field(d, "serial"));
    }
    Ok(())
}

fn cmd_info(client: &CaptureClient, cli: &Cli) -> Result<(), String> {
    let dev = pick_device(client, cli)?;
    request(client, "open", json!({"device_id": dev.get("device_id")}))?;
    let desc = request(client, "describe", json!({}))?;
    println!("{}", serde_json::to_string_pretty(&desc).map_err(|e| e.to_string())?);

    let cfg = request(client, "config.get", json!({}))?;
    println!("\n# device config:");
    println!("{}", serde_json::to_string_pretty(cfg.get("values").unwrap_or(&json!({})))
             .map_err(|e| e.to_string())?);

    for ch in analog_channels(&desc) {
        let vals = request(client, "config.get", json!({"channel": ch}))?;
        println!("# {ch}: {}", vals.get("values").unwrap_or(&json!({})));
    }
    Ok(())
}

fn cmd_raw(client: &CaptureClient, cli: &Cli, commands: &[String]) -> Result<(), String> {
    let dev = pick_device(client, cli)?;
    request(client, "open", json!({"device_id": dev.get("device_id")}))?;
    for cmd in commands {
        let r = request(client, "device.raw", json!({"command": cmd}))?;
        if let Some(resp) = r.get("response").and_then(Value::as_str) {
            println!("{resp}");
        }
    }
    Ok(())
}

fn analog_channels(desc: &Value) -> Vec<String> {
    desc.get("channels").and_then(Value::as_array).map(|chs| {
        chs.iter()
            .filter(|c| c.get("kind").and_then(Value::as_str) == Some("analog"))
            .filter_map(|c| c.get("id").and_then(Value::as_str).map(String::from))
            .collect()
    }).unwrap_or_default()
}

fn cmd_capture(client: &CaptureClient, cli: &Cli, args: &CaptureArgs,
               collector: &Sink) -> Result<(), String> {
    let dev = pick_device(client, cli)?;
    request(client, "open", json!({"device_id": dev.get("device_id")}))?;
    if !args.quiet {
        eprintln!("opened {} {} via {}", field(&dev, "vendor"), field(&dev, "model"),
                  field(&dev, "connection"));
    }

    let desc = request(client, "describe", json!({}))?;
    let analog = analog_channels(&desc);
    if let Some(list) = &args.channels {
        let wanted: Vec<&str> = list.split(',').collect();
        let unknown: Vec<&&str> = wanted.iter()
            .filter(|c| !analog.iter().any(|a| a == *c)).collect();
        if !unknown.is_empty() {
            return Err(format!("unknown channels: {unknown:?} (device has {analog:?})"));
        }
        for ch in &analog {
            request(client, "config.set",
                    json!({"channel": ch,
                           "values": {"enabled": wanted.contains(&ch.as_str())}}))?;
        }
    }

    for spec in &args.set {
        let (scope, kv) = match spec.rfind('@') {
            Some(i) => (Some(&spec[..i]), &spec[i + 1..]),
            None => (None, spec.as_str()),
        };
        let (key, raw) = kv.split_once('=')
            .ok_or_else(|| format!("--set {spec:?} is not [CH@]KEY=VALUE"))?;
        // Bare words stay strings; anything JSON-shaped keeps its type.
        let value: Value = serde_json::from_str(raw)
            .unwrap_or_else(|_| Value::String(raw.to_string()));
        let mut params = json!({"values": {key: value}});
        if let Some(ch) = scope.filter(|s| !s.is_empty()) {
            params["channel"] = json!(ch);
        }
        let applied = request(client, "config.set", params)?;
        if !args.quiet {
            eprintln!("set {spec} -> {}", applied.get("applied").unwrap_or(&json!({})));
        }
    }

    let r = request(client, "acquire.start",
                    json!({"mode": args.mode, "timeout": args.timeout}))?;
    if !args.quiet {
        eprintln!("capture {} started ({})",
                  r.get("capture_id").unwrap_or(&json!(null)), args.mode);
    }

    // Generous slack over the trigger timeout: the transfer itself can be slow.
    if !collector.wait(Duration::from_secs_f64(args.timeout + 120.0)) {
        return Err("capture timed out".to_string());
    }
    let state = collector.state();
    let end = state.end.as_ref().expect("wait returned true");
    if !end.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Err(format!("capture failed: {}", field(end, "error")));
    }
    write_outputs(&state, args)
}

struct AnalogStream {
    name: String,
    volts: Vec<f64>,
}

fn write_outputs(state: &sink::State, args: &CaptureArgs) -> Result<(), String> {
    let begin = state.begin.as_ref().ok_or("no capture.begin received")?;
    let samplerate = begin.get("samplerate").and_then(Value::as_f64)
        .ok_or("capture.begin has no samplerate")?;
    let empty = vec![];
    let streams = begin.get("streams").and_then(Value::as_array).unwrap_or(&empty);

    let mut analog: Vec<AnalogStream> = Vec::new();
    let mut logic_channels: Vec<String> = Vec::new();
    let mut logic_blob: Vec<u8> = Vec::new();

    for s in streams {
        let id = s.get("stream").and_then(Value::as_i64).unwrap_or(0);
        let channels: Vec<String> = s.get("channels").and_then(Value::as_array)
            .map(|cs| cs.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        match s.get("kind").and_then(Value::as_str) {
            Some("analog") => {
                let enc = s.get("encoding").cloned().unwrap_or_else(|| json!({}));
                let volts = sink::decode(
                    &state.stream_bytes(id),
                    enc.get("dtype").and_then(Value::as_str).unwrap_or("int8"),
                    enc.get("scale").and_then(Value::as_f64).unwrap_or(1.0),
                    enc.get("offset").and_then(Value::as_f64).unwrap_or(0.0))?;
                let name = channels.first().cloned()
                    .unwrap_or_else(|| format!("stream{id}"));
                analog.push(AnalogStream { name, volts });
            }
            Some("logic") => {
                logic_channels.extend(channels);
                logic_blob.extend_from_slice(&state.stream_bytes(id));
            }
            _ => {}
        }
    }

    let nsamples = analog.iter().map(|a| a.volts.len()).max().unwrap_or(0);
    println!("capture: {} analog channel(s), {nsamples} samples @ {} Sa/s",
             analog.len(), fmt_rate(samplerate));
    for a in &analog {
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

    if let Some(path) = &args.output {
        let mut w = SrZipWriter::new(samplerate, logic_channels,
                                     analog.iter().map(|a| a.name.clone()).collect());
        for (i, a) in analog.iter().enumerate() {
            w.add_analog(i, &a.volts);
        }
        w.add_logic(&logic_blob);
        w.write(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!("wrote {}", path.display());
    }

    if let Some(path) = &args.csv {
        let step = args.csv_decimate.max(1);
        let t0 = begin.get("t0").and_then(Value::as_f64).unwrap_or(0.0);
        write_csv(path, &analog, nsamples, samplerate, t0, step)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!("wrote {}{}", path.display(),
                 if step > 1 { format!(" (1:{step} decimated)") } else { String::new() });
    }
    Ok(())
}

fn write_csv(path: &PathBuf, analog: &[AnalogStream], nsamples: usize,
             samplerate: f64, t0: f64, step: usize) -> std::io::Result<()> {
    let f = std::fs::File::create(path)?;
    let mut w = std::io::BufWriter::new(f);
    write!(w, "time")?;
    for a in analog {
        write!(w, ",{}", a.name)?;
    }
    writeln!(w)?;
    for i in (0..nsamples).step_by(step) {
        write!(w, "{:.12e}", i as f64 / samplerate + t0)?;
        for a in analog {
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

    #[test]
    fn rates_print_without_exponent_notation() {
        assert_eq!(fmt_rate(1e6), "1000000");
        assert_eq!(fmt_rate(1.25e6), "1250000");
        assert_eq!(fmt_rate(2.5), "2.5");
    }

    #[test]
    fn analog_channel_ids_come_from_describe() {
        let desc = json!({"channels": [
            {"id": "A0", "kind": "analog"},
            {"id": "D0", "kind": "logic"},
            {"id": "A1", "kind": "analog"}]});
        assert_eq!(analog_channels(&desc), vec!["A0", "A1"]);
        assert!(analog_channels(&json!({})).is_empty());
    }

    #[test]
    fn cli_accepts_the_documented_invocations() {
        use clap::CommandFactory;
        Cli::command().debug_assert();

        let cli = Cli::try_parse_from(
            ["omso-cli", "--plugin", "demo", "capture", "-o", "x.sr"]).unwrap();
        assert_eq!(cli.plugin, "demo");
        let Command::Capture(a) = &cli.command else { panic!("expected capture") };
        assert_eq!(a.output.as_deref(), Some(std::path::Path::new("x.sr")));
        assert_eq!(a.mode, "single");

        let cli = Cli::try_parse_from(
            ["omso-cli", "scan", "--address", "192.168.1.155"]).unwrap();
        assert_eq!(cli.address.as_deref(), Some("192.168.1.155"));

        assert_eq!(Cli::try_parse_from(["omso-cli", "scan"]).unwrap().plugin, "demo");

        assert!(Cli::try_parse_from(
            ["omso-cli", "capture", "--mode", "continuous"]).is_err());
    }
}
