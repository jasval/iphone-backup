use anyhow::Result;
use chrono::Local;
use serde_json::json;
use std::fs::Permissions;
use std::io::{BufRead as _, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Instant;

fn log(msg: &str, tx: &Sender<String>, log_path: &Path) {
    let line = format!("[{}] {}", Local::now().format("%H:%M:%S"), msg);
    let _ = tx.send(line.clone());
    if let Ok(resolved) = log_path.canonicalize().or_else(|_| {
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
            std::fs::set_permissions(parent, Permissions::from_mode(0o700))?;
        }
        log_path.canonicalize()
    }) {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&resolved)
        {
            let _ = writeln!(f, "{}", line);
        }
    }
}

pub fn run(backup_path: &Path, tx: Sender<String>) -> Result<()> {
    // Verify the backup location is accessible before doing anything.
    if let Err(e) = std::fs::read_dir(backup_path) {
        let msg = format!(
            "ERROR: Backup path '{}' is not accessible: {}. \
             Check that the drive is mounted and the path exists.",
            backup_path.display(),
            e
        );
        let _ = tx.send(msg.clone());
        // Write a minimal status so the TUI shows the failure.
        let status_dir = backup_path.parent().unwrap_or(backup_path).join(".status");
        if std::fs::create_dir_all(&status_dir).is_ok() {
            let summary = serde_json::json!({
                "last_run": chrono::Utc::now().to_rfc3339(),
                "status": "no_storage",
                "total_devices": 0,
                "failed": 0,
            });
            let _ = std::fs::write(
                status_dir.join("summary.json"),
                serde_json::to_string_pretty(&summary)?,
            );
        }
        return Ok(());
    }

    let status_dir = backup_path.join(".status");
    std::fs::create_dir_all(&status_dir)?;
    let _ = std::fs::set_permissions(&status_dir, Permissions::from_mode(0o700));
    let log_path = status_dir.join("ibackup.log");

    let job_id = crate::pid::make_job_id();
    log(&format!("Job ID: {}", job_id), &tx, &log_path);

    log("Discovering devices...", &tx, &log_path);

    // Discover all devices and which ones are reachable via network (WiFi/Tailscale).
    // Network-connected devices are backed up first and with the -n flag so
    // libimobiledevice uses the network path even when USB is also plugged in.
    let (devices, error_logged) = discover_devices(&tx, &log_path);

    if devices.is_empty() {
        if !error_logged {
            log(
                "No devices found. Is the iPhone on the same Wi-Fi or Tailscale? Is Wi-Fi sync enabled?",
                &tx,
                &log_path,
            );
        }
        let summary = json!({
            "last_run": chrono::Utc::now().to_rfc3339(),
            "status": "no_devices",
            "total_devices": 0,
            "failed": 0,
        });
        std::fs::write(
            status_dir.join("summary.json"),
            serde_json::to_string_pretty(&summary)?,
        )?;
        return Ok(());
    }

    let _ = error_logged; // suppress unused warning

    let mut total = 0u64;
    let mut failed = 0u64;
    let mut names: Vec<String> = Vec::new();

    for (udid, use_network) in &devices {
        total += 1;
        // Fetch all device properties in a single ideviceinfo call.
        let info = device_info_batch(udid);
        let name = sanitize_name(
            info.get("DeviceName")
                .map(|s| s.as_str())
                .unwrap_or(udid),
        );
        let model = info
            .get("ProductType")
            .cloned()
            .unwrap_or_else(|| "Unknown".into());
        let ios = info
            .get("ProductVersion")
            .cloned()
            .unwrap_or_else(|| "Unknown".into());
        let dest = backup_path.join(&name);
        std::fs::create_dir_all(&dest)?;

        let conn = if *use_network { "network" } else { "USB" };
        log(
            &format!("Backing up {} ({}) via {}", name, udid, conn),
            &tx,
            &log_path,
        );

        let t0 = Instant::now();
        let ok = run_idevicebackup2(
            udid,
            &dest.to_string_lossy(),
            &job_id,
            *use_network,
            &tx,
            &log_path,
        );
        let elapsed = t0.elapsed().as_secs();
        let size = dir_size(&dest);

        if ok {
            log(
                &format!("✓ {} done in {}s ({})", name, elapsed, size),
                &tx,
                &log_path,
            );
        } else {
            failed += 1;
            log(
                &format!("✗ {} failed after {}s", name, elapsed),
                &tx,
                &log_path,
            );
        }

        let entry = json!({
            "name": name,
            "udid": udid,
            "model": model,
            "ios": ios,
            "status": if ok { "success" } else { "failed" },
            "last_run": chrono::Utc::now().to_rfc3339(),
            "size": size,
            "elapsed_sec": elapsed,
            "connection": if *use_network { "network" } else { "usb" },
        });
        let status_file = status_dir.join(format!("{name}.json"));
        std::fs::write(&status_file, serde_json::to_string_pretty(&entry)?)?;
        let _ = std::fs::set_permissions(&status_file, Permissions::from_mode(0o600));
        names.push(name);
    }

    std::fs::write(
        status_dir.join("manifest.json"),
        serde_json::to_string_pretty(&json!({ "devices": names }))?,
    )?;

    let summary_status = if failed == 0 { "ok" } else { "partial_failure" };
    std::fs::write(
        status_dir.join("summary.json"),
        serde_json::to_string_pretty(&json!({
            "last_run": chrono::Utc::now().to_rfc3339(),
            "total_devices": total,
            "failed": failed,
            "status": summary_status,
        }))?,
    )?;

    log(
        &format!("=== Done. Devices: {}, Failed: {} ===", total, failed),
        &tx,
        &log_path,
    );
    Ok(())
}

