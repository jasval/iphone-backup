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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let record = JobRecord {
        job_id: job_id.to_string(),
        child_pid,
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    std::fs::write(&path, serde_json::to_string_pretty(&record)?)?;
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
        let start_time = chrono::DateTime::parse_from_rfc3339(&record.started_at)
            .map(|d| d.format("%H:%M:%S").to_string())
            .unwrap_or_else(|_| "?".into());
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
    kill_child(info.child_pid)?;
    let _ = remove_pid();
    Ok(())
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn is_pid_running(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

fn kill_child(pid: u32) -> Result<()> {
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    if is_pid_running(pid) {
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
    Ok(())
}
