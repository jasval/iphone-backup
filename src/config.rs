use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Where backups are stored. Can be a local path or an SMB mount point.
    pub backup_path: String,
}

impl Default for Config {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            backup_path: home.join("Backups/iOS").to_string_lossy().into_owned(),
        }
    }
}

impl Config {
    pub fn config_path() -> Result<PathBuf> {
        Ok(dirs::config_dir()
            .context("cannot locate config directory")?
            .join("iphone-backup")
            .join("config.toml"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).context("parsing config.toml")
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn backup_path(&self) -> PathBuf {
        PathBuf::from(&self.backup_path)
    }

    pub fn status_dir(&self) -> PathBuf {
        self.backup_path().join(".status")
    }

    pub fn log_path(&self) -> PathBuf {
        self.status_dir().join("ibackup.log")
    }
}
