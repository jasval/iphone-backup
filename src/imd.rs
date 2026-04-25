//! Typed wrapper around the libimobiledevice CLI tools.
//!
//! Centralises three things that used to be scattered across `backup.rs`,
//! `device.rs`, and `restore.rs`:
//!
//! 1. **Locale-pinned invocation.** All tools are run with `LC_ALL=C` and
//!    `LANG=C` so progress lines and byte counts use the formats our parsers
//!    expect, regardless of the user's shell locale.
//! 2. **Typed errors.** Callers get [`ImdError`] instead of a generic
//!    `io::Error`, making it easier to distinguish a missing binary from a
//!    protocol-level failure.
//! 3. **Shared parsers.** `parse_bytes_progress`, `parse_human_bytes`,
//!    `parse_ioreg_*`, and UDID normalization live here.
//!
//! The streaming spawn path (reading live progress from `idevicebackup2
//! backup`/`restore`) is intentionally *not* wrapped — it's owned by
//! `backup.rs` where the ratatui-facing log channel lives.

use std::collections::HashMap;
use std::process::{Command, Output};

#[derive(Debug)]
pub enum ImdError {
    /// The underlying binary is not on PATH. Usually means
    /// `brew install libimobiledevice` was never run.
    NotFound(String),
    /// The command exited non-zero. `stderr` is the trimmed error output.
    CommandFailed { program: String, code: Option<i32>, stderr: String },
    /// Output was not valid UTF-8.
    NonUtf8(String),
}

impl std::fmt::Display for ImdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(p) => write!(
                f,
                "{p} not found on PATH — install with `brew install libimobiledevice`"
            ),
            Self::CommandFailed { program, code, stderr } => match code {
                Some(c) => write!(f, "{program} exited with {c}: {stderr}"),
                None => write!(f, "{program} terminated without exit code: {stderr}"),
            },
            Self::NonUtf8(p) => write!(f, "{p} produced non-UTF-8 output"),
        }
    }
}

impl std::error::Error for ImdError {}

/// Run a libimobiledevice CLI tool with `LC_ALL=C` / `LANG=C` forced, capture
/// stdout/stderr, and surface typed errors.
fn run(program: &str, args: &[&str]) -> Result<Output, ImdError> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ImdError::NotFound(program.to_string())
            } else {
                ImdError::CommandFailed {
                    program: program.to_string(),
                    code: None,
                    stderr: e.to_string(),
                }
            }
        })?;
    if !out.status.success() {
        return Err(ImdError::CommandFailed {
            program: program.to_string(),
            code: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(out)
}

fn stdout_string(program: &str, out: Output) -> Result<String, ImdError> {
    String::from_utf8(out.stdout).map_err(|_| ImdError::NonUtf8(program.to_string()))
}

/// Return the list of UDIDs for USB-attached devices.
pub fn list_usb() -> Result<Vec<String>, ImdError> {
    let out = run("idevice_id", &["--list"])?;
    Ok(parse_udid_list(&stdout_string("idevice_id", out)?))
}

/// Return the list of UDIDs reachable via Wi-Fi/Tailscale.
pub fn list_network() -> Result<Vec<String>, ImdError> {
    let out = run("idevice_id", &["--network", "--list"])?;
    Ok(parse_udid_list(&stdout_string("idevice_id", out)?))
}

/// Fetch all device properties in one `ideviceinfo` call.
pub fn device_info(udid: &str) -> Result<HashMap<String, String>, ImdError> {
    let udid = normalize_udid(udid);
    let out = run("ideviceinfo", &["--udid", &udid])?;
    let text = stdout_string("ideviceinfo", out)?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ": ");
            let key = parts.next()?.trim().to_string();
            let val = parts.next()?.trim().to_string();
            (!key.is_empty() && !val.is_empty()).then_some((key, val))
        })
        .collect())
}

/// Return the version string printed by `idevicebackup2 --version`. Used to
/// capture the tool version in the per-run summary for later debugging.
pub fn idevicebackup2_version() -> Result<String, ImdError> {
    // `idevicebackup2 --version` prints a line like
    // `idevicebackup2 1.3.0`. We just return the whole trimmed line.
    let out = run("idevicebackup2", &["--version"])?;
    let s = stdout_string("idevicebackup2", out)?;
    Ok(s.trim().to_string())
}

// ── Parsers ──────────────────────────────────────────────────────────────

/// Split an `idevice_id --list` output into clean UDIDs, dropping any
/// trailing " (Network)" decoration some versions append.
pub fn parse_udid_list(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| strip_udid_suffix(l.trim()))
        .filter(|l| !l.is_empty())
        .map(|u| normalize_udid(&u))
        .collect()
}

/// Strip trailing decoration (`" (Network)"`, etc.) from a UDID line.
pub fn strip_udid_suffix(s: &str) -> String {
    s.split_whitespace().next().unwrap_or(s).to_string()
}

/// Normalize modern iPhone UDIDs to a single canonical form. libimobiledevice
/// issue #1204: USB serial and lockdownd report UDIDs with and without the
/// internal dash (`00008030-001C293C1E01402E` vs `00008030001C293C1E01402E`).
/// Dash-stripping keeps lookups consistent across sources.
pub fn normalize_udid(udid: &str) -> String {
    let s = udid.trim();
    // Only strip a dash that appears to be the structural separator at offset
    // 8 (position after the hex vendor field). Leaves legacy 40-char UDIDs
    // (which never contain dashes) untouched.
    if s.len() == 25 && s.as_bytes().get(8) == Some(&b'-') {
        let (a, b) = s.split_at(8);
        let b = &b[1..];
        format!("{a}{b}")
    } else {
        s.to_string()
    }
}

