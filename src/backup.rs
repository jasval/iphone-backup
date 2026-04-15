use anyhow::Result;
use chrono::Local;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Instant;

fn log(msg: &str, tx: &Sender<String>, log_path: &Path) {
    let line = format!("[{}] {}", Local::now().format("%H:%M:%S"), msg);
    let _ = tx.send(line.clone());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(f, "{}", line);
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
    let log_path = status_dir.join("ibackup.log");

    log("Discovering devices...", &tx, &log_path);

    let udids_str = match Command::new("idevice_id").arg("-l").output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        Ok(o) => {
            log(
                &format!(
                    "idevice_id error: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
                &tx,
                &log_path,
            );
            String::new()
        }
        Err(e) => {
            log(
                &format!(
                    "ERROR: idevice_id not found ({e}). Install with: brew install libimobiledevice"
                ),
                &tx,
                &log_path,
            );
            String::new()
        }
    };

    let udids: Vec<&str> = udids_str.lines().filter(|l| !l.is_empty()).collect();

    if udids.is_empty() {
        log(
            "No devices found. Is the iPhone on the same Wi-Fi? Is Wi-Fi sync enabled?",
            &tx,
            &log_path,
        );
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

    let mut total = 0u64;
    let mut failed = 0u64;
    let mut names: Vec<String> = Vec::new();

    for udid in &udids {
        total += 1;
        let name = device_info(udid, "DeviceName")
            .unwrap_or_else(|| udid.to_string())
            .replace(' ', "_");
        let model =
            device_info(udid, "ProductType").unwrap_or_else(|| "Unknown".into());
        let ios =
            device_info(udid, "ProductVersion").unwrap_or_else(|| "Unknown".into());
        let dest = backup_path.join(&name);
        std::fs::create_dir_all(&dest)?;

        log(
            &format!("Backing up {} ({})", name, udid),
            &tx,
            &log_path,
        );

        let t0 = Instant::now();
        let ok = run_idevicebackup2(udid, &dest.to_string_lossy(), &tx, &log_path);
        let elapsed = t0.elapsed().as_secs();
        let size = dir_size(&dest);

        if ok {
            log(
                &format!("✓ {} backed up in {}s  ({})", name, elapsed, size),
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
        });
        std::fs::write(
            status_dir.join(format!("{name}.json")),
            serde_json::to_string_pretty(&entry)?,
        )?;
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

fn run_idevicebackup2(udid: &str, dest: &str, tx: &Sender<String>, log_path: &Path) -> bool {
    let mut child = match Command::new("idevicebackup2")
        .args(["-u", udid, "backup", "--full", dest])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(format!("[{}] ERROR: {e}", Local::now().format("%H:%M:%S")));
            return false;
        }
    };

    // Read stderr in a separate thread to avoid deadlock
    let tx2 = tx.clone();
    let log_path2 = log_path.to_path_buf();
    let stderr = child.stderr.take().unwrap();
    let stderr_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().flatten() {
            let _ = tx2.send(line.clone());
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path2)
            {
                let _ = writeln!(f, "{}", line);
            }
        }
    });

    // Read stdout in the current thread
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().flatten() {
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

    let _ = stderr_thread.join();
    child.wait().map(|s| s.success()).unwrap_or(false)
}

fn device_info(udid: &str, key: &str) -> Option<String> {
    let out = Command::new("ideviceinfo")
        .args(["-u", udid, "-k", key])
        .output()
        .ok()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
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
