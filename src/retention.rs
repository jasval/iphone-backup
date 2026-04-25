//! Archive + pruning policy for per-device backups.
//!
//! `idevicebackup2` overwrites the device directory in place, so the last
//! successful backup is the only "current" copy. To keep history we clone the
//! device directory after a successful run into
//! `<backup_root>/.archive/<device>/<timestamp>/`. On APFS the clone is
//! near-free — blocks are shared until the next run diverges.
//!
//! Pruning keeps an archive if it satisfies EITHER rule (count or age). When
//! both rules are unset, retention is disabled.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

/// Root directory where per-device archive timestamps live.
pub fn archive_root(backup_root: &Path, device_name: &str) -> PathBuf {
    backup_root.join(".archive").join(device_name)
}

/// Clone `source` into a fresh `<archive_root>/<timestamp>/` directory and
/// return its path. Uses `cp -cRp` so the copy is an APFS clone when the
/// source and destination share a volume.
pub fn archive(backup_root: &Path, device_name: &str, source: &Path) -> Result<PathBuf> {
    let parent = archive_root(backup_root, device_name);
    std::fs::create_dir_all(&parent)
        .with_context(|| format!("creating archive parent {}", parent.display()))?;
    let ts = chrono::Local::now().format("%Y%m%dT%H%M%S").to_string();
    let dest = parent.join(&ts);

    // Run `cp -cRp source dest`. -c requests a clonefile on APFS; -R is
    // recursive; -p preserves attributes. When `dest` does not exist, cp
    // creates it as a copy of `source`, which is what we want.
    let out = Command::new("cp")
        .arg("-cRp")
        .arg(source)
        .arg(&dest)
        .output()
        .context("invoking cp for archive clone")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        anyhow::bail!("cp -cRp failed: {err}");
    }
    Ok(dest)
}

/// Prune archives under `<backup_root>/.archive/<device_name>/` according to
/// the retention rules. Returns the list of directories that were removed.
///
/// - `keep_last`: keep the N newest archives.
/// - `keep_days`: keep archives whose mtime is within the last N days.
/// - An archive is kept if it satisfies EITHER rule. Passing both as `None`
///   means "retain everything" (no-op).
pub fn prune(
    backup_root: &Path,
    device_name: &str,
    keep_last: Option<u32>,
    keep_days: Option<u32>,
) -> Result<Vec<PathBuf>> {
    if keep_last.is_none() && keep_days.is_none() {
        return Ok(vec![]);
    }
    prune_from_now(backup_root, device_name, keep_last, keep_days, SystemTime::now())
}

