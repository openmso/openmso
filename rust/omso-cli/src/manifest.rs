// SPDX-License-Identifier: Apache-2.0
//! Resolve a plugin name to the argv that launches it, via
//! `plugins/<name>/plugin.json`.

use std::path::{Path, PathBuf};

use openmso::manifest;
use openmso::proto::PluginManifest;

#[derive(Debug)]
pub struct Plugin {
    pub manifest: PluginManifest,
    pub argv: Vec<String>,
}

fn python() -> String {
    std::env::var("OPENMSO_PYTHON").unwrap_or_else(|_| "python3".to_string())
}

/// Locate the plugins directory: explicit flag, then `$OPENMSO_PLUGINS_DIR`,
/// then paths relative to the executable — `../plugins` for an installed
/// tarball (`bin/omso-cli`), `../../../plugins` for a dev build under
/// `rust/target/<profile>/`.
pub fn plugins_dir(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(d) = explicit {
        return Ok(d);
    }
    if let Some(d) = std::env::var_os("OPENMSO_PLUGINS_DIR") {
        return Ok(PathBuf::from(d));
    }
    let exe = std::env::current_exe()
        .map_err(|e| format!("cannot locate own executable: {e}"))?;
    let dir = exe.parent().ok_or("executable has no parent directory")?;
    for rel in ["../plugins", "../../../plugins"] {
        let candidate = dir.join(rel);
        if candidate.is_dir() {
            return Ok(std::fs::canonicalize(&candidate).unwrap_or(candidate));
        }
    }
    Err(format!("cannot find a plugins directory near {}; \
                 set OPENMSO_PLUGINS_DIR or pass --plugins-dir", dir.display()))
}

pub fn find_plugin(plugins_dir: &Path, name: &str) -> Result<Plugin, String> {
    let plugin_dir = plugins_dir.join(name);
    let manifest = manifest::load(&plugin_dir)
        .map_err(|e| format!("{}: {e}", plugin_dir.join(manifest::FILENAME).display()))?;

    let argv = manifest::resolve_argv(&manifest, &plugin_dir, &python());
    let program = argv.first().ok_or_else(|| format!("plugin {name:?} has an empty run list"))?;
    if program.contains('/') && !Path::new(program).exists() {
        let hint = match manifest.build.as_str() {
            "" => String::new(),
            build => format!(" — build it with: {build}"),
        };
        return Err(format!("plugin {name:?} executable missing: {program}{hint}"));
    }
    Ok(Plugin { manifest, argv })
}

/// The manifest claims which `--device` URLs the plugin handles, so a typo is
/// worth catching before a process is spawned to reject it.
pub fn check_scheme(manifest: &PluginManifest, device: &str) -> Result<(), String> {
    if manifest.url_schemes.is_empty() {
        return Ok(());
    }
    let scheme = device.split("://").next().unwrap_or_default();
    if manifest.url_schemes.iter().any(|s| s == scheme) {
        return Ok(());
    }
    Err(format!("plugin {:?} handles {:?} URLs, not {scheme:?}",
                manifest.name, manifest.url_schemes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join(format!("omso-manifest-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_plugin(root: &Path, name: &str, body: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plugin.json"), body).unwrap();
    }

    #[test]
    fn relative_binary_resolves_against_the_plugin_dir() {
        let root = scratch("rel");
        write_plugin(&root, "demo", r#"{"name":"demo","run":["./demo"]}"#);
        // The executable must exist, or resolution reports it missing.
        std::fs::write(root.join("demo/demo"), b"#!/bin/true\n").unwrap();

        let p = find_plugin(&root, "demo").unwrap();
        assert_eq!(p.argv, vec![root.join("demo/./demo").to_string_lossy()]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_executable_reports_the_build_hint() {
        let root = scratch("missing");
        write_plugin(&root, "demo",
            r#"{"name":"demo","run":["../../rust/target/release/demo"],
                "build":"cargo build --release"}"#);
        let err = find_plugin(&root, "demo").unwrap_err();
        assert!(err.contains("executable missing"), "{err}");
        assert!(err.contains("cargo build --release"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_manifest_that_is_not_a_manifest_says_so() {
        let root = scratch("bad");
        write_plugin(&root, "demo", r#"{"name":"demo","runn":["./demo"]}"#);
        let err = find_plugin(&root, "demo").unwrap_err();
        assert!(err.contains("plugin.json"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn device_urls_are_matched_against_the_declared_schemes() {
        let manifest = PluginManifest {
            name: "demo".into(),
            url_schemes: vec!["demo".into()],
            ..Default::default()
        };
        assert!(check_scheme(&manifest, "demo://0").is_ok());
        let err = check_scheme(&manifest, "usb://04b4:8613").unwrap_err();
        assert!(err.contains("usb"), "{err}");
        // A manifest that claims nothing is not second-guessed.
        assert!(check_scheme(&PluginManifest::default(), "usb://x").is_ok());
    }
}
