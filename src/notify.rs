//! macOS user-notification helper.
//!
//! Fires a native notification via `osascript`. Gated behind
//! [`running_under_launchd`] so we don't pop notifications during
//! interactive TUI use.

use std::process::{Command, Stdio};

/// True if this process was started by launchd. launchd sets
/// `XPC_SERVICE_NAME` on every agent/daemon it spawns.
pub fn running_under_launchd() -> bool {
    std::env::var_os("XPC_SERVICE_NAME").is_some()
}

/// Escape a string for safe interpolation into an AppleScript string literal.
fn escape_applescript(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}

/// Fire a macOS notification. Best-effort: errors are swallowed because a
/// failure here must not mask the real exit status of the backup.
#[cfg(target_os = "macos")]
pub fn display_notification(title: &str, body: &str) {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape_applescript(body),
        escape_applescript(title),
    );
    let _ = Command::new("osascript")
        .args(["-e", &script])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(target_os = "macos"))]
pub fn display_notification(_title: &str, _body: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_handles_quotes_and_backslashes() {
        assert_eq!(escape_applescript("plain"), "plain");
        assert_eq!(escape_applescript("a\"b"), "a\\\"b");
        assert_eq!(escape_applescript("a\\b"), "a\\\\b");
        assert_eq!(escape_applescript("\"\\"), "\\\"\\\\");
    }

    #[test]
    fn running_under_launchd_respects_env() {
        // This test is inherently process-global; guard against interference
        // by explicitly setting the var for the duration of the check.
        // SAFETY: std::env::set_var is marked unsafe as of Rust 1.84 because
        // of TOCTOU with getenv in other threads; tests run single-threaded
        // for this assertion path.
        unsafe {
            std::env::remove_var("XPC_SERVICE_NAME");
            assert!(!running_under_launchd());
            std::env::set_var("XPC_SERVICE_NAME", "com.example.test");
            assert!(running_under_launchd());
            std::env::remove_var("XPC_SERVICE_NAME");
        }
    }
}
