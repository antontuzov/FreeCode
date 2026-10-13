//! Settings schema + persistence for SmoLLM Studio.
//!
//! One `Settings` document is the single source of truth for the desktop app, the engine launcher
//! and the agent loop. It is written atomically as JSON so a crash mid-save cannot leave a
//! half-written config, and unknown or missing keys fall back to defaults so an older file always
//! loads in a newer build.

pub mod agent;
pub mod appearance;
pub mod sampling;
pub mod voice;

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use agent::{
    ApprovalMode, Approvals, Context, LocalModel, PackingChoice, PermissionClass, Privacy,
};
pub use appearance::{
    Appearance, ContentWidth, FileIconSet, FontStyle, InterfaceStyle, Language, TerminalTheme,
    ThemeMode,
};
pub use sampling::{GenerationBudget, ReasoningEffort, SamplingProfile};
pub use voice::{InputDevice, Shortcut, Voice};

/// Default catalog entry — see `catalog/models.toml`.
///
/// The 1.7B rather than Bonsai 2 27B: it is the ternary generation this build's engine can load from
/// the pack the demo pins, it needs 463 MB instead of 5.9 GB, and its 32k window is what the launcher
/// can fill on a 16 GB laptop. The 27B stays in the catalog for a host that has it.
pub const DEFAULT_MODEL_ID: &str = "ternary-bonsai-1-7b";

/// Bump when a stored document must be migrated rather than silently defaulted.
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

pub const APP_CONFIG_DIR: &str = "smollm-studio";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read settings at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse settings at {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("could not serialize settings for {path}: {source}")]
    Serialize {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("could not write settings at {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not replace settings file at {path}: {source}")]
    Replace {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub schema_version: u32,
    pub appearance: Appearance,
    pub sampling: SamplingSettings,
    pub voice: Voice,
    pub approvals: Approvals,
    pub privacy: Privacy,
    pub context: Context,
    pub local_model: LocalModel,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            appearance: Appearance::default(),
            sampling: SamplingSettings::default(),
            voice: Voice::default(),
            approvals: Approvals::default(),
            privacy: Privacy::default(),
            context: Context::default(),
            local_model: LocalModel::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SamplingSettings {
    pub thinking: SamplingProfile,
    pub instruct: SamplingProfile,
    pub effort: ReasoningEffort,
    pub budget: GenerationBudget,
    pub system_prompt: String,
    /// Use the instruct profile instead of the thinking one. Off by default: the model's
    /// intelligence is measured in thinking mode.
    pub non_thinking_mode: bool,
}

impl Default for SamplingSettings {
    fn default() -> Self {
        Self {
            thinking: SamplingProfile::THINKING,
            instruct: SamplingProfile::INSTRUCT,
            effort: ReasoningEffort::default(),
            budget: GenerationBudget::default(),
            system_prompt: "You are a helpful assistant".into(),
            non_thinking_mode: false,
        }
    }
}

impl SamplingSettings {
    pub fn active(&self) -> SamplingProfile {
        if self.non_thinking_mode {
            self.instruct
        } else {
            self.thinking
        }
    }
}

impl Settings {
    /// `~/Library/Application Support/smollm-studio/settings.json` on macOS, the platform
    /// equivalent elsewhere.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join(APP_CONFIG_DIR).join("settings.json"))
    }

    /// Load, falling back to defaults for a missing file. A file that exists but cannot be parsed
    /// is returned as an error rather than silently replaced — losing a user's model directory or
    /// approvals list to a typo is worse than surfacing it.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw)
                .map(Self::migrated)
                .map_err(|source| ConfigError::Parse {
                    path: path.to_path_buf(),
                    source,
                }),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Atomic write: temp file in the same directory, fsync, then rename over the target.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(self).map_err(|source| ConfigError::Serialize {
            path: path.to_path_buf(),
            source,
        })?;
        {
            let mut file = std::fs::File::create(&tmp).map_err(|source| ConfigError::Write {
                path: tmp.clone(),
                source,
            })?;
            file.write_all(&body)
                .and_then(|()| file.sync_all())
                .map_err(|source| ConfigError::Write {
                    path: tmp.clone(),
                    source,
                })?;
        }
        std::fs::rename(&tmp, path).map_err(|source| ConfigError::Replace {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    }

    fn migrated(self) -> Self {
        // v0 documents predate the schema version field; serde defaults already filled them in.
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut s = Settings::default();
        s.appearance.interface_style = InterfaceStyle::Parchment;
        s.sampling.effort = ReasoningEffort::Medium;
        s.save(&path).unwrap();
        let back = Settings::load(&path).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn missing_file_yields_defaults_but_corrupt_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(matches!(
            Settings::load(&path),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn active_profile_follows_thinking_mode() {
        let s = SamplingSettings::default();
        assert_eq!(s.active(), SamplingProfile::THINKING);
        let s = SamplingSettings {
            non_thinking_mode: true,
            ..s
        };
        assert_eq!(s.active(), SamplingProfile::INSTRUCT);
    }

    #[test]
    fn no_temp_file_survives_a_successful_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        Settings::default().save(&path).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }
}