/// Parse bytes transferred from an idevicebackup2 progress line.
/// Handles patterns like "(500.0 MB of 1.2 GB)" or "(512000 of 1200000000)".
pub fn parse_bytes_progress(line: &str) -> Option<(u64, u64)> {
    let start = line.find('(')?;
    let end = line[start..].find(')')? + start;
    let inner = line[start + 1..end].trim();
    let (lhs, rhs) = inner.split_once(" of ")?;
    let cur = parse_human_bytes(lhs.trim())?;
    let tot = parse_human_bytes(rhs.trim())?;
    (tot > 0).then_some((cur, tot))
}

pub fn parse_human_bytes(s: &str) -> Option<u64> {
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }
    let mut parts = s.splitn(2, ' ');
    let num: f64 = parts.next()?.parse().ok()?;
    let unit = parts.next()?.trim().to_uppercase();
    let multiplier: u64 = match unit.as_str() {
        "B" => 1,
        "KB" => 1_000,
        "MB" => 1_000_000,
        "GB" => 1_000_000_000,
        "TB" => 1_000_000_000_000,
        "KIB" => 1_024,
        "MIB" => 1_024 * 1_024,
        "GIB" => 1_024 * 1_024 * 1_024,
        _ => return None,
    };
    Some((num * multiplier as f64).round() as u64)
}

/// Extract a string value from an `ioreg` line of the form `"key" = "value"`.
pub fn parse_ioreg_str(line: &str, key: &str) -> Option<String> {
    let prefix = format!("\"{key}\" = \"");
    let s = line.strip_prefix(&prefix)?;
    let val = s.strip_suffix('"')?;
    Some(val.to_string())
}

/// Extract an integer value from an `ioreg` line of the form `"key" = 1234`.
pub fn parse_ioreg_int(line: &str, key: &str) -> Option<u32> {
    let prefix = format!("\"{key}\" = ");
    let s = line.strip_prefix(&prefix)?;
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udid_list_strips_network_suffix() {
        let udids = parse_udid_list("abc123 (Network)\ndef456\n\n");
        assert_eq!(udids, vec!["abc123".to_string(), "def456".to_string()]);
    }

    #[test]
    fn normalize_strips_modern_dash() {
        assert_eq!(
            normalize_udid("00008030-001C293C1E01402E"),
            "00008030001C293C1E01402E"
        );
    }

    #[test]
    fn normalize_leaves_legacy_udid_alone() {
        let legacy = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0";
        assert_eq!(normalize_udid(legacy), legacy);
    }

    #[test]
    fn normalize_leaves_shortened_udid_alone() {
        assert_eq!(normalize_udid("tooShort"), "tooShort");
    }

    #[test]
    fn parse_human_bytes_integer() {
        assert_eq!(parse_human_bytes("512000"), Some(512_000));
    }

    #[test]
    fn parse_human_bytes_mb() {
        assert_eq!(parse_human_bytes("500.0 MB"), Some(500_000_000));
    }

    #[test]
    fn parse_human_bytes_gib() {
        assert_eq!(parse_human_bytes("1 GiB"), Some(1_073_741_824));
    }

    #[test]
    fn parse_human_bytes_rejects_unknown_unit() {
        assert_eq!(parse_human_bytes("5 FURLONGS"), None);
    }

    #[test]
    fn parse_bytes_progress_mb() {
        let (cur, tot) = parse_bytes_progress("Progress (500.0 MB of 1.2 GB)").unwrap();
        assert_eq!(cur, 500_000_000);
        assert_eq!(tot, 1_200_000_000);
    }

    #[test]
    fn parse_bytes_progress_rejects_total_zero() {
        assert!(parse_bytes_progress("(0 of 0)").is_none());
    }

    #[test]
    fn parse_bytes_progress_no_parens() {
        assert!(parse_bytes_progress("plain line").is_none());
    }

    #[test]
    fn parse_ioreg_str_extracts_value() {
        let line = "    \"kUSBProductString\" = \"iPhone\"";
        assert_eq!(
            parse_ioreg_str(line.trim(), "kUSBProductString"),
            Some("iPhone".to_string())
        );
    }

    #[test]
    fn parse_ioreg_int_extracts_value() {
        let line = "    \"idVendor\" = 1452";
        assert_eq!(parse_ioreg_int(line.trim(), "idVendor"), Some(1452));
    }

    #[test]
    fn parse_ioreg_wrong_key_returns_none() {
        let line = "    \"idVendor\" = 1452";
        assert_eq!(parse_ioreg_int(line.trim(), "idProduct"), None);
    }

    #[test]
    fn imd_error_display_formats_sensibly() {
        let e = ImdError::NotFound("idevice_id".into());
        assert!(e.to_string().contains("idevice_id"));
        assert!(e.to_string().contains("brew install"));

        let e = ImdError::CommandFailed {
            program: "idevicebackup2".into(),
            code: Some(255),
            stderr: "device locked".into(),
        };
        assert!(e.to_string().contains("255"));
        assert!(e.to_string().contains("device locked"));
    }
}
