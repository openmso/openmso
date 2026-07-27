// SPDX-License-Identifier: Apache-2.0
//! `--set` specs onto the typed `Config` message.
//!
//! OCP v1 configuration is a schema, not a key/value bag, so the CLI has to
//! map what a user types onto named fields. The key names here are the proto
//! field names, which is what `Describe` reports limits against.

use openmso::proto::{ChannelConfig, Config, Coupling, DeviceConfig};

const DEVICE_KEYS: [&str; 3] = ["samplerate", "sample_depth", "averaging"];
const CHANNEL_KEYS: [&str; 9] = [
    "enabled", "probe_factor", "full_scale", "offset", "coupling", "impedance",
    "bandwidth_limit", "invert", "threshold",
];

/// Parse one `[CHANNEL@]KEY=VALUE` spec into `config`.
pub fn apply_spec(config: &mut Config, spec: &str) -> Result<(), String> {
    let (channel, assignment) = match spec.rsplit_once('@') {
        Some((channel, assignment)) if !channel.is_empty() => (Some(channel), assignment),
        _ => (None, spec),
    };
    let (key, value) = assignment
        .split_once('=')
        .ok_or_else(|| format!("--set {spec:?} is not [CHANNEL@]KEY=VALUE"))?;

    match channel {
        Some(channel) => apply_channel(channel_entry(config, channel), key, value),
        None => apply_device(config.device.get_or_insert_with(DeviceConfig::default), key, value),
    }
    .map_err(|e| format!("--set {spec:?}: {e}"))
}

/// Enable exactly the named channels, disabling every other one the device has.
pub fn select_channels(config: &mut Config, wanted: &[&str], available: &[String]) -> Result<(), String> {
    if let Some(unknown) = wanted.iter().find(|c| !available.iter().any(|a| a == *c)) {
        return Err(format!("no channel {unknown:?}; device has {available:?}"));
    }
    for id in available {
        channel_entry(config, id).enabled = Some(wanted.contains(&id.as_str()));
    }
    Ok(())
}

fn channel_entry<'a>(config: &'a mut Config, id: &str) -> &'a mut ChannelConfig {
    if let Some(index) = config.channels.iter().position(|c| c.id == id) {
        return &mut config.channels[index];
    }
    config.channels.push(ChannelConfig { id: id.to_string(), ..Default::default() });
    config.channels.last_mut().expect("just pushed")
}

fn apply_device(device: &mut DeviceConfig, key: &str, value: &str) -> Result<(), String> {
    match key {
        "samplerate" => device.samplerate = Some(number(value)?),
        "sample_depth" => device.sample_depth = Some(count(value)?),
        "averaging" => device.averaging = Some(count(value)? as u32),
        _ => return Err(format!("unknown device setting; try one of {DEVICE_KEYS:?}")),
    }
    Ok(())
}

fn apply_channel(channel: &mut ChannelConfig, key: &str, value: &str) -> Result<(), String> {
    match key {
        "enabled" => channel.enabled = Some(boolean(value)?),
        "probe_factor" => channel.probe_factor = Some(number(value)?),
        "full_scale" => channel.full_scale = Some(number(value)?),
        "offset" => channel.offset = Some(number(value)?),
        "coupling" => channel.coupling = Some(coupling(value)? as i32),
        "impedance" => channel.impedance = Some(number(value)?),
        "bandwidth_limit" => channel.bandwidth_limit = Some(boolean(value)?),
        "invert" => channel.invert = Some(boolean(value)?),
        "threshold" => channel.threshold = Some(number(value)?),
        _ => return Err(format!("unknown channel setting; try one of {CHANNEL_KEYS:?}")),
    }
    Ok(())
}

