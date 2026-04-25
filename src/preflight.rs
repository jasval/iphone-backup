//! Pre-flight checks run before any backup starts.
//!
//! Motivates catching obvious failure modes (unmounted external drive, read-only
//! filesystem, not enough free space) before spawning `idevicebackup2` — which
//! will otherwise stall or produce a half-written backup.

use std::ffi::CString;
use std::path::Path;

#[derive(Debug)]
pub enum PreflightError {
    Missing(String),
    NotDirectory(String),
    NotWritable(String),
    LowDiskSpace { free_gb: u64, required_gb: u64 },
    Stat(String),
}

impl std::fmt::Display for PreflightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(p) => write!(
                f,
                "backup path '{p}' does not exist (external drive unmounted?)"
            ),
            Self::NotDirectory(p) => write!(f, "backup path '{p}' is not a directory"),
            Self::NotWritable(p) => write!(f, "backup path '{p}' is not writable"),
            Self::LowDiskSpace { free_gb, required_gb } => write!(
                f,
                "only {free_gb} GiB free on backup volume, need at least {required_gb} GiB"
            ),
            Self::Stat(msg) => write!(f, "cannot stat backup volume: {msg}"),
        }
    }
}

impl std::error::Error for PreflightError {}

/// Run all pre-flight checks. Returns `Ok(())` if the backup can proceed.
pub fn check_backup_path(path: &Path, min_free_gb: u64) -> Result<(), PreflightError> {
    let display = path.display().to_string();

    let meta = std::fs::metadata(path).map_err(|_| PreflightError::Missing(display.clone()))?;
    if !meta.is_dir() {
        return Err(PreflightError::NotDirectory(display));
    }

    // Writability test: create + delete a sentinel file. Cheaper than parsing
    // `st_mode` and the ACLs on macOS, and it catches read-only mounts.
    let sentinel = path.join(".iphone-backup-write-test");
    match std::fs::write(&sentinel, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&sentinel);
        }
        Err(_) => return Err(PreflightError::NotWritable(display)),
    }

    if min_free_gb > 0 {
        let free = free_bytes(path).map_err(PreflightError::Stat)?;
        let gib = 1024u64 * 1024 * 1024;
        let free_gb = free / gib;
        if free_gb < min_free_gb {
            return Err(PreflightError::LowDiskSpace {
                free_gb,
                required_gb: min_free_gb,
            });
        }
    }

    Ok(())
}

/// Bytes available to an unprivileged writer on the filesystem containing
/// `path`. Uses `statvfs(2)`.
fn free_bytes(path: &Path) -> Result<u64, String> {
    let c_path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| "path contains NUL".to_string())?;
    // SAFETY: `statvfs` reads through the C string pointer and writes into the
    // provided `statvfs` struct, both of which are valid for the duration of
    // the call. We check the return code before touching the struct.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // `f_frsize` is the fragment size; `f_bavail` the blocks available to
    // non-root. Both are u32/u64 depending on platform — widen before mul to
    // avoid overflow on large volumes.
    let frsize = u64::from(stat.f_frsize as u32);
    let bavail = stat.f_bavail as u64;
    Ok(frsize.saturating_mul(bavail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_path_fails() {
        let err = check_backup_path(Path::new("/nonexistent/definitely/not-here"), 0).unwrap_err();
        assert!(matches!(err, PreflightError::Missing(_)), "got: {err}");
    }

    #[test]
    fn file_path_fails_with_not_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, b"").unwrap();
        let err = check_backup_path(&file, 0).unwrap_err();
        assert!(matches!(err, PreflightError::NotDirectory(_)), "got: {err}");
    }

    #[test]
    fn writable_directory_with_no_space_requirement_passes() {
        let dir = tempfile::tempdir().unwrap();
        check_backup_path(dir.path(), 0).unwrap();
    }

    #[test]
    fn low_disk_space_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        // Ask for an absurd amount of space — no real volume has this.
        let err = check_backup_path(dir.path(), 1_000_000).unwrap_err();
        assert!(
            matches!(err, PreflightError::LowDiskSpace { .. }),
            "got: {err}"
        );
    }

    #[test]
    fn writability_sentinel_is_cleaned_up() {
        let dir = tempfile::tempdir().unwrap();
        check_backup_path(dir.path(), 0).unwrap();
        let sentinel = dir.path().join(".iphone-backup-write-test");
        assert!(!sentinel.exists(), "sentinel file should be removed");
    }

    #[test]
    fn non_writable_directory_is_rejected() {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        // Make the directory read-only. On most unices this blocks file
        // creation even for the owner (when not running as root).
        std::fs::set_permissions(dir.path(), Permissions::from_mode(0o555)).unwrap();
        let res = check_backup_path(dir.path(), 0);
        // Restore before asserting so tempdir cleanup works.
        std::fs::set_permissions(dir.path(), Permissions::from_mode(0o755)).unwrap();
        if nix_running_as_root() {
            // Root bypasses DAC write restrictions — skip the assertion.
            return;
        }
        assert!(
            matches!(res, Err(PreflightError::NotWritable(_))),
            "got: {res:?}"
        );
    }

    fn nix_running_as_root() -> bool {
        // SAFETY: libc::geteuid is a pure lookup of the current euid with no
        // side effects. Always safe to call.
        unsafe { libc::geteuid() == 0 }
    }
}
