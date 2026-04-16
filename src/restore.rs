use chrono::{DateTime, Utc};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use serde_json;

#[derive(Debug, Clone)]
pub struct BackupEntry {
    pub path: PathBuf,
    pub name: String,
    pub size: String,
    pub last_run: String,
}

/// Read from `reader` splitting on `\r` or `\n`, strip ANSI codes, and send
/// each non-empty line to `tx`.
fn drain_to_channel(reader: impl Read, tx: &Sender<String>) {
    let mut reader = BufReader::new(reader);
    let mut buf: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' || byte[0] == b'\r' {
                    if !buf.is_empty() {
                        let raw = String::from_utf8_lossy(&buf).to_string();
                        let _ = tx.send(crate::backup::strip_ansi(&raw));
                        buf.clear();
                    }
                } else {
                    buf.push(byte[0]);
                }
            }
            Err(_) => break,
        }
    }
    if !buf.is_empty() {
        let raw = String::from_utf8_lossy(&buf).to_string();
        let _ = tx.send(crate::backup::strip_ansi(&raw));
    }
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
        let stderr_thread = child.stderr.take().map(|stderr| {
            std::thread::spawn(move || {
                drain_to_channel(stderr, &tx2);
            })
        });

        if let Some(stdout) = child.stdout.take() {
            drain_to_channel(stdout, &tx);
        }

        if let Some(handle) = stderr_thread {
            let _ = handle.join();
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

/// Delete a backup directory and its associated status files.
/// Updates manifest.json to remove the device entry.
pub fn delete_backup(entry: &BackupEntry, status_dir: &Path) -> std::io::Result<()> {
    if entry.path.exists() {
        std::fs::remove_dir_all(&entry.path)?;
    }
    let status_file = status_dir.join(format!("{}.json", entry.name));
    if status_file.exists() {
        std::fs::remove_file(&status_file)?;
    }
    let manifest_path = status_dir.join("manifest.json");
    if manifest_path.exists() {
        if let Ok(text) = std::fs::read_to_string(&manifest_path) {
            if let Ok(mut manifest) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(devices) = manifest.get_mut("devices").and_then(|d| d.as_array_mut()) {
                    devices.retain(|v| v.as_str() != Some(&entry.name));
                    if let Ok(updated) = serde_json::to_string_pretty(&manifest) {
                        let _ = std::fs::write(&manifest_path, updated);
                    }
                }
            }
        }
    }
    Ok(())
}

fn dir_size(path: &Path) -> String {
    let mut total: u64 = 0;
    if let Ok(entries) = walkdir(path) {
        for entry in entries {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total += meta.len();
                }
            }
        }
    }
    format_bytes(total)
}

fn walkdir(path: &Path) -> std::io::Result<Vec<std::fs::DirEntry>> {
    let mut result = vec![];
    let mut stack = vec![std::fs::read_dir(path)?];
    while let Some(dir) = stack.last_mut() {
        match dir.next() {
            Some(Ok(entry)) => {
                let path = entry.path();
                if path.is_dir() {
                    if let Ok(rd) = std::fs::read_dir(&path) {
                        stack.push(rd);
                    }
                }
                result.push(entry);
            }
            Some(Err(_)) => continue,
            None => {
                stack.pop();
            }
        }
    }
    Ok(result)
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0}K", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
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

    #[test]
    fn delete_backup_removes_dir_and_status() {
        let dir = tempfile::tempdir().unwrap();
        let backup_dir = dir.path().join("Phone");
        std::fs::create_dir(&backup_dir).unwrap();
        std::fs::write(backup_dir.join("data.bin"), "test").unwrap();

        let status_dir = dir.path().join(".status");
        std::fs::create_dir(&status_dir).unwrap();
        std::fs::write(
            status_dir.join("Phone.json"),
            r#"{"name":"Phone","status":"ok"}"#,
        )
        .unwrap();
        std::fs::write(
            status_dir.join("manifest.json"),
            r#"{"devices":["Phone","iPad"]}"#,
        )
        .unwrap();

        let entry = BackupEntry {
            path: backup_dir.clone(),
            name: "Phone".into(),
            size: "1G".into(),
            last_run: "2025-01-01T00:00:00Z".into(),
        };
        delete_backup(&entry, &status_dir).unwrap();

        assert!(!backup_dir.exists());
        assert!(!status_dir.join("Phone.json").exists());

        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(status_dir.join("manifest.json")).unwrap(),
        )
        .unwrap();
        let devices = manifest["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].as_str().unwrap(), "iPad");
    }

    #[test]
    fn delete_backup_nonexistent_dir_ok() {
        let dir = tempfile::tempdir().unwrap();
        let status_dir = dir.path().join(".status");
        std::fs::create_dir(&status_dir).unwrap();
        let entry = BackupEntry {
            path: dir.path().join("Nonexistent"),
            name: "Nonexistent".into(),
            size: "0B".into(),
            last_run: "".into(),
        };
        assert!(delete_backup(&entry, &status_dir).is_ok());
    }
}