/// Accepts engineering suffixes, because sample rates and depths are written
/// that way everywhere else: `1M`, `2.5k`, `14M`.
fn number(value: &str) -> Result<f64, String> {
    let (digits, multiplier) = match value.chars().last() {
        Some('k') | Some('K') => (&value[..value.len() - 1], 1e3),
        Some('M') => (&value[..value.len() - 1], 1e6),
        Some('G') => (&value[..value.len() - 1], 1e9),
        _ => (value, 1.0),
    };
    digits
        .parse::<f64>()
        .map(|n| n * multiplier)
        .map_err(|_| format!("{value:?} is not a number"))
}

fn count(value: &str) -> Result<u64, String> {
    let n = number(value)?;
    if !n.is_finite() || n < 0.0 {
        return Err(format!("{value:?} is not a count"));
    }
    Ok(n as u64)
}

fn boolean(value: &str) -> Result<bool, String> {
    match value {
        "true" | "on" | "1" => Ok(true),
        "false" | "off" | "0" => Ok(false),
        _ => Err(format!("{value:?} is not true or false")),
    }
}

fn coupling(value: &str) -> Result<Coupling, String> {
    match value.to_ascii_lowercase().as_str() {
        "dc" => Ok(Coupling::Dc),
        "ac" => Ok(Coupling::Ac),
        "gnd" => Ok(Coupling::Gnd),
        _ => Err(format!("{value:?} is not dc, ac or gnd")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(specs: &[&str]) -> Config {
        let mut config = Config::default();
        for spec in specs {
            apply_spec(&mut config, spec).unwrap();
        }
        config
    }

    #[test]
    fn device_settings_land_in_the_device_message() {
        let c = config(&["samplerate=1M", "sample_depth=14M"]);
        let device = c.device.unwrap();
        assert_eq!(device.samplerate, Some(1e6));
        assert_eq!(device.sample_depth, Some(14_000_000));
        // Untouched fields stay absent, so the plugin leaves them alone.
        assert_eq!(device.averaging, None);
        assert!(c.channels.is_empty());
    }

    #[test]
    fn channel_settings_accumulate_into_one_entry_per_channel() {
        let c = config(&["C1@probe_factor=10", "C1@full_scale=8", "C2@coupling=ac"]);
        assert_eq!(c.channels.len(), 2);
        let c1 = &c.channels[0];
        assert_eq!(c1.id, "C1");
        assert_eq!(c1.probe_factor, Some(10.0));
        assert_eq!(c1.full_scale, Some(8.0));
        assert_eq!(c.channels[1].coupling, Some(Coupling::Ac as i32));
    }

    #[test]
    fn suffixes_and_booleans_parse_the_way_they_are_written() {
        assert_eq!(number("2.5k").unwrap(), 2500.0);
        assert_eq!(number("1G").unwrap(), 1e9);
        assert_eq!(number("48").unwrap(), 48.0);
        assert!(number("1 MHz").is_err());
        assert!(boolean("on").unwrap());
        assert!(!boolean("0").unwrap());
        assert!(boolean("yes").is_err());
    }

    #[test]
    fn a_misspelt_key_names_the_alternatives() {
        let mut c = Config::default();
        let err = apply_spec(&mut c, "sampelrate=1M").unwrap_err();
        assert!(err.contains("samplerate"), "{err}");
        let err = apply_spec(&mut c, "C1@vdiv=0.5").unwrap_err();
        assert!(err.contains("full_scale"), "{err}");
        assert!(apply_spec(&mut c, "samplerate").is_err());
    }

    #[test]
    fn selecting_channels_disables_the_rest() {
        let available: Vec<String> = ["A0", "A1", "A2"].map(String::from).to_vec();
        let mut c = Config::default();
        select_channels(&mut c, &["A1"], &available).unwrap();
        let enabled: Vec<(&str, Option<bool>)> =
            c.channels.iter().map(|c| (c.id.as_str(), c.enabled)).collect();
        assert_eq!(enabled, [("A0", Some(false)), ("A1", Some(true)), ("A2", Some(false))]);

        assert!(select_channels(&mut c, &["C9"], &available).is_err());
    }
}
