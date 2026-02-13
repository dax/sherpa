use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::cli_detection::AiCli;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SherpaConfig {
    #[serde(default)]
    pub ai: AiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiConfig {
    pub selected_cli: Option<AiCli>,
}

impl SherpaConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path).map_err(ConfigError::Io)?;
        toml::from_str(&content).map_err(ConfigError::Parse)
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(ConfigError::Io)?;
        }
        let content = toml::to_string_pretty(self).map_err(ConfigError::Serialize)?;

        let tmp_path = path.with_extension("toml.tmp");
        let mut file = fs::File::create(&tmp_path).map_err(ConfigError::Io)?;
        file.write_all(content.as_bytes())
            .map_err(ConfigError::Io)?;
        file.sync_all().map_err(ConfigError::Io)?;
        fs::rename(&tmp_path, path).map_err(ConfigError::Io)?;

        Ok(())
    }

    pub fn config_dir() -> Result<PathBuf, ConfigError> {
        dirs::home_dir()
            .map(|h| h.join(".sherpa"))
            .ok_or(ConfigError::NoHomeDir)
    }

    pub fn default_path() -> Result<PathBuf, ConfigError> {
        Self::config_dir().map(|d| d.join("config.toml"))
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(toml::de::Error),
    Serialize(toml::ser::Error),
    NoHomeDir,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "config I/O error: {e}"),
            Self::Parse(e) => write!(f, "config parse error: {e}"),
            Self::Serialize(e) => write!(f, "config serialize error: {e}"),
            Self::NoHomeDir => write!(f, "could not determine home directory"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_default_config() {
        let config = SherpaConfig::default();
        assert!(config.ai.selected_cli.is_none());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("sherpa_test_config");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");

        let mut config = SherpaConfig::default();
        config.ai.selected_cli = Some(AiCli::Claude);

        config.save(&path).unwrap();

        let mut content = String::new();
        fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert!(content.contains("claude"));

        let loaded = SherpaConfig::load(&path).unwrap();
        assert_eq!(loaded.ai.selected_cli, Some(AiCli::Claude));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_nonexistent_returns_default() {
        let path = PathBuf::from("/tmp/sherpa_nonexistent_test/config.toml");
        let config = SherpaConfig::load(&path).unwrap();
        assert!(config.ai.selected_cli.is_none());
    }

    #[test]
    fn test_atomic_write_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("sherpa_test_atomic");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("config.toml");

        let config = SherpaConfig::default();
        config.save(&path).unwrap();

        assert!(path.exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