fn run_idevicebackup2(
    udid: &str,
    dest: &str,
    job_id: &str,
    use_network: bool,
    tx: &Sender<String>,
    log_path: &Path,
) -> bool {
    let mut cmd = Command::new("idevicebackup2");
    if use_network {
        cmd.arg("--network");
    }
    cmd.args(["--udid", udid, "backup", dest]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(format!("[{}] ERROR: {e}", Local::now().format("%H:%M:%S")));
            return false;
        }
    };

    // Record the child PID immediately so the TUI (or another session) can cancel.
    let child_pid = child.id();
    if let Err(e) = crate::pid::write_job(job_id, child_pid) {
        let _ = tx.send(format!("[warn] could not write PID file: {e}"));
    }

    let tx2 = tx.clone();
    let log_path2 = log_path.to_path_buf();
    let stderr_thread = child.stderr.take().map(|stderr| {
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = tx2.send(line.clone());
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path2)
                {
                    let _ = writeln!(f, "{}", line);
                }
            }
        })
    });

    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line.clone());
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)
            {
                let _ = writeln!(f, "{}", line);
            }
        }
    }

    if let Some(handle) = stderr_thread {
        let _ = handle.join();
    }

    let result = child.wait().map(|s| s.success()).unwrap_or(false);
    let _ = crate::pid::remove_pid();
    result
}