/// Test-friendly variant of [`prune`] that takes an explicit "now".
pub(crate) fn prune_from_now(
    backup_root: &Path,
    device_name: &str,
    keep_last: Option<u32>,
    keep_days: Option<u32>,
    now: SystemTime,
) -> Result<Vec<PathBuf>> {
    let root = archive_root(backup_root, device_name);
    if !root.exists() {
        return Ok(vec![]);
    }

    // Collect (name, path, mtime) tuples. Entries without a readable mtime
    // are treated as infinitely old.
    let mut entries: Vec<(String, PathBuf, SystemTime)> = std::fs::read_dir(&root)
        .with_context(|| format!("reading archive dir {}", root.display()))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let ft = entry.file_type().ok()?;
            if !ft.is_dir() {
                return None;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let mtime = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((name, path, mtime))
        })
        .collect();

    // Newest-first by name (timestamps sort lexicographically) then by mtime
    // as a tie-breaker.
    entries.sort_by(|a, b| b.0.cmp(&a.0).then(b.2.cmp(&a.2)));

    let cutoff = keep_days.map(|d| now - Duration::from_secs(u64::from(d) * 86_400));

    let mut removed = Vec::new();
    for (idx, (_name, path, mtime)) in entries.iter().enumerate() {
        let keep_by_last = keep_last.is_some_and(|n| (idx as u32) < n);
        let keep_by_days = cutoff.is_some_and(|c| *mtime >= c);
        if keep_by_last || keep_by_days {
            continue;
        }
        std::fs::remove_dir_all(path)
            .with_context(|| format!("removing stale archive {}", path.display()))?;
        removed.push(path.clone());
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn mk_archive(root: &Path, device: &str, name: &str) -> PathBuf {
        let p = archive_root(root, device).join(name);
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("file"), b"x").unwrap();
        p
    }

    #[test]
    fn prune_returns_empty_when_rules_unset() {
        let dir = tempfile::tempdir().unwrap();
        mk_archive(dir.path(), "Phone", "20260101T000000");
        let removed = prune(dir.path(), "Phone", None, None).unwrap();
        assert!(removed.is_empty());
        assert!(archive_root(dir.path(), "Phone").join("20260101T000000").exists());
    }

    #[test]
    fn prune_handles_missing_archive_dir() {
        let dir = tempfile::tempdir().unwrap();
        let removed = prune(dir.path(), "Phone", Some(3), None).unwrap();
        assert!(removed.is_empty());
    }

    #[test]
    fn keep_last_retains_n_newest() {
        let dir = tempfile::tempdir().unwrap();
        for ts in &[
            "20260101T000000",
            "20260102T000000",
            "20260103T000000",
            "20260104T000000",
            "20260105T000000",
        ] {
            mk_archive(dir.path(), "Phone", ts);
        }
        let removed = prune(dir.path(), "Phone", Some(2), None).unwrap();
        assert_eq!(removed.len(), 3, "should remove the 3 oldest");

        let remaining: Vec<String> = fs::read_dir(archive_root(dir.path(), "Phone"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(remaining.contains(&"20260104T000000".to_string()));
        assert!(remaining.contains(&"20260105T000000".to_string()));
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn keep_days_is_applied_via_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let a = mk_archive(dir.path(), "Phone", "20260101T000000");
        let b = mk_archive(dir.path(), "Phone", "20260102T000000");

        // Set mtimes: a is 10 days old, b is 1 day old.
        let now = SystemTime::now();
        set_mtime(&a, now - Duration::from_secs(10 * 86_400));
        set_mtime(&b, now - Duration::from_secs(1 * 86_400));

        // keep_days=5 → a should be pruned, b kept.
        let removed = prune_from_now(dir.path(), "Phone", None, Some(5), now).unwrap();
        assert_eq!(removed.len(), 1);
        assert!(removed[0].ends_with("20260101T000000"));
        assert!(b.exists());
    }

    #[test]
    fn either_rule_keeps_an_archive() {
        // Rule: keep_last=1 OR keep_days=1. An archive that satisfies either
        // should be retained.
        let dir = tempfile::tempdir().unwrap();
        let a = mk_archive(dir.path(), "Phone", "20260101T000000"); // old, but newest
        let b = mk_archive(dir.path(), "Phone", "20251231T000000"); // old
        let c = mk_archive(dir.path(), "Phone", "20251225T000000"); // ancient

        let now = SystemTime::now();
        set_mtime(&a, now - Duration::from_secs(30 * 86_400));
        set_mtime(&b, now - Duration::from_secs(30 * 86_400));
        set_mtime(&c, now - Duration::from_secs(30 * 86_400));

        // Only keep_last=1 can save anything here; age rule helps nothing.
        let removed = prune_from_now(dir.path(), "Phone", Some(1), Some(1), now).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(a.exists(), "newest archive kept by keep_last=1");
        assert!(!b.exists());
        assert!(!c.exists());
    }

    fn set_mtime(path: &Path, when: SystemTime) {
        // `filetime` isn't a dep — use `utimes(2)` directly.
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let c = CString::new(path.as_os_str().as_bytes()).unwrap();
        let secs = when
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as libc::time_t;
        let tv = [
            libc::timeval { tv_sec: secs, tv_usec: 0 },
            libc::timeval { tv_sec: secs, tv_usec: 0 },
        ];
        // SAFETY: `utimes` reads the NUL-terminated path and a 2-element
        // `timeval` array. Both are valid for the duration of the call.
        let rc = unsafe { libc::utimes(c.as_ptr(), tv.as_ptr()) };
        assert_eq!(rc, 0, "utimes failed: {:?}", std::io::Error::last_os_error());
    }
}
