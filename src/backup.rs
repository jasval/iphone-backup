use anyhow::{Context, Result};
use chrono::Local;
use serde_json::json;
use std::fs::Permissions;
use std::io::{BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::imd;

pub(crate) struct BackupOutcome {
    pub ok: bool,
    pub reason: Option<String>,
}

/// Top-level result of a whole `run()` invocation. Used by `main.rs` to
/// decide whether to fire a failure notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Ok,
    NoStorage,
    NoDevices,
    PartialFailure { failed: u64, total: u64 },
}

impl RunOutcome {
    pub fn is_failure(&self) -> bool {
        !matches!(self, RunOutcome::Ok)
    }

    pub fn summary_line(&self) -> String {
        match self {
            RunOutcome::Ok => "backup completed".into(),
            RunOutcome::NoStorage => "backup path is not accessible".into(),
            RunOutcome::NoDevices => "no devices were reachable".into(),
            RunOutcome::PartialFailure { failed, total } => {
                format!("{failed}/{total} device(s) failed")
            }
        }
    }
}

/// Strip ANSI/VT100 escape sequences (e.g. colour codes) from a string.
pub(crate) fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(next) = chars.next() {
                if next == '[' {
                    // CSI sequence: consume until an ASCII letter (the terminator)
                    for cc in chars.by_ref() {
                        if cc.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                // bare ESC + non-'[': skip both characters
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Check whether a directory contains markers of a valid idevicebackup2 backup.
/// Only removes the directory on failure if neither marker is present, meaning
/// idevicebackup2 never successfully wrote any data.
fn is_valid_backup(dir: &Path) -> bool {
    dir.join("Manifest.db").exists() || dir.join("Status.plist").exists()
}

/// Read all bytes from `reader`, splitting on `\r` or `\n`, strip ANSI codes,
/// truncate long lines, and send each non-empty line to `tx` and `log_path`.
fn drain_stream(
    reader: impl Read,
    tx: &Sender<String>,
    log_path: &Path,
) {
    let mut reader = BufReader::new(reader);
    let mut buf: Vec<u8> = Vec::new();

    let flush = |buf: &mut Vec<u8>| {
        if buf.is_empty() {
            return;
        }
        let raw = String::from_utf8_lossy(buf).to_string();
        let clean = strip_ansi(&raw);
        let line = if clean.len() > 512 {
            format!("{}…", &clean[..512])
        } else {
            clean
        };
        // Emit bytes sentinel before the normal line so the TUI updates the
        // overall gauge before displaying the progress text.
        if let Some((cur, tot)) = imd::parse_bytes_progress(&line) {
            let _ = tx.send(format!("__BACKUP_BYTES__ {cur}/{tot}"));
        }
        let _ = tx.send(line.clone());
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
        {
            let _ = writeln!(f, "{line}");
        }
        buf.clear();
    };

    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if byte[0] == b'\n' || byte[0] == b'\r' {
                    flush(&mut buf);
                } else {
                    buf.push(byte[0]);
                }
            }
        }
    }
    flush(&mut buf);
}

fn log(msg: &str, tx: &Sender<String>, log_path: &Path) {
    let line = format!("[{}] {}", Local::now().format("%H:%M:%S"), msg);
    let _ = tx.send(line.clone());
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(f, "{line}");
    }
}

