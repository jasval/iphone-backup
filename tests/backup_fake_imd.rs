//! End-to-end coverage of the `iphone-backup backup` subcommand.
//!
//! The test runs the real binary with `PATH` and `HOME` redirected so that:
//!   * `idevice_id`, `ideviceinfo`, `idevicebackup2`, `du`, etc. resolve to
//!     POSIX-shell shims under our control;
//!   * `Config::load()` reads the test's TOML instead of the user's real one.
//!
//! Two scenarios are exercised: a clean success run, and a run that hits the
//! configured timeout. These together exercise the spawn + drain + reap +
//! status-write paths end to end.

use assert_cmd::Command;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn write_shim(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Build a temp dir that looks like a fresh `$HOME` plus a `bin/` directory
/// of shims. Returns `(home, backup_path, bin_dir)`.
fn make_world(extra_config: &str, ibackup2_body: &str) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let backup_path = home.path().join("Backups/iOS");
    fs::create_dir_all(&backup_path).unwrap();

    let cfg_dir = home.path().join(".config/iphone-backup");
    fs::create_dir_all(&cfg_dir).unwrap();
    fs::write(
        cfg_dir.join("config.toml"),
        format!(
            r#"backup_path = "{}"
schedule_hour = 2
schedule_minute = 0
min_free_gb = 0
notify_on_failure = false
{extra_config}
"#,
            backup_path.display()
        ),
    )
    .unwrap();

    let bin = home.path().join("bin");
    fs::create_dir_all(&bin).unwrap();

    // idevice_id: prints one fake UDID for `--list`, nothing for network.
    write_shim(
        &bin,
        "idevice_id",
        r#"case "$1" in
  --list) echo "FAKEUDID0001" ;;
  --network) echo "" ;;
  *) echo "" ;;
esac"#,
    );

    // ideviceinfo: canned device info (and disk_usage hits the same path).
    write_shim(
        &bin,
        "ideviceinfo",
        r#"echo "DeviceName: TestPhone"
echo "ProductType: iPhone15,2"
echo "ProductVersion: 18.1"
echo "AmountDataUsed: 1000000""#,
    );

    write_shim(&bin, "idevicebackup2", ibackup2_body);

    // du -sh <path>: emit a size + the path so dir_size's tokeniser is happy.
    write_shim(&bin, "du", r#"echo "1.0M	$2""#);

    (home, backup_path, bin)
}

fn run_backup(home: &Path, bin: &Path) -> assert_cmd::assert::Assert {
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::cargo_bin("iphone-backup")
        .unwrap()
        .env("HOME", home)
        .env("PATH", path)
        .env_remove("XPC_SERVICE_NAME")
        .arg("backup")
        .assert()
}

#[test]
fn fake_backup_succeeds_and_writes_status() {
    // Shim writes a Manifest.db big enough (>= 4 KiB) to satisfy verify.rs.
    let ibackup2 = r#"if [ "$1" = "--version" ]; then
  echo "idevicebackup2 1.3.0"
  exit 0
fi
DEST=""
for a in "$@"; do DEST="$a"; done
mkdir -p "$DEST"
dd if=/dev/zero of="$DEST/Manifest.db" bs=1024 count=8 2>/dev/null
echo "Backup successful."
exit 0"#;

    let (home, backup, bin) = make_world("", ibackup2);

    run_backup(home.path(), &bin).success();

    let summary = fs::read_to_string(backup.join(".status/summary.json")).unwrap();
    assert!(summary.contains("\"status\": \"ok\""), "summary: {summary}");
    assert!(
        summary.contains("idevicebackup2 1.3.0"),
        "summary should record libimobiledevice version: {summary}"
    );

    let device = fs::read_to_string(backup.join(".status/TestPhone.json")).unwrap();
    assert!(device.contains("\"status\": \"success\""), "device: {device}");
    assert!(device.contains("verification"), "device: {device}");
}

#[test]
fn fake_backup_hits_timeout_and_marks_failure() {
    // Sleep longer than the 0-minute timeout so the deadline fires on the
    // first poll iteration.
    let ibackup2 = r#"if [ "$1" = "--version" ]; then
  echo "idevicebackup2 1.3.0"
  exit 0
fi
sleep 30"#;

    let (home, backup, bin) =
        make_world("backup_timeout_minutes = 0\n", ibackup2);

    run_backup(home.path(), &bin).failure();

    let summary = fs::read_to_string(backup.join(".status/summary.json")).unwrap();
    assert!(
        summary.contains("\"status\": \"partial_failure\""),
        "summary: {summary}"
    );

    let device = fs::read_to_string(backup.join(".status/TestPhone.json")).unwrap();
    assert!(device.contains("\"status\": \"failed\""), "device: {device}");
    assert!(
        device.contains("timeout"),
        "device JSON should record timeout reason: {device}"
    );
}
