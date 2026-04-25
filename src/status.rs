use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use crate::verify::VerificationReport;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub name: String,
    pub udid: String,
    pub model: Option<String>,
    pub ios: Option<String>,
    pub status: String, // "success" | "failed" | "no_devices"
    pub last_run: String,
    pub size: Option<String>,
    pub elapsed_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<VerificationReport>,
}

/// Atomically write `bytes` to `path`: write to a sibling `.tmp` file, fsync,
/// rename. A crash mid-write leaves the destination intact.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(format!("{}.tmp", file_name.to_string_lossy()));

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub last_run: String,
    pub total_devices: u64,
    pub failed: u64,
    pub status: String, // "ok" | "partial_failure" | "no_devices"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    devices: Vec<String>,
}

pub fn load_summary(status_dir: &Path) -> Option<Summary> {
    let text = std::fs::read_to_string(status_dir.join("summary.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn load_devices(status_dir: &Path) -> Vec<DeviceStatus> {
    let text = std::fs::read_to_string(status_dir.join("manifest.json")).ok();
    let manifest: Option<Manifest> = text.and_then(|t| serde_json::from_str(&t).ok());
    let Some(manifest) = manifest else {
        return vec![];
    };
    manifest
        .devices
        .iter()
        .filter_map(|name| {
            let text = std::fs::read_to_string(status_dir.join(format!("{name}.json"))).ok()?;
            serde_json::from_str(&text).ok()
        })
        .collect()
}

/// Return the last `n` lines of the log file, oldest first.
pub fn tail_log(log_path: &Path, n: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(log_path) else {
        return vec![];
    };
    let lines: Vec<_> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(std::string::ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_json(dir: &Path, name: &str, data: &serde_json::Value) {
        std::fs::write(dir.join(name), serde_json::to_string(data).unwrap()).unwrap();
    }

    #[test]
    fn load_summary_parses_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        write_json(
            dir.path(),
            "summary.json",
            &serde_json::json!({
                "last_run": "2025-01-01T00:00:00Z",
                "total_devices": 2,
                "failed": 0,
                "status": "ok"
            }),
        );
        let s = load_summary(dir.path()).unwrap();
        assert_eq!(s.total_devices, 2);
        assert_eq!(s.failed, 0);
        assert_eq!(s.status, "ok");
    }

    #[test]
    fn load_summary_returns_none_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_summary(dir.path()).is_none());
    }

    #[test]
    fn load_summary_returns_none_for_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("summary.json"), "not json").unwrap();
        assert!(load_summary(dir.path()).is_none());
    }

    #[test]
    fn load_devices_returns_empty_for_no_manifest() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_devices(dir.path()).is_empty());
    }

    #[test]
    fn load_devices_parses_manifest_and_device_files() {
        let dir = tempfile::tempdir().unwrap();
        write_json(
            dir.path(),
            "manifest.json",
            &serde_json::json!({ "devices": ["Phone", "iPad"] }),
        );
        write_json(
            dir.path(),
            "Phone.json",
            &serde_json::json!({
                "name": "Phone",
                "udid": "abc123",
                "model": "iPhone15,2",
                "ios": "18.1",
                "status": "success",
                "last_run": "2025-01-01T00:00:00Z",
                "size": "10G",
                "elapsed_sec": 120
            }),
        );
        write_json(
            dir.path(),
            "iPad.json",
            &serde_json::json!({
                "name": "iPad",
                "udid": "def456",
                "status": "failed",
                "last_run": "2025-01-01T00:00:00Z",
            }),
        );
        let devices = load_devices(dir.path());
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "Phone");
        assert_eq!(devices[0].ios.as_deref(), Some("18.1"));
        assert_eq!(devices[1].name, "iPad");
        assert!(devices[1].ios.is_none());
    }

    #[test]
    fn load_devices_skips_missing_device_files() {
        let dir = tempfile::tempdir().unwrap();
        write_json(
            dir.path(),
            "manifest.json",
            &serde_json::json!({ "devices": ["Phone", "Missing"] }),
        );
        write_json(
            dir.path(),
            "Phone.json",
            &serde_json::json!({
                "name": "Phone", "udid": "a", "status": "success", "last_run": ""
            }),
        );
        let devices = load_devices(dir.path());
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Phone");
    }

    #[test]
    fn load_devices_skips_corrupt_device_files() {
        let dir = tempfile::tempdir().unwrap();
        write_json(
            dir.path(),
            "manifest.json",
            &serde_json::json!({ "devices": ["Phone"] }),
        );
        std::fs::write(dir.path().join("Phone.json"), "{bad json").unwrap();
        let devices = load_devices(dir.path());
        assert!(devices.is_empty());
    }

    #[test]
    fn tail_log_returns_last_n_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "line1\nline2\nline3\nline4\nline5\n").unwrap();
        let lines = tail_log(&path, 3);
        assert_eq!(lines, vec!["line3", "line4", "line5"]);
    }

    #[test]
    fn tail_log_returns_all_when_fewer_than_n() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "line1\nline2\n").unwrap();
        let lines = tail_log(&path, 10);
        assert_eq!(lines, vec!["line1", "line2"]);
    }

    #[test]
    fn tail_log_returns_empty_for_missing_file() {
        let lines = tail_log(std::path::Path::new("/nonexistent/log.txt"), 5);
        assert!(lines.is_empty());
    }

    #[test]
    fn atomic_write_creates_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        atomic_write(&path, b"hello").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    fn atomic_write_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        std::fs::write(&path, b"old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn atomic_write_leaves_original_intact_when_tmp_is_orphaned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        std::fs::write(&path, b"original").unwrap();
        // Simulate a crash mid-write: a stray `.tmp` exists but the real file
        // is untouched.
        std::fs::write(dir.path().join("out.json.tmp"), b"partial").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        // A subsequent successful atomic_write still replaces the destination.
        atomic_write(&path, b"fresh").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"fresh");
    }

    #[test]
    fn atomic_write_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deeper/out.json");
        atomic_write(&path, b"x").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
    }

    #[test]
    fn tail_log_returns_empty_for_n_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "line1\nline2\n").unwrap();
        let lines = tail_log(&path, 0);
        assert!(lines.is_empty());
    }
}