pub fn run(config: &Config, tx: &Sender<String>) -> Result<RunOutcome> {
    let backup_path = config.backup_path();
    let backup_path = backup_path.as_path();
    // Pre-flight: the backup location must exist, be a directory, be writable,
    // and have enough free space. Covers unmounted external drives, typos in
    // the config path, and full disks.
    if let Err(e) = crate::preflight::check_backup_path(backup_path, config.min_free_gb) {
        let msg = format!("ERROR: pre-flight check failed: {e}");
        let _ = tx.send(msg);
        let status_dir = backup_path.parent().unwrap_or(backup_path).join(".status");
        if std::fs::create_dir_all(&status_dir).is_ok() {
            let summary = serde_json::json!({
                "last_run": chrono::Utc::now().to_rfc3339(),
                "status": "no_storage",
                "total_devices": 0,
                "failed": 0,
                "reason": e.to_string(),
            });
            let _ = crate::status::atomic_write(
                &status_dir.join("summary.json"),
                serde_json::to_string_pretty(&summary)?.as_bytes(),
            );
        }
        return Ok(RunOutcome::NoStorage);
    }

    let status_dir = backup_path.join(".status");
    std::fs::create_dir_all(&status_dir)?;
    let _ = std::fs::set_permissions(&status_dir, Permissions::from_mode(0o700));
    let log_path = status_dir.join("ibackup.log");

    let job_id = crate::pid::make_job_id();
    log(&format!("Job ID: {job_id}"), tx, &log_path);

    log("Discovering devices...", tx, &log_path);

    // Discover all devices and which ones are reachable via network (WiFi/Tailscale).
    // Network-connected devices are backed up first and with the -n flag so
    // libimobiledevice uses the network path even when USB is also plugged in.
    let (devices, error_logged) = discover_devices(tx, &log_path);

    if devices.is_empty() {
        if !error_logged {
            log(
                "No devices found. Is the iPhone on the same Wi-Fi or Tailscale? Is Wi-Fi sync enabled?",
                tx,
                &log_path,
            );
        }
        let summary = json!({
            "last_run": chrono::Utc::now().to_rfc3339(),
            "status": "no_devices",
            "total_devices": 0,
            "failed": 0,
        });
        crate::status::atomic_write(
            &status_dir.join("summary.json"),
            serde_json::to_string_pretty(&summary)?.as_bytes(),
        )?;
        return Ok(RunOutcome::NoDevices);
    }

    let _ = error_logged; // suppress unused warning

    // Resolve the encryption password once per run, if configured. We surface
    // helper-command failures as a hard error because silently falling back
    // to an unencrypted backup would be a nasty surprise.
    let encryption_password = match &config.encryption_password_cmd {
        Some(cmd) => match resolve_password(cmd) {
            Ok(p) => Some(p),
            Err(e) => {
                log(
                    &format!("ERROR: encryption_password_cmd failed: {e}"),
                    tx,
                    &log_path,
                );
                return Ok(RunOutcome::PartialFailure {
                    failed: devices.len() as u64,
                    total: devices.len() as u64,
                });
            }
        },
        None => None,
    };

    let mut total = 0u64;
    let mut failed = 0u64;
    let mut names: Vec<String> = Vec::new();

    for (udid, use_network) in &devices {
        total += 1;
        // Fetch all device properties in a single ideviceinfo call.
        let info = imd::device_info(udid).unwrap_or_default();
        let name = sanitize_name(
            info.get("DeviceName")
                .map_or(udid, std::string::String::as_str),
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

        // Send a priori size so the TUI can show overall progress.
        if let Some(total_bytes) = query_backup_size(udid) {
            let _ = tx.send(format!("__BACKUP_TOTAL_BYTES__ {total_bytes}"));
        }

        let conn = if *use_network { "network" } else { "USB" };
        log(
            &format!("Backing up {name} ({udid}) via {conn}"),
            tx,
            &log_path,
        );

        let t0 = Instant::now();
        let outcome = run_idevicebackup2(
            udid,
            &dest.to_string_lossy(),
            &job_id,
            *use_network,
            config.backup_timeout_minutes,
            encryption_password.as_deref(),
            tx,
            &log_path,
        );
        let elapsed = t0.elapsed().as_secs();
        let size = dir_size(&dest);

        // Verification (post-success only). Uses the previous run's file count
        // to detect a sudden collapse that would indicate data loss.
        let previous_count = read_previous_file_count(&status_dir, &name);
        let verification = if outcome.ok {
            let v = crate::verify::verify_backup(&dest, previous_count);
            if let Some(w) = &v.warning {
                log(&format!("  verification warning: {w}"), tx, &log_path);
            }
            Some(v)
        } else {
            None
        };

        if outcome.ok {
            log(
                &format!("✓ {name} done in {elapsed}s ({size})"),
                tx,
                &log_path,
            );
            // Best-effort retention: archive the fresh backup and prune
            // older ones. Failures are logged but don't mark the run failed.
            archive_and_prune(
                backup_path,
                &name,
                &dest,
                config.retention_keep_last,
                config.retention_keep_days,
                tx,
                &log_path,
            );
        } else {
            failed += 1;
            let reason = outcome.reason.as_deref().unwrap_or("failed");
            log(
                &format!("✗ {name} failed after {elapsed}s ({reason})"),
                tx,
                &log_path,
            );
            // Remove empty/corrupt backup directories — only if idevicebackup2
            // never wrote valid data (no Manifest.db or Status.plist present).
            // Pre-existing incremental backups that fail mid-way keep their
            // prior data intact, so those are left alone.
            if !is_valid_backup(&dest) {
                log(
                    &format!("  Removing empty backup dir: {}", dest.display()),
                    tx,
                    &log_path,
                );
                let _ = std::fs::remove_dir_all(&dest);
            }
        }

        let mut entry = json!({
            "name": name,
            "udid": udid,
            "model": model,
            "ios": ios,
            "status": if outcome.ok { "success" } else { "failed" },
            "last_run": chrono::Utc::now().to_rfc3339(),
            "size": size,
            "elapsed_sec": elapsed,
            "connection": if *use_network { "network" } else { "usb" },
        });
        if let Some(reason) = &outcome.reason {
            entry["reason"] = json!(reason);
        }
        if let Some(v) = &verification {
            entry["verification"] = serde_json::to_value(v).unwrap_or(serde_json::Value::Null);
        }
        let status_file = status_dir.join(format!("{name}.json"));
        crate::status::atomic_write(&status_file, serde_json::to_string_pretty(&entry)?.as_bytes())?;
        let _ = std::fs::set_permissions(&status_file, Permissions::from_mode(0o600));
        // Only add to manifest if the backup dir still exists (may have been
        // removed above when the backup failed with no valid data).
        if outcome.ok || dest.exists() {
            names.push(name);
        }
    }

    crate::status::atomic_write(
        &status_dir.join("manifest.json"),
        serde_json::to_string_pretty(&json!({ "devices": names }))?.as_bytes(),
    )?;

    let summary_status = if failed == 0 { "ok" } else { "partial_failure" };
    let imd_version = imd::idevicebackup2_version().ok();
    let mut summary = json!({
        "last_run": chrono::Utc::now().to_rfc3339(),
        "total_devices": total,
        "failed": failed,
        "status": summary_status,
    });
    if let Some(v) = imd_version {
        summary["idevicebackup2_version"] = json!(v);
    }
    crate::status::atomic_write(
        &status_dir.join("summary.json"),
        serde_json::to_string_pretty(&summary)?.as_bytes(),
    )?;

    log(
        &format!("=== Done. Devices: {total}, Failed: {failed} ==="),
        tx,
        &log_path,
    );
    if failed == 0 {
        Ok(RunOutcome::Ok)
    } else {
        Ok(RunOutcome::PartialFailure { failed, total })
    }
}

fn run_idevicebackup2(
    udid: &str,
    dest: &str,
    job_id: &str,
    use_network: bool,
    timeout_minutes: u64,
    encryption_password: Option<&str>,
    tx: &Sender<String>,
    log_path: &Path,
) -> BackupOutcome {
    let mut cmd = Command::new("idevicebackup2");
    if use_network {
        cmd.arg("--network");
    }
    if encryption_password.is_some() {
        // -i makes idevicebackup2 read the password interactively from stdin,
        // which is where we write it below.
        cmd.arg("-i");
    }
    cmd.args(["--udid", udid, "backup", dest]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    if encryption_password.is_some() {
        cmd.stdin(Stdio::piped());
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(format!("[{}] ERROR: {e}", Local::now().format("%H:%M:%S")));
            return BackupOutcome {
                ok: false,
                reason: Some(format!("spawn error: {e}")),
            };
        }
    };

    // Feed the password then close stdin so idevicebackup2 sees EOF after
    // consuming the prompt. Done before the read loop below so the child
    // never blocks waiting for input.
    if let (Some(pw), Some(mut stdin)) = (encryption_password, child.stdin.take()) {
        let _ = stdin.write_all(pw.as_bytes());
        let _ = stdin.write_all(b"\n");
        // Drop closes the pipe and triggers EOF on the child side.
    }

    let child_pid = child.id();
    if let Err(e) = crate::pid::write_job(job_id, child_pid) {
        let _ = tx.send(format!("[warn] could not write PID file: {e}"));
    }

    // Drain both streams on background threads so the main thread can poll
    // the child and enforce the timeout. The threads exit naturally when the
    // pipes close (either on success or after we kill the child).
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_thread = stdout.map(|s| {
        let tx = tx.clone();
        let log_path = log_path.to_path_buf();
        std::thread::spawn(move || drain_stream(s, &tx, &log_path))
    });
    let stderr_thread = stderr.map(|s| {
        let tx = tx.clone();
        let log_path = log_path.to_path_buf();
        std::thread::spawn(move || drain_stream(s, &tx, &log_path))
    });

    let deadline = Instant::now() + Duration::from_secs(timeout_minutes.saturating_mul(60));
    let poll = Duration::from_millis(500);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = tx.send(format!(
                        "[{}] timeout after {timeout_minutes}m — killing idevicebackup2 (pid {child_pid})",
                        Local::now().format("%H:%M:%S")
                    ));
                    let _ = child.kill();
                    timed_out = true;
                    break child.wait().ok();
                }
                std::thread::sleep(poll);
            }
            Err(e) => {
                let _ = tx.send(format!("[warn] try_wait failed: {e}"));
                break None;
            }
        }
    };

    if let Some(h) = stdout_thread {
        let _ = h.join();
    }
    if let Some(h) = stderr_thread {
        let _ = h.join();
    }

    let _ = crate::pid::remove_pid();

    if timed_out {
        return BackupOutcome {
            ok: false,
            reason: Some(format!("timeout after {timeout_minutes}m")),
        };
    }
    match status {
        Some(s) if s.success() => BackupOutcome { ok: true, reason: None },
        Some(s) => BackupOutcome {
            ok: false,
            reason: Some(format!("exited with {s}")),
        },
        None => BackupOutcome {
            ok: false,
            reason: Some("wait failed".into()),
        },
    }
}

