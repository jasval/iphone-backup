use chrono::{DateTime, Utc};
use std::io::{BufRead as _, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

#[derive(Debug, Clone)]
pub struct BackupEntry {
    pub path: PathBuf,
    pub name: String,
    pub size: String,
    pub last_run: String,
}

pub fn list_backups(backup_path: &Path) -> Vec<BackupEntry> {
    let mut entries = Vec::new();
    let Ok(dir) = std::fs::read_dir(backup_path) else {
        return entries;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name.starts_with('.') {
            continue;
        }
        let size = dir_size(&path);
        let last_run = dir_modified(&path);
        entries.push(BackupEntry {
            path,
            name,
            size,
            last_run,
        });
    }
    entries.sort_by(|a, b| b.last_run.cmp(&a.last_run));
    entries
}

/// Spawn a thread that runs `idevicebackup2 -u <udid> restore <backup_dir>`,
/// streaming stdout/stderr to `tx`. Returns the JoinHandle.
pub fn run(udid: &str, backup_dir: &Path, tx: Sender<String>) -> JoinHandle<bool> {
    let udid = udid.to_string();
    let backup_dir = backup_dir.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(format!(
            "Starting restore from {} to device {}...",
            backup_dir.display(),
            udid
        ));
        let mut child = match Command::new("idevicebackup2")
            .args(["-u", &udid, "restore", backup_dir.to_str().unwrap_or("")])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(format!("ERROR: idevicebackup2 not found ({e}). Install with: brew install libimobiledevice"));
                return false;
            }
        };

        let tx2 = tx.clone();
        if let Some(stderr) = child.stderr.take() {
            let stderr_thread = std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let _ = tx2.send(line);
                }
            });
            let _ = stderr_thread.join();
        }

        if let Some(stdout) = child.stdout.take() {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        }

        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        if ok {
            let _ = tx.send("✓ Restore complete.".into());
        } else {
            let _ = tx.send("✗ Restore failed.".into());
        }
        ok
    })
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

fn dir_modified(path: &Path) -> String {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| {
            let dt: DateTime<Utc> = t.into();
            dt.to_rfc3339()
        })
        .unwrap_or_else(|_| "unknown".into())
}

/// Write a log line to a file handle (shared with backup.rs pattern)
#[allow(dead_code)]
pub fn log_to_file(msg: &str, tx: &Sender<String>, log_path: &Path) {
    let line = format!("[{}] {}", chrono::Local::now().format("%H:%M:%S"), msg);
    let _ = tx.send(line.clone());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(f, "{}", line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_backups_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let entries = list_backups(dir.path());
        assert!(entries.is_empty());
    }

    #[test]
    fn list_backups_skips_dot_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".status")).unwrap();
        std::fs::create_dir(dir.path().join(".hidden")).unwrap();
        let entries = list_backups(dir.path());
        assert!(entries.is_empty());
    }

    #[test]
    fn list_backups_skips_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readme.txt"), "hello").unwrap();
        let entries = list_backups(dir.path());
        assert!(entries.is_empty());
    }

    #[test]
    fn list_backups_finds_backup_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Phone")).unwrap();
        std::fs::create_dir(dir.path().join("iPad")).unwrap();
        let entries = list_backups(dir.path());
        assert_eq!(entries.len(), 2);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"Phone"));
        assert!(names.contains(&"iPad"));
    }

    #[test]
    fn list_backups_includes_dot_dirs_but_not_regular() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".status")).unwrap();
        std::fs::create_dir(dir.path().join("Phone")).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "data").unwrap();
        let entries = list_backups(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Phone");
    }

    #[test]
    fn list_backups_nonexistent_dir() {
        let entries = list_backups(Path::new("/nonexistent/path/abc123"));
        assert!(entries.is_empty());
    }

    #[test]
    fn list_backups_entry_has_correct_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Phone")).unwrap();
        let entries = list_backups(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, dir.path().join("Phone"));
        assert_eq!(entries[0].name, "Phone");
    }
}