fn sanitize_name(raw: &str) -> String {
    let s: String = raw
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | '\0'))
        .collect();
    let s = s.trim();
    if s.is_empty() || s == "." || s == ".." {
        "Unknown".into()
    } else {
        s.replace(' ', "_")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_normal_name() {
        assert_eq!(sanitize_name("Jaspers iPhone"), "Jaspers_iPhone");
    }

    #[test]
    fn sanitize_strips_forward_slash() {
        assert_eq!(sanitize_name("foo/bar"), "foobar");
    }

    #[test]
    fn sanitize_strips_backslash() {
        assert_eq!(sanitize_name("foo\\bar"), "foobar");
    }

    #[test]
    fn sanitize_strips_null() {
        assert_eq!(sanitize_name("foo\0bar"), "foobar");
    }

    #[test]
    fn sanitize_rejects_dot_dot() {
        assert_eq!(sanitize_name(".."), "Unknown");
    }

    #[test]
    fn sanitize_rejects_dot() {
        assert_eq!(sanitize_name("."), "Unknown");
    }

    #[test]
    fn sanitize_rejects_empty() {
        assert_eq!(sanitize_name(""), "Unknown");
    }

    #[test]
    fn sanitize_rejects_whitespace_only() {
        assert_eq!(sanitize_name("   "), "Unknown");
    }

    #[test]
    fn sanitize_trims_whitespace() {
        assert_eq!(sanitize_name("  phone  "), "phone");
    }

    #[test]
    fn sanitize_path_traversal_combined() {
        assert_eq!(sanitize_name("../etc/cron.d"), "..etccron.d");
    }

    #[test]
    fn sanitize_preserves_valid_name() {
        assert_eq!(sanitize_name("my-phone"), "my-phone");
        assert_eq!(sanitize_name("iPad Pro"), "iPad_Pro");
    }

    #[test]
    fn sanitize_udid_fallback() {
        let udid = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0";
        assert_eq!(sanitize_name(udid), udid);
    }
}

/// Discover all connected devices, returning `(udid, use_network)` pairs.
///
/// Network-reachable devices (WiFi sync / Tailscale) come first and are
/// flagged `use_network = true` so `idevicebackup2 --network` is used,
/// which keeps the USB port free and works over Tailscale. USB-only devices
/// follow with `use_network = false`.
///
/// Returns `(devices, fatal_error_logged)`.
fn discover_devices(
    tx: &Sender<String>,
    log_path: &Path,
) -> (Vec<(String, bool)>, bool) {
    // -- All devices (USB + network) ------------------------------------------
    let all_udids: Vec<String> = match Command::new("idevice_id").arg("--list").output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
            .collect(),
        Ok(o) => {
            log(
                &format!(
                    "idevice_id error: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
                tx,
                log_path,
            );
            return (vec![], true);
        }
        Err(e) => {
            log(
                &format!(
                    "ERROR: idevice_id not found ({e}). Install with: brew install libimobiledevice"
                ),
                tx,
                log_path,
            );
            return (vec![], true);
        }
    };

    // -- Network-only devices (WiFi / Tailscale) -------------------------------
    let network_udids: std::collections::HashSet<String> =
        match Command::new("idevice_id").args(["--network", "--list"]).output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().to_string())
                .collect(),
            _ => std::collections::HashSet::new(),
        };

    if !network_udids.is_empty() {
        log(
            &format!(
                "Found {} device(s) via network (WiFi/Tailscale), {} total",
                network_udids.len(),
                all_udids.len()
            ),
            tx,
            log_path,
        );
    }

    // Network-reachable devices first, then USB-only.
    let mut result: Vec<(String, bool)> = Vec::new();
    for udid in &all_udids {
        if network_udids.contains(udid) {
            result.push((udid.clone(), true));
        }
    }
    for udid in &all_udids {
        if !network_udids.contains(udid) {
            result.push((udid.clone(), false));
        }
    }
    (result, false)
}

/// Fetch all device properties in a single `ideviceinfo` call and return
/// them as a map.  Much faster than one call per key.
fn device_info_batch(udid: &str) -> std::collections::HashMap<String, String> {
    let out = match Command::new("ideviceinfo").args(["--udid", udid]).output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => return std::collections::HashMap::new(),
    };
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ": ");
            let key = parts.next()?.trim().to_string();
            let val = parts.next()?.trim().to_string();
            if key.is_empty() || val.is_empty() {
                None
            } else {
                Some((key, val))
            }
        })
        .collect()
}

fn dir_size(path: &Path) -> String {
    Command::new("du")
        .args(["-sh", path.to_str().unwrap_or("")])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout)
                    .split_whitespace()
                    .next()
                    .map(|s| s.to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "?".into())
}
