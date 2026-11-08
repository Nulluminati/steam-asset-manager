//! App settings, stored as TOML in the user's config folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The OpenAI image model the app uses when the config names none.
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-image-2";

const FILE_NAME: &str = "config.toml";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot access {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("cannot write the config as TOML: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// A Steam root that wins over the discovered one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_root: Option<PathBuf>,
    pub openai_model: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            steam_root: None,
            openai_model: DEFAULT_OPENAI_MODEL.to_string(),
        }
    }
}

/// The config file in the OS config folder, for example
/// `~/.config/steam-asset-manager/config.toml` on Linux.
pub fn default_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "steam-asset-manager")
        .map(|dirs| dirs.config_dir().join(FILE_NAME))
}

impl Config {
    /// Reads the config file. A missing file gives the defaults, and a
    /// missing field gives its default value.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Config::default());
            }
            Err(source) => {
                return Err(ConfigError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Writes the config file. Creates the parent folder if it is missing.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string(self)?;
        let io_error = |source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io_error)?;
        }
        std::fs::write(path, text).map_err(io_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = TempDir::new().unwrap();
        let config = Config::load(&dir.path().join(FILE_NAME)).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.steam_root, None);
        assert_eq!(config.openai_model, DEFAULT_OPENAI_MODEL);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let config = Config {
            steam_root: Some(PathBuf::from("/games/Steam")),
            openai_model: "gpt-image-test".to_string(),
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
    }

    #[test]
    fn save_creates_missing_folders() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a/b").join(FILE_NAME);
        Config::default().save(&path).unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn save_without_steam_root_omits_the_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        Config::default().save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("steam_root"), "{text}");
        assert_eq!(Config::load(&path).unwrap(), Config::default());
    }

    #[test]
    fn missing_fields_get_defaults() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, "steam_root = \"/games/Steam\"\n").unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.steam_root, Some(PathBuf::from("/games/Steam")));
        assert_eq!(config.openai_model, DEFAULT_OPENAI_MODEL);
    }

    #[test]
    fn invalid_toml_is_a_parse_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, "openai_model = \n").unwrap();
        let err = Config::load(&path).unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }), "{err}");
    }

    #[test]
    fn unreadable_path_is_an_io_error() {
        let dir = TempDir::new().unwrap();
        // A folder in place of the file cannot be read as text.
        let path = dir.path().join(FILE_NAME);
        fs::create_dir(&path).unwrap();
        let err = Config::load(&path).unwrap_err();
        assert!(matches!(err, ConfigError::Io { .. }), "{err}");
    }

    #[test]
    fn default_path_ends_with_app_folder_and_file_name() {
        let path = default_path().unwrap();
        assert_eq!(path.file_name().unwrap(), FILE_NAME);
        let folder = path.parent().unwrap().to_string_lossy().to_lowercase();
        assert!(folder.contains("steam-asset-manager"), "{folder}");
    }
}
