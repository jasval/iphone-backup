use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const LABEL: &str = "com.user.iphone-backup";
const LAUNCHD_LOG: &str = "/tmp/iphone-backup-launchd.log";
const LAUNCHD_LOG_ROLLS: u32 = 3;

/// Rotate the launchd stdout/stderr log when it exceeds `max_mb` MiB.
///
/// Keeps up to [`LAUNCHD_LOG_ROLLS`] rolls (`.1`, `.2`, …). Best-effort:
/// any I/O error is swallowed so a rotate failure can never abort a backup.
pub fn rotate_launchd_log(max_mb: u64) {
    rotate_log_at(Path::new(LAUNCHD_LOG), max_mb, LAUNCHD_LOG_ROLLS);
}

/// Rotate a generic log at `path` when it exceeds `max_mb` MiB, keeping
/// `rolls` prior generations. Extracted for testability.
pub(crate) fn rotate_log_at(path: &Path, max_mb: u64, rolls: u32) {
    if max_mb == 0 || rolls == 0 {
        return;
    }
    let max_bytes = max_mb.saturating_mul(1024 * 1024);
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if size <= max_bytes {
        return;
    }
    let base = path.to_string_lossy();
    let oldest = format!("{base}.{rolls}");
    let _ = std::fs::remove_file(&oldest);
    for i in (1..rolls).rev() {
        let from = format!("{base}.{i}");
        let to = format!("{base}.{}", i + 1);
        let _ = std::fs::rename(&from, &to);
    }
    let _ = std::fs::rename(path, format!("{base}.1"));
}

const PLIST_TEMPLATE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.user.iphone-backup</string>

    <key>ProgramArguments</key>
    <array>
        <string>BINARY_PATH</string>
        <string>backup</string>
    </array>

    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>BREW_BIN:/usr/local/bin:/usr/bin:/bin</string>
    </dict>

    <key>StartCalendarInterval</key>
    <dict>
        <key>Hour</key>
        <integer>SCHED_HOUR</integer>
        <key>Minute</key>
        <integer>SCHED_MINUTE</integer>
    </dict>

    <key>RunAtLoad</key>
    <false/>

    <key>StandardOutPath</key>
    <string>/tmp/iphone-backup-launchd.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/iphone-backup-launchd.log</string>
</dict>
</plist>
"#;

#[derive(Debug, Clone)]
pub struct LaunchdStatus {
    pub installed: bool,
    pub loaded: bool,
    pub plist_path: PathBuf,
}

pub fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/LaunchAgents/com.user.iphone-backup.plist")
}

pub fn status() -> LaunchdStatus {
    let path = plist_path();
    let installed = path.exists();
    let loaded = if installed {
        Command::new("launchctl")
            .args(["list", LABEL])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    } else {
        false
    };
    LaunchdStatus {
        installed,
        loaded,
        plist_path: path,
    }
}

/// Write the plist (using the current executable path) and load it.
pub fn install(binary_path: &Path, hour: u8, minute: u8) -> Result<()> {
    write_plist(binary_path, hour, minute)?;
    load()?;
    Ok(())
}

/// Rewrite the plist with a new schedule and reload the agent.
pub fn set_schedule(hour: u8, minute: u8) -> Result<()> {
    // Keep the same binary path that's already in the plist (or fall back to current exe).
    let binary_path = current_binary_path();
    write_plist(&binary_path, hour, minute)?;
    // Unload then load so launchd picks up the new interval.
    let _ = unload();
    load()?;
    Ok(())
}

fn brew_bin() -> String {
    // Prefer the running brew prefix; fall back to both common locations.
    if let Ok(out) = Command::new("brew").args(["--prefix"]).output() {
        if out.status.success() {
            let prefix = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !prefix.is_empty() {
                return format!("{prefix}/bin");
            }
        }
    }
    // Apple Silicon and Intel fallback
    "/opt/homebrew/bin:/usr/local/bin".to_string()
}

fn write_plist(binary_path: &Path, hour: u8, minute: u8) -> Result<()> {
    let path = plist_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = PLIST_TEMPLATE
        .replace("BINARY_PATH", &binary_path.to_string_lossy())
        .replace("BREW_BIN", &brew_bin())
        .replace("SCHED_HOUR", &hour.to_string())
        .replace("SCHED_MINUTE", &minute.to_string());
    std::fs::write(&path, content)
        .with_context(|| format!("writing plist to {}", path.display()))?;
    Ok(())
}

fn current_binary_path() -> std::path::PathBuf {
    std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("/usr/local/bin/iphone-backup"))
}

pub fn load() -> Result<()> {
    let path = plist_path();
    let out = Command::new("launchctl")
        .args(["load", path.to_str().unwrap_or("")])
        .output()
        .context("running launchctl load")?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // "already loaded" is not an error
        if !msg.contains("already") {
            anyhow::bail!("launchctl load: {msg}");
        }
    }
    Ok(())
}

