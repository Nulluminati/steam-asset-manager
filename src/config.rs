//! App settings, stored as TOML in the user's config folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The OpenAI image model the app uses when the config names none.
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-image-2.6-sunburst";

/// The most images one Generate or Modify request makes. The candidate
/// grid has room for this many.
pub const MAX_IMAGES_PER_REQUEST: u8 = 4;

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

/// Where Generate and Modify get their images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageSource {
    /// The OpenAI image API, paid with the OpenAI API key.
    #[default]
    #[serde(rename = "openai_api")]
    OpenAiApi,
    /// The Codex CLI on the user's ChatGPT plan.
    Codex,
}

impl ImageSource {
    pub const ALL: [ImageSource; 2] = [ImageSource::OpenAiApi, ImageSource::Codex];

    /// The name the settings dialog shows.
    pub fn label(self) -> &'static str {
        match self {
            ImageSource::OpenAiApi => "OpenAI API",
            ImageSource::Codex => "Codex (ChatGPT plan)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// A Steam root that wins over the discovered one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_root: Option<PathBuf>,
    pub openai_model: String,
    pub image_source: ImageSource,
    /// How many images Generate and Modify make, 1 to
    /// [`MAX_IMAGES_PER_REQUEST`]. A stored value outside that range moves
    /// to the nearest edge.
    #[serde(deserialize_with = "clamped_image_count")]
    pub images_per_request: u8,
}

/// Reads any integer and moves it into 1 to [`MAX_IMAGES_PER_REQUEST`].
fn clamped_image_count<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = i64::deserialize(deserializer)?;
    let clamped = value.clamp(1, i64::from(MAX_IMAGES_PER_REQUEST));
    Ok(u8::try_from(clamped).expect("the clamped count fits in u8"))
}

impl Default for Config {
    fn default() -> Self {
        Config {
            steam_root: None,
            openai_model: DEFAULT_OPENAI_MODEL.to_string(),
            image_source: ImageSource::default(),
            images_per_request: 1,
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
    fn default_model_is_gpt_image_2_6_sunburst() {
        assert_eq!(DEFAULT_OPENAI_MODEL, "gpt-image-2.6-sunburst");
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let config = Config {
            steam_root: Some(PathBuf::from("/games/Steam")),
            openai_model: "gpt-image-test".to_string(),
            image_source: ImageSource::Codex,
            images_per_request: 2,
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
    fn image_source_defaults_to_the_openai_api() {
        assert_eq!(Config::default().image_source, ImageSource::OpenAiApi);
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, "openai_model = \"gpt-image-2\"\n").unwrap();
        assert_eq!(
            Config::load(&path).unwrap().image_source,
            ImageSource::OpenAiApi
        );
    }

    #[test]
    fn image_source_codex_is_saved_as_codex() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let config = Config {
            image_source: ImageSource::Codex,
            ..Config::default()
        };
        config.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("image_source = \"codex\""), "{text}");
        assert_eq!(Config::load(&path).unwrap(), config);
    }

    #[test]
    fn image_source_api_is_saved_as_openai_api() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        Config::default().save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("image_source = \"openai_api\""), "{text}");
    }

    #[test]
    fn image_source_labels() {
        assert_eq!(ImageSource::OpenAiApi.label(), "OpenAI API");
        assert_eq!(ImageSource::Codex.label(), "Codex (ChatGPT plan)");
    }

    #[test]
    fn images_per_request_defaults_to_one() {
        assert_eq!(Config::default().images_per_request, 1);
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, "openai_model = \"gpt-image-2\"\n").unwrap();
        assert_eq!(Config::load(&path).unwrap().images_per_request, 1);
    }

    #[test]
    fn images_per_request_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let config = Config {
            images_per_request: 3,
            ..Config::default()
        };
        config.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("images_per_request = 3"), "{text}");
        assert_eq!(Config::load(&path).unwrap(), config);
    }

    #[test]
    fn images_per_request_outside_the_range_is_clamped() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        for (stored, expected) in [("0", 1), ("9", 4), ("-5", 1), ("1000", 4), ("2", 2)] {
            fs::write(&path, format!("images_per_request = {stored}\n")).unwrap();
            assert_eq!(
                Config::load(&path).unwrap().images_per_request,
                expected,
                "stored {stored}"
            );
        }
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
