//! Post-backup verification.
//!
//! Run after `idevicebackup2 backup` reports success. Confirms the on-disk
//! shape of the backup is sane and records a summary for the TUI. Intentionally
//! conservative: verification failures produce a warning, not a hard failure,
//! because a half-broken backup is still more useful than no record at all.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationReport {
    pub manifest_ok: bool,
    pub info_plist_ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_backup_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

const MIN_MANIFEST_BYTES: u64 = 4 * 1024;
const FILE_COUNT_REGRESSION_THRESHOLD: f64 = 0.20;

/// Run all verification checks on `backup_dir` (the per-device directory
/// idevicebackup2 just populated). `previous_file_count`, when set, enables
/// the regression check.
pub fn verify_backup(backup_dir: &Path, previous_file_count: Option<u64>) -> VerificationReport {
    let mut report = VerificationReport::default();

    // Manifest.db presence + size. A manifest smaller than 4 KiB is almost
    // certainly truncated — idevicebackup2 always writes at least a header
    // table.
    let manifest = backup_dir.join("Manifest.db");
    report.manifest_ok = std::fs::metadata(&manifest)
        .map(|m| m.len() >= MIN_MANIFEST_BYTES)
        .unwrap_or(false);

    // Info.plist: parse via `plutil -convert json -o - <path>`. Skip quietly
    // when plutil isn't on PATH — that's the only libimobiledevice-adjacent
    // tool not guaranteed to be installed.
    let info = backup_dir.join("Info.plist");
    if info.exists() {
        if let Some(json) = plutil_to_json(&info) {
            report.info_plist_ok = true;
            report.device_name = json.get("Device Name").and_then(|v| v.as_str()).map(str::to_string);
            report.product_version = json.get("Product Version").and_then(|v| v.as_str()).map(str::to_string);
            report.last_backup_date = json
                .get("Last Backup Date")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
    }

    // File count + regression check.
    let count = count_files(backup_dir);
    report.file_count = Some(count);
    if let Some(prev) = previous_file_count {
        if prev > 0 {
            let drop = prev.saturating_sub(count);
            if (drop as f64) / (prev as f64) > FILE_COUNT_REGRESSION_THRESHOLD {
                report.warning = Some(format!(
                    "file count dropped from {prev} to {count} ({:.0}% loss)",
                    (drop as f64) / (prev as f64) * 100.0
                ));
            }
        }
    }

    if !report.manifest_ok {
        report.warning.get_or_insert_with(|| {
            format!(
                "Manifest.db missing or smaller than {} bytes",
                MIN_MANIFEST_BYTES
            )
        });
    }

    report
}

/// Convert an Info.plist to a flat JSON map via `plutil`. Returns None if
/// plutil isn't available or the plist is unreadable.
fn plutil_to_json(path: &Path) -> Option<serde_json::Value> {
    let out = Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Count the regular files under `dir`. Walks the tree directly — faster and
/// more predictable than shelling out to `find`.
fn count_files(dir: &Path) -> u64 {
    fn walk(p: &Path, acc: &mut u64) {
        let entries = match std::fs::read_dir(p) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_dir() {
                walk(&entry.path(), acc);
            } else if ft.is_file() {
                *acc += 1;
            }
        }
    }
    let mut acc = 0u64;
    walk(dir, &mut acc);
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_too_small_marks_not_ok() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Manifest.db"), b"tiny").unwrap();
        let r = verify_backup(dir.path(), None);
        assert!(!r.manifest_ok);
        assert!(r.warning.as_deref().unwrap().contains("Manifest.db"));
    }

    #[test]
    fn large_manifest_passes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Manifest.db"), vec![0u8; 8 * 1024]).unwrap();
        let r = verify_backup(dir.path(), None);
        assert!(r.manifest_ok);
    }

    #[test]
    fn counts_files_recursively() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/b"), b"").unwrap();
        std::fs::write(dir.path().join("sub/c"), b"").unwrap();
        let r = verify_backup(dir.path(), None);
        assert_eq!(r.file_count, Some(3));
    }

    #[test]
    fn regression_warning_fires_on_large_drop() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Manifest.db"), vec![0u8; 8 * 1024]).unwrap();
        // Count: Manifest.db + "a" = 2 files; 2/10 = 80% loss > 20% threshold.
        std::fs::write(dir.path().join("a"), b"").unwrap();
        let r = verify_backup(dir.path(), Some(10));
        assert!(r.warning.as_deref().unwrap().contains("80%"), "got: {r:?}");
    }

    #[test]
    fn regression_warning_skipped_on_small_drop() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Manifest.db"), vec![0u8; 8 * 1024]).unwrap();
        for i in 0..9 {
            std::fs::write(dir.path().join(format!("f{i}")), b"").unwrap();
        }
        // 9 now vs 10 previously = 10% loss < 20% threshold.
        let r = verify_backup(dir.path(), Some(10));
        assert!(r.warning.is_none(), "expected no warning, got: {r:?}");
    }

    #[test]
    fn first_run_without_previous_count_does_not_warn() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Manifest.db"), vec![0u8; 8 * 1024]).unwrap();
        let r = verify_backup(dir.path(), None);
        assert!(r.warning.is_none());
        assert!(r.manifest_ok);
    }
}