pub fn unload() -> Result<()> {
    let path = plist_path();
    let out = Command::new("launchctl")
        .args(["unload", path.to_str().unwrap_or("")])
        .output()
        .context("running launchctl unload")?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if !msg.contains("not loaded") && !msg.contains("Could not find") {
            anyhow::bail!("launchctl unload: {msg}");
        }
    }
    Ok(())
}

pub fn start() -> Result<()> {
    let out = Command::new("launchctl")
        .args(["start", LABEL])
        .output()
        .context("running launchctl start")?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        anyhow::bail!("launchctl start: {msg}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_plist(binary_path: &str, hour: u8, minute: u8) -> String {
        PLIST_TEMPLATE
            .replace("BINARY_PATH", binary_path)
            .replace("BREW_BIN", "/opt/homebrew/bin")
            .replace("SCHED_HOUR", &hour.to_string())
            .replace("SCHED_MINUTE", &minute.to_string())
    }

    #[test]
    fn plist_contains_label() {
        let xml = render_plist("/usr/local/bin/iphone-backup", 2, 0);
        assert!(xml.contains("<string>com.user.iphone-backup</string>"));
    }

    #[test]
    fn plist_contains_binary_path() {
        let xml = render_plist("/opt/homebrew/bin/iphone-backup", 2, 0);
        assert!(xml.contains("<string>/opt/homebrew/bin/iphone-backup</string>"));
        assert!(xml.contains("<string>backup</string>"));
    }

    #[test]
    fn plist_contains_schedule() {
        let xml = render_plist("/usr/local/bin/iphone-backup", 14, 30);
        assert!(xml.contains("<integer>14</integer>"));
        assert!(xml.contains("<integer>30</integer>"));
    }

    #[test]
    fn plist_contains_midnight_schedule() {
        let xml = render_plist("/usr/local/bin/iphone-backup", 0, 0);
        assert!(xml.contains("<integer>0</integer>"));
    }

    #[test]
    fn plist_no_unreplaced_placeholders() {
        let xml = render_plist("/usr/local/bin/iphone-backup", 2, 0);
        assert!(!xml.contains("BINARY_PATH"));
        assert!(!xml.contains("BREW_BIN"));
        assert!(!xml.contains("SCHED_HOUR"));
        assert!(!xml.contains("SCHED_MINUTE"));
    }

    #[test]
    fn plist_is_valid_xml() {
        let xml = render_plist("/usr/local/bin/iphone-backup", 2, 0);
        assert!(xml.contains("<?xml"));
        assert!(xml.contains("<plist"));
        assert!(xml.contains("</plist>"));
        assert!(xml.contains("<dict>"));
        assert!(xml.contains("</dict>"));
    }

    #[test]
    fn plist_has_log_paths() {
        let xml = render_plist("/usr/local/bin/iphone-backup", 2, 0);
        assert!(xml.contains("/tmp/iphone-backup-launchd.log"));
    }

    #[test]
    fn plist_run_at_load_false() {
        let xml = render_plist("/usr/local/bin/iphone-backup", 2, 0);
        assert!(xml.contains("<false/>"));
    }

    #[test]
    fn rotate_is_noop_when_under_limit() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("app.log");
        std::fs::write(&log, b"tiny").unwrap();
        rotate_log_at(&log, 1, 3);
        assert!(log.exists(), "small log should not be rotated");
        assert!(!dir.path().join("app.log.1").exists());
    }

    #[test]
    fn rotate_shifts_rolls_and_drops_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("app.log");
        // 2 MiB of bytes — over the 1 MiB threshold.
        std::fs::write(&log, vec![b'x'; 2 * 1024 * 1024]).unwrap();
        std::fs::write(dir.path().join("app.log.1"), b"old-1").unwrap();
        std::fs::write(dir.path().join("app.log.2"), b"old-2").unwrap();
        std::fs::write(dir.path().join("app.log.3"), b"old-3").unwrap();

        rotate_log_at(&log, 1, 3);

        assert!(!log.exists(), "current log should be moved away");
        assert_eq!(
            std::fs::read(dir.path().join("app.log.2")).unwrap(),
            b"old-1",
            ".1 should have shifted to .2"
        );
        assert_eq!(
            std::fs::read(dir.path().join("app.log.3")).unwrap(),
            b"old-2",
            ".2 should have shifted to .3"
        );
        assert!(
            !dir.path().join("app.log.4").exists(),
            "should never exceed configured roll count"
        );
    }

    #[test]
    fn rotate_handles_missing_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("never-existed.log");
        // Must not panic or error-out.
        rotate_log_at(&log, 1, 3);
    }

    #[test]
    fn plist_path_is_under_home_library() {
        let path = plist_path();
        let s = path.to_string_lossy();
        assert!(s.contains("Library/LaunchAgents"));
        assert!(s.ends_with("com.user.iphone-backup.plist"));
    }
}
