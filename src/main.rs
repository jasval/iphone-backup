#[cfg(not(target_os = "macos"))]
compile_error!(
    "iphone-backup is macOS-only: it depends on launchd, ioreg, osascript, and Apple's plist tooling."
);

mod backup;
mod config;
mod device;
mod imd;
mod launchd;
mod notify;
mod pid;
mod preflight;
mod restore;
mod retention;
mod status;
mod tui;
mod update;
mod verify;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "iphone-backup",
    about = "Automated iPhone/iPad backup manager",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a backup and exit (called by launchd)
    Backup,
    /// Show current configuration and paths
    Config,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = config::Config::load()?;

    match cli.command {
        Some(Cmd::Backup) => {
            launchd::rotate_launchd_log(config.launchd_log_max_mb);
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            // Print each log line to stderr so launchd captures it
            std::thread::spawn(move || {
                for line in rx {
                    eprintln!("{line}");
                }
            });
            let outcome = backup::run(&config, &tx)?;
            if outcome.is_failure()
                && config.notify_on_failure
                && notify::running_under_launchd()
            {
                notify::display_notification(
                    "iPhone Backup failed",
                    &outcome.summary_line(),
                );
            }
            if outcome.is_failure() {
                std::process::exit(1);
            }
        }
        Some(Cmd::Config) => {
            println!("Config file: {}", config::Config::config_path()?.display());
            println!("{}", toml::to_string_pretty(&config)?);
        }
        None => {
            tui::run(config)?;
        }
    }

    Ok(())
}
