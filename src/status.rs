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
            let text =
                std::fs::read_to_string(status_dir.join(format!("{name}.json"))).ok()?;
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
