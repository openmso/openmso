// SPDX-License-Identifier: Apache-2.0
//! Locate the system-installed fx2lafw firmware blob at runtime.
//!
//! The fx2lafw firmware is GPL-2.0+; this plugin reads the user's
//! system-installed blob at runtime (like the kernel loading a firmware file)
//! and does NOT vendor or redistribute the binary. See `docs/fx2-plan/README.md`
//! §3 for the licensing rationale.

use std::path::{Path, PathBuf};

/// Default basename for the Saleae Logic clone. Other FX2 boards may need a
/// different blob — extend `locate_for` as more boards are added.
const SALEAE_LOGIC_FW: &str = "fx2lafw-saleae-logic.fw";

/// System firmware directories searched in order.
const SEARCH_DIRS: &[&str] = &[
    "/usr/share/sigrok-firmware",
    "/usr/local/share/sigrok-firmware",
];

/// Locate the firmware blob for the Saleae Logic (0925:3881) clone.
///
/// Search order:
/// 1. `$OPENMSO_FX2_FIRMWARE` — full path to any blob (escape hatch).
/// 2. `<search dir>/fx2lafw-saleae-logic.fw` for each system firmware dir.
///
/// Returns the resolved path and the raw bytes, or a clear error with an
/// install hint (`apt install sigrok-firmware-fx2lafw` on Debian/Ubuntu).
pub fn locate() -> Result<(PathBuf, Vec<u8>), String> {
    locate_for(SALEAE_LOGIC_FW)
}

fn locate_for(basename: &str) -> Result<(PathBuf, Vec<u8>), String> {
    if let Ok(p) = std::env::var("OPENMSO_FX2_FIRMWARE") {
        let path = PathBuf::from(&p);
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("OPENMSO_FX2_FIRMWARE={p}: read failed: {e}"))?;
        return Ok((path, bytes));
    }
    for dir in SEARCH_DIRS {
        let path = Path::new(dir).join(basename);
        if let Ok(bytes) = std::fs::read(&path) {
            return Ok((path, bytes));
        }
    }
    Err(format!(
        "fx2lafw firmware blob `{basename}` not found in {SEARCH_DIRS:?}. \
         Install it (e.g. `apt install sigrok-firmware-fx2lafw`) or set \
         OPENMSO_FX2_FIRMWARE to the full path of the blob."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saleae_blob_present_on_bench() {
        // Bench machine has sigrok-firmware-fx2lafw installed; tests run here.
        if std::env::var_os("OPENMSO_FX2_FIRMWARE_SKIP_BENCH").is_some() {
            return;
        }
        let (path, bytes) = locate().expect("fx2lafw-saleae-logic.fw should be installed");
        assert!(bytes.len() > 100, "blob too small: {} bytes", bytes.len());
        // Raw binary loaded at 0x0000 — first bytes should be an 8051 LJMP
        // (opcode 0x02) at the reset vector.
        assert_eq!(bytes[0], 0x02, "expected LJMP reset vector at offset 0: {}",
                   path.display());
    }
}