/// Read the previous run's recorded file count for this device, so the
/// verification step can spot a regression. Returns `None` on first run
/// or when the status JSON is missing/unreadable.
fn read_previous_file_count(status_dir: &Path, device_name: &str) -> Option<u64> {
    let path = status_dir.join(format!("{device_name}.json"));
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("verification")?.get("file_count")?.as_u64()
}

/// Run the user-supplied helper command and return its stdout as the
/// encryption password. Runs under `sh -c` so the shape matches `git`'s
/// `credential.helper` convention (e.g. `security find-generic-password -w ...`).
fn resolve_password(cmd: &str) -> Result<String> {
    let out = Command::new("sh")
        .args(["-c", cmd])
        .output()
        .context("spawning password helper")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        anyhow::bail!("password helper exited with {}: {stderr}", out.status);
    }
    let mut s = String::from_utf8(out.stdout)
        .context("password helper output was not valid UTF-8")?;
    while s.ends_with('\n') || s.ends_with('\r') {
        s.pop();
    }
    if s.is_empty() {
        anyhow::bail!("password helper produced empty output");
    }
    Ok(s)
}

/// Archive the just-completed backup (APFS clone) and prune old archives.
/// Errors are logged but never propagated — retention is a best-effort feature.
fn archive_and_prune(
    backup_root: &Path,
    device_name: &str,
    source: &Path,
    keep_last: Option<u32>,
    keep_days: Option<u32>,
    tx: &Sender<String>,
    log_path: &Path,
) {
    if keep_last.is_none() && keep_days.is_none() {
        return;
    }
    match crate::retention::archive(backup_root, device_name, source) {
        Ok(path) => log(
            &format!("  archived to {}", path.display()),
            tx,
            log_path,
        ),
        Err(e) => {
            log(&format!("  archive failed: {e}"), tx, log_path);
            return;
        }
    }
    match crate::retention::prune(backup_root, device_name, keep_last, keep_days) {
        Ok(removed) if !removed.is_empty() => log(
            &format!("  pruned {} old archive(s)", removed.len()),
            tx,
            log_path,
        ),
        Ok(_) => {}
        Err(e) => log(&format!("  prune failed: {e}"), tx, log_path),
    }
}

