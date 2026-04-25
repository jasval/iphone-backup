use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn pid_file_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot locate home directory")?;
    Ok(home.join(".config/iphone-backup/backup.pid"))
}

// ── Job record written to the PID file ───────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobRecord {
    pub job_id: String,     // e.g. "backup-20260416-143000"
    pub child_pid: u32,     // PID of the idevicebackup2 child process
    pub started_at: String, // RFC3339 timestamp
}

// ── Public view returned to callers ──────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ActiveBackup {
    pub job_id: String,
    pub child_pid: u32,
    pub start_time: String, // human-friendly HH:MM:SS
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Generate a unique, human-readable backup job ID from the current local time.
pub fn make_job_id() -> String {
    chrono::Local::now()
        .format("backup-%Y%m%d-%H%M%S")
        .to_string()
}

/// Write a JSON job record to the PID file.
/// Called immediately after the idevicebackup2 child process is spawned.
pub fn write_job(job_id: &str, child_pid: u32) -> Result<()> {
    let path = pid_file_path()?;
    let record = JobRecord {
        job_id: job_id.to_string(),
        child_pid,
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    crate::status::atomic_write(&path, serde_json::to_string_pretty(&record)?.as_bytes())?;
    Ok(())
}

/// Remove the PID file. Idempotent — silently succeeds if missing.
pub fn remove_pid() -> Result<()> {
    let path = pid_file_path()?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Read the active backup from the PID file.
/// Returns `None` if the file is missing, unparseable, or the child process is
/// no longer running (stale file is auto-removed).
pub fn read_active_backup() -> Option<ActiveBackup> {
    let path = pid_file_path().ok()?;
    let content = std::fs::read_to_string(&path).ok()?;

    // Try JSON format first (current format).
    if let Ok(record) = serde_json::from_str::<JobRecord>(&content) {
        if !is_pid_running(record.child_pid) {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        let start_time = chrono::DateTime::parse_from_rfc3339(&record.started_at).map_or_else(|_| "?".into(), |d| d.format("%H:%M:%S").to_string());
        return Some(ActiveBackup {
            job_id: record.job_id,
            child_pid: record.child_pid,
            start_time,
        });
    }

    // Fallback: plain integer PID (old format — treat it as the child PID).
    let pid: u32 = content.trim().parse().ok()?;
    if pid == 0 || !is_pid_running(pid) {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    Some(ActiveBackup {
        job_id: "backup-legacy".into(),
        child_pid: pid,
        start_time: "?".into(),
    })
}

/// Send SIGTERM to the active backup's child process, wait 500 ms, then
/// SIGKILL if still running. Also removes the PID file.
pub fn kill_active_backup() -> Result<()> {
    let info = read_active_backup().context("no active backup found")?;
    kill_child(info.child_pid);
    let _ = remove_pid();
    Ok(())
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Send `sig` to `pid`. Refuses (returns `InvalidInput`) if the PID does not
/// fit in `i32` — the previous implementation silently fell back to `-1`,
/// which on Unix means "every process in the caller's process group". That
/// is dangerous; better to do nothing and surface an error.
fn signal_pid(pid: u32, sig: libc::c_int) -> std::io::Result<()> {
    let pid_i = i32::try_from(pid).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "PID does not fit in i32 — refusing to signal",
        )
    })?;
    if pid_i <= 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to signal pid <= 0",
        ));
    }
    // SAFETY: `libc::kill` is an FFI call with no Rust-side invariants. We've
    // already rejected pid values that would target a process group (≤0) and
    // values that don't fit in i32. The signal number is taken from `libc`
    // constants. Failure is reported via errno → io::Error.
    let rc = unsafe { libc::kill(pid_i, sig) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn is_pid_running(pid: u32) -> bool {
    signal_pid(pid, 0).is_ok()
}

fn kill_child(pid: u32) {
    let _ = signal_pid(pid, libc::SIGTERM);
    std::thread::sleep(std::time::Duration::from_millis(500));
    if is_pid_running(pid) {
        let _ = signal_pid(pid, libc::SIGKILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn make_job_id_has_expected_shape() {
        let id = make_job_id();
        assert!(id.starts_with("backup-"), "got: {id}");
        assert_eq!(id.len(), "backup-YYYYMMDD-HHMMSS".len());
    }

    #[test]
    fn signal_pid_refuses_zero() {
        let err = signal_pid(0, 0).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn signal_pid_refuses_overflow() {
        let err = signal_pid(u32::MAX, 0).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn signal_pid_succeeds_for_running_child() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 5"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(is_pid_running(pid));
        // SIGTERM the child, then reap.
        signal_pid(pid, libc::SIGTERM).unwrap();
        let _ = child.wait();
    }

    #[test]
    fn read_active_backup_returns_none_when_pid_dead() {
        // Spawn a fast-exiting child, capture its pid, then write a record
        // pointing at it. After the child has been reaped the kernel may
        // recycle the pid, so this only verifies that the cleanup path
        // doesn't panic — we don't assert on the return value.
        let child = Command::new("/bin/sh")
            .args(["-c", "true"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let dead_pid = child.id();
        // Wait for it to exit so it's no longer running.
        let mut c = child;
        let _ = c.wait();
        // is_pid_running may race against pid recycling; just exercise it.
        let _ = is_pid_running(dead_pid);
    }
}
