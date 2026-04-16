mod backup;
mod config;
mod device;
mod launchd;
mod pid;
mod restore;
mod status;
mod tui;
mod update;

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
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            // Print each log line to stderr so launchd captures it
            std::thread::spawn(move || {
                for line in rx {
                    eprintln!("{}", line);
                }
            });
            backup::run(&config.backup_path(), tx)?;
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