/// Query the device's total data capacity via `ideviceinfo --domain com.apple.disk_usage`.
/// Returns bytes, used as the a-priori total for the overall progress gauge.
fn query_backup_size(udid: &str) -> Option<u64> {
    let out = Command::new("ideviceinfo")
        .args(["--udid", udid, "--domain", "com.apple.disk_usage"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // Prefer AmountDataUsed (actual data), fall back to TotalDataCapacity.
    let text = String::from_utf8_lossy(&out.stdout);
    for key in &["AmountDataUsed", "TotalDataCapacity", "TotalDiskCapacity"] {
        for line in text.lines() {
            if let Some(val) = line.strip_prefix(&format!("{key}: ")) {
                if let Ok(n) = val.trim().parse::<u64>() {
                    return Some(n);
                }
            }
        }
    }
    None
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
    fn strip_ansi_plain_text() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }

    #[test]
    fn strip_ansi_color_code() {
        assert_eq!(strip_ansi("\x1b[31mERROR\x1b[0m"), "ERROR");
    }

    #[test]
    fn strip_ansi_complex_sequence() {
        assert_eq!(strip_ansi("\x1b[1;32mOK\x1b[0m done"), "OK done");
    }

    #[test]
    fn is_valid_backup_with_manifest_db() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Manifest.db"), "data").unwrap();
        assert!(is_valid_backup(dir.path()));
    }

    #[test]
    fn is_valid_backup_with_status_plist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Status.plist"), "data").unwrap();
        assert!(is_valid_backup(dir.path()));
    }

    #[test]
    fn is_valid_backup_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_valid_backup(dir.path()));
    }

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
    fn log_writes_to_file_even_when_it_does_not_exist_yet() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("sub/ibackup.log");
        let (tx, _rx) = std::sync::mpsc::channel();
        log("hello from log", &tx, &log_path);
        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("hello from log"));
    }

    #[test]
    fn sanitize_udid_fallback() {
        let udid = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0";
        assert_eq!(sanitize_name(udid), udid);
    }

    /// Drive the same poll-and-kill logic `run_idevicebackup2` uses, but
    /// against an arbitrary long-running child, to verify the timeout fires
    /// and the child is reaped.
    fn wait_with_timeout(mut child: std::process::Child, timeout: Duration) -> (bool, bool) {
        let deadline = Instant::now() + timeout;
        let mut timed_out = false;
        let exited_ok = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s.success(),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        timed_out = true;
                        let _ = child.wait();
                        break false;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => break false,
            }
        };
        (exited_ok, timed_out)
    }

    #[test]
    fn timeout_kills_long_running_child() {
        let child = Command::new("/bin/sh")
            .args(["-c", "sleep 60"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let (ok, timed_out) = wait_with_timeout(child, Duration::from_millis(200));
        assert!(!ok, "long-running child should not have exited cleanly");
        assert!(timed_out, "timeout branch should have fired");
    }

    #[test]
    fn resolve_password_returns_trimmed_stdout() {
        let pw = resolve_password("printf 'secret\\n'").unwrap();
        assert_eq!(pw, "secret");
    }

    #[test]
    fn resolve_password_fails_on_empty_output() {
        let err = resolve_password("true").unwrap_err();
        assert!(err.to_string().contains("empty"), "got: {err}");
    }

    #[test]
    fn resolve_password_fails_on_non_zero_exit() {
        let err = resolve_password("echo oops >&2; exit 1").unwrap_err();
        let s = err.to_string();
        assert!(s.contains("exited with"), "got: {s}");
    }

    #[test]
    fn resolve_password_preserves_internal_whitespace() {
        let pw = resolve_password("printf 'hello world\\n'").unwrap();
        assert_eq!(pw, "hello world");
    }

    #[test]
    fn timeout_does_not_fire_for_fast_child() {
        let child = Command::new("/bin/sh")
            .args(["-c", "true"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let (ok, timed_out) = wait_with_timeout(child, Duration::from_secs(5));
        assert!(ok, "`true` should exit cleanly");
        assert!(!timed_out, "fast child should not hit the deadline");
    }
}

/// Discover all connected devices, returning `(udid, use_network)` pairs.
///
/// Network-reachable devices (`WiFi` sync / Tailscale) come first and are
/// flagged `use_network = true` so `idevicebackup2 --network` is used,
/// which keeps the USB port free and works over Tailscale. USB-only devices
/// follow with `use_network = false`.
///
/// Returns `(devices, fatal_error_logged)`.
fn discover_devices(
    tx: &Sender<String>,
    log_path: &Path,
) -> (Vec<(String, bool)>, bool) {
    let all_udids: Vec<String> = match imd::list_usb() {
        Ok(v) => v,
        Err(imd::ImdError::NotFound(_)) => {
            log(
                "ERROR: idevice_id not found. Install with: brew install libimobiledevice",
                tx,
                log_path,
            );
            return (vec![], true);
        }
        Err(e) => {
            log(&format!("idevice_id error: {e}"), tx, log_path);
            return (vec![], true);
        }
    };

    let network_udids: std::collections::HashSet<String> =
        imd::list_network().unwrap_or_default().into_iter().collect();

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

    // Network-reachable devices first (including network-only), then USB-only.
    let mut result: Vec<(String, bool)> = Vec::new();
    for udid in &network_udids {
        result.push((udid.clone(), true));
    }
    for udid in &all_udids {
        if !network_udids.contains(udid) {
            result.push((udid.clone(), false));
        }
    }
    (result, false)
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
                    .map(std::string::ToString::to_string)
            } else {
                None
            }
        })
        .unwrap_or_else(|| "?".into())
}
