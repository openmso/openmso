// SPDX-License-Identifier: Apache-2.0
//! Resolve a capture plugin name to its launch argv via
//! `plugins/<name>/plugin.json`.

use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Debug)]
pub struct Manifest {
    pub argv: Vec<String>,
}

fn default_python() -> String {
    std::env::var("OPENMSO_PYTHON").unwrap_or_else(|_| "python3".to_string())
}

/// Relative path entries (anything with a separator, or a `*.py` script)
/// resolve against the plugin directory; plain flags pass through.
fn resolve_arg(token: &str, plugin_dir: &Path) -> String {
    if token == "{python}" {
        return default_python();
    }
    let looks_like_path = token.contains('/') || token.ends_with(".py");
    if !looks_like_path {
        return token.to_string();
    }
    let p = Path::new(token);
    if p.is_absolute() {
        return token.to_string();
    }
    plugin_dir.join(p).to_string_lossy().into_owned()
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

pub fn find_plugin(plugins_dir: &Path, name: &str) -> Result<Manifest, String> {
    let plugin_dir = plugins_dir.join(name);
    let manifest_path = plugin_dir.join("plugin.json");
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| {
        format!("cannot read {}: {e}", manifest_path.display())
    })?;
    let manifest: Value = serde_json::from_str(&text)
        .map_err(|e| format!("bad JSON in {}: {e}", manifest_path.display()))?;

    let run = manifest.get("run").and_then(Value::as_array).ok_or_else(|| {
        format!("{} has no \"run\" array", manifest_path.display())
    })?;
    let argv: Vec<String> = run
        .iter()
        .filter_map(Value::as_str)
        .map(|t| resolve_arg(t, &plugin_dir))
        .collect();
    if argv.is_empty() {
        return Err(format!("{} has an empty \"run\" array", manifest_path.display()));
    }

    if argv[0].contains('/') && !Path::new(&argv[0]).exists() {
        let hint = manifest.get("build").and_then(Value::as_str)
            .map(|b| format!(" — build it with: {b}"))
            .unwrap_or_default();
        return Err(format!("plugin {name:?} executable missing: {}{hint}", argv[0]));
    }
    Ok(Manifest { argv })
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

        let m = find_plugin(&root, "demo").unwrap();
        assert_eq!(m.argv, vec![root.join("demo/./demo").to_string_lossy()]);
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
    fn bare_tokens_pass_through_as_path_lookups() {
        // A token with no separator and no .py suffix is a flag or a $PATH
        // lookup, never a file beside the manifest. This is exactly why an
        // installed manifest must say "./demo" and not "demo".
        let dir = Path::new("/plugins/demo");
        assert_eq!(resolve_arg("--listen", dir), "--listen");
        assert_eq!(resolve_arg("demo", dir), "demo");
        assert_eq!(resolve_arg("./demo", dir), "/plugins/demo/./demo");
        assert_eq!(resolve_arg("plugin.py", dir), "/plugins/demo/plugin.py");
        assert_eq!(resolve_arg("/usr/bin/thing", dir), "/usr/bin/thing");
    }

    #[test]
    fn python_token_expands_to_the_interpreter() {
        std::env::set_var("OPENMSO_PYTHON", "/usr/bin/python3.13");
        assert_eq!(resolve_arg("{python}", Path::new("/x")), "/usr/bin/python3.13");
        std::env::remove_var("OPENMSO_PYTHON");
    }
}
