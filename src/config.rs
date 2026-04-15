use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Where backups are stored (e.g. ~/Backups/iOS or /Volumes/my-drive).
    pub backup_path: String,
    /// Hour (0–23) at which launchd runs the daily backup.
    #[serde(default = "default_hour")]
    pub schedule_hour: u8,
    /// Minute (0–59) at which launchd runs the daily backup.
    #[serde(default = "default_minute")]
    pub schedule_minute: u8,
}

fn default_hour() -> u8 {
    2
}
fn default_minute() -> u8 {
    0
}

impl Default for Config {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            backup_path: home.join("Backups/iOS").to_string_lossy().into_owned(),
            schedule_hour: default_hour(),
            schedule_minute: default_minute(),
        }
    }
}

impl Config {
    pub fn config_path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("cannot locate home directory")?;
        Ok(home.join(".config/iphone-backup/config.toml"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            let config = Self::default();
            config.save()?;
            return Ok(config);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let c = Config::default();
        assert!(c.backup_path.contains("Backups/iOS"));
        assert_eq!(c.schedule_hour, 2);
        assert_eq!(c.schedule_minute, 0);
    }

    #[test]
    fn roundtrip_toml() {
        let c = Config {
            backup_path: "/mnt/backup".into(),
            schedule_hour: 3,
            schedule_minute: 30,
        };
        let s = toml::to_string_pretty(&c).unwrap();
        let c2: Config = toml::from_str(&s).unwrap();
        assert_eq!(c2.backup_path, c.backup_path);
        assert_eq!(c2.schedule_hour, c.schedule_hour);
        assert_eq!(c2.schedule_minute, c.schedule_minute);
    }

    #[test]
    fn parse_toml_with_defaults() {
        let s = r#"backup_path = "/tmp/test""#;
        let c: Config = toml::from_str(s).unwrap();
        assert_eq!(c.backup_path, "/tmp/test");
        assert_eq!(c.schedule_hour, 2);
        assert_eq!(c.schedule_minute, 0);
    }

    #[test]
    fn parse_full_toml() {
        let s = r#"
backup_path = "/Volumes/drive/iOS"
schedule_hour = 5
schedule_minute = 45
"#;
        let c: Config = toml::from_str(s).unwrap();
        assert_eq!(c.backup_path, "/Volumes/drive/iOS");
        assert_eq!(c.schedule_hour, 5);
        assert_eq!(c.schedule_minute, 45);
    }

    #[test]
    fn backup_path_returns_pathbuf() {
        let c = Config {
            backup_path: "/tmp/backups".into(),
            schedule_hour: 0,
            schedule_minute: 0,
        };
        assert_eq!(c.backup_path(), PathBuf::from("/tmp/backups"));
    }

    #[test]
    fn status_dir_is_backup_path_plus_status() {
        let c = Config {
            backup_path: "/tmp/backups".into(),
            schedule_hour: 0,
            schedule_minute: 0,
        };
        assert_eq!(c.status_dir(), PathBuf::from("/tmp/backups/.status"));
    }

    #[test]
    fn log_path_is_status_dir_plus_log() {
        let c = Config {
            backup_path: "/tmp/backups".into(),
            schedule_hour: 0,
            schedule_minute: 0,
        };
        assert_eq!(
            c.log_path(),
            PathBuf::from("/tmp/backups/.status/ibackup.log")
        );
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("iphone-backup/config.toml");

        let c = Config {
            backup_path: "/custom/path".into(),
            schedule_hour: 14,
            schedule_minute: 30,
        };

        std::fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
        std::fs::write(&cfg_path, toml::to_string_pretty(&c).unwrap()).unwrap();

        let loaded_text = std::fs::read_to_string(&cfg_path).unwrap();
        let c2: Config = toml::from_str(&loaded_text).unwrap();
        assert_eq!(c2.backup_path, c.backup_path);
        assert_eq!(c2.schedule_hour, c.schedule_hour);
        assert_eq!(c2.schedule_minute, c.schedule_minute);
    }

    #[test]
    fn config_path_is_under_dotconfig() {
        let path = Config::config_path().unwrap();
        let s = path.to_string_lossy();
        assert!(s.contains(".config/iphone-backup/config.toml"), "got: {s}");
    }
}
