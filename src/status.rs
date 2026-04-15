use serde::{Deserialize, Serialize};
use std::path::Path;

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
    let manifest = match manifest {
        Some(m) => m,
        None => return vec![],
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
    let text = match std::fs::read_to_string(log_path) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    let lines: Vec<_> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|s| s.to_string())
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
    fn tail_log_returns_empty_for_n_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "line1\nline2\n").unwrap();
        let lines = tail_log(&path, 0);
        assert!(lines.is_empty());
    }
}
