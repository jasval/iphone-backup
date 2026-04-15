use std::io::{BufRead as _, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

/// Spawn an update in a background thread, streaming progress to `tx`.
/// Returns true if the update succeeded.
pub fn run(tx: Sender<String>) -> JoinHandle<bool> {
    std::thread::spawn(move || {
        let _ = tx.send("Checking for updates...".into());

        // Is this install managed by Homebrew?
        let brew_managed = Command::new("brew")
            .args(["list", "--formula", "iphone-backup"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if brew_managed {
            let _ = tx.send("Found Homebrew install — running brew upgrade iphone-backup".into());
            stream_command(
                Command::new("brew")
                    .args(["upgrade", "iphone-backup"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped()),
                &tx,
            )
        } else {
            // Try to find the source repo relative to the running binary.
            // Layout when built from source: repo/target/release/iphone-backup
            let source_dir = std::env::current_exe()
                .ok()
                .and_then(|p| {
                    // Walk up: release/ → target/ → repo root
                    p.parent()?.parent()?.parent().map(|p| p.to_path_buf())
                })
                .filter(|d| d.join("Cargo.toml").exists() && d.join(".git").exists());

            if let Some(dir) = source_dir {
                let _ = tx.send(format!("Found source repo at {}", dir.display()));
                let _ = tx.send("Running git pull...".into());

                let pull_ok = stream_command(
                    Command::new("git")
                        .args(["pull"])
                        .current_dir(&dir)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped()),
                    &tx,
                );
                if !pull_ok {
                    let _ = tx.send("✗ git pull failed.".into());
                    return false;
                }

                let tag_ok = stream_command(
                    Command::new("git")
                        .args(["verify-tag", "--raw"])
                        .current_dir(&dir)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped()),
                    &tx,
                );
                if !tag_ok {
                    let _ = tx.send(
                        "⚠ Tag signature could not be verified. Proceeding without verification."
                            .into(),
                    );
                } else {
                    let _ = tx.send("✓ Tag signature verified.".into());
                }

                let _ = tx.send("Building (cargo build --release)...".into());
                let build_ok = stream_command(
                    Command::new("cargo")
                        .args(["build", "--release"])
                        .current_dir(&dir)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped()),
                    &tx,
                );
                if !build_ok {
                    let _ = tx.send("✗ Build failed.".into());
                    return false;
                }

                let bin_src = dir.join("target/release/iphone-backup");
                let bin_dst = std::env::current_exe()
                    .unwrap_or_else(|_| std::path::PathBuf::from("/usr/local/bin/iphone-backup"));

                let _ = tx.send(format!(
                    "Installing {} → {}",
                    bin_src.display(),
                    bin_dst.display()
                ));
                let cp_ok = if bin_dst.parent().map(is_writable).unwrap_or(false) {
                    std::fs::copy(&bin_src, &bin_dst).is_ok()
                } else {
                    Command::new("sudo")
                        .args([
                            "cp",
                            bin_src.to_str().unwrap_or(""),
                            bin_dst.to_str().unwrap_or(""),
                        ])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                };

                if cp_ok {
                    let _ = tx.send(
                        "✓ Update complete. Restart iphone-backup to use the new version.".into(),
                    );
                    true
                } else {
                    let _ = tx.send(
                        "✗ Copy failed — try: sudo cp target/release/iphone-backup /usr/local/bin/"
                            .into(),
                    );
                    false
                }
            } else {
                let _ = tx.send("Cannot determine install method.".into());
                let _ = tx.send("To update manually:".into());
                let _ = tx.send("  Homebrew:    brew upgrade iphone-backup".into());
                let _ = tx.send("  From source: git pull && cargo build --release && sudo cp target/release/iphone-backup /usr/local/bin/".into());
                false
            }
        }
    })
}

fn stream_command(cmd: &mut Command, tx: &Sender<String>) -> bool {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(format!("ERROR: {e}"));
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

    child.wait().map(|s| s.success()).unwrap_or(false)
}

fn is_writable(path: &std::path::Path) -> bool {
    path.metadata()
        .map(|m| !m.permissions().readonly())
        .unwrap_or(false)
}
