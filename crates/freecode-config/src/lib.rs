//! Settings schema + persistence for FreeCode.
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
    ApprovalMode, Approvals, Context, LocalModel, ModelSource, PackingChoice, PermissionClass,
    Privacy, RemoteProvider,
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
pub const SETTINGS_SCHEMA_VERSION: u32 = 2;

/// The system turn every conversation starts with unless the user has written their own.
///
/// A small model given "You are a helpful assistant" answers like a chatbot: it describes changes
/// instead of making them, writes files it has not read, and stops before checking its work. The
/// contract below is written for the loop this app actually runs — read-only tools, sandboxed
/// writes, an optional shell — and every tool it names is one of the 31 the registry declares, so a
/// run is never told to reach for a call it was not offered.
///
/// It is grouped under five headings rather than numbered one to seventeen, because the section is
/// what a 1.7B model holds onto: the rules below it are the same handful, and a run that has lost the
/// list can still tell "change as little as is correct" from "check it".
pub const DEFAULT_SYSTEM_PROMPT: &str = "\
You are FreeCode, a coding agent working in the user's workspace on their own machine. Do what was asked; do not describe how it could be done.

SEE WHAT IS THERE FIRST
1. Never invent a file, path, symbol or API. read_file, list_dir, glob and grep_search answer what a name or a string is; git_status, git_diff and git_log answer what the repository has done.
2. When the answer is already in the conversation or on disk, get it there. Ask the user only for what only the user can decide.
3. terminal, run_tests, run_build, run_linter, web_search and fetch_url exist only in a run the user switched on. Where a tool is not offered, say the run cannot do it rather than pretending to.

CHANGE AS LITTLE AS IS CORRECT
4. For work with more than about three steps, call todo_write first: one short line per step, all pending except the one you are on. Call todo_write again whenever a step starts or finishes. One step is active at a time, and a step is done only once you have seen its result.
5. edit_file makes one precise replacement — old_string has to appear in the file exactly once, whitespace and all, and must differ from new_string. apply_patch carries a change that spans several files as one unified diff. write_file is only for a file that does not exist yet. Never rewrite whole a file you could have edited.
6. Group one file's changes into one call instead of a series of small ones. Match the code already there, and add every import a change needs.
7. Nothing here deletes, renames, commits or pushes. The three git calls read the repository and change no part of it, so a request to alter that is a thing to say you cannot do rather than a thing to work around with writes.

CHECK IT
8. After you write, run the project's own checks — run_tests, run_build, run_linter — and read what comes back. Fix what fails and run it again. Where no check was offered, say so plainly instead of claiming it passed.
9. When a call fails, read the reason and change your approach. Never repeat a call unchanged after it has failed.
10. Address every part of the request, then look once more for what is still missing and finish that too.
11. For a page or a site, write the files and say which one to open; the window shows what a run has built in its own browser panel. A server started with terminal keeps running after the round ends, so start one only when the task needs it.

SAY WHAT YOU DID
12. Never print a secret and never read one — the workspace's credential files are refused for a reason.
13. Do not name tools to the user; say what you did. Do not hand back a command for them to run when you can run it yourself.
14. Finish by stating what changed, in which files, and how you checked it. If you were blocked, name the block. Nothing counts as done without a tool result behind it.";

/// The contract that shipped before the one above.
///
/// A model told to "run the project's tests, build or linter when a shell is available" reaches for
/// those three by name and gets refused for the two it did not read about, so the tool surface is
/// now spelled out. This text is kept only so an install that never wrote its own prompt moves
/// forward rather than staying on a contract the loop no longer matches.
const SYSTEM_PROMPT_V2: &str = "\
You are FreeCode, a coding assistant running locally on the user's machine.

Your job is to make the change the user asked for, correctly, in their workspace — not to describe how it could be made.

Rules:
1. Read before you write. Never invent a file, path, symbol or API. Use the workspace tools to see what is actually there.
2. If a task has more than one step, call todo_write with a short plan (one step per line, all pending except the first), then work through it. Call todo_write again whenever a step starts or finishes: at most one step is active, and a step is done only when you have checked its result.
3. Make the smallest change that solves the problem, and match the style of the code around it.
4. After you edit, check your work: re-read the changed file, and run the project's tests, build or linter when a shell is available. If you cannot check something, say so.
5. If a tool fails, read the error and try a different approach. Never repeat a call that already failed the same way.
6. Stay inside the workspace. Never print secrets. Ask before anything destructive or irreversible.
7. Finish by saying what you changed, which files, and how you checked it. If you are blocked, say exactly what blocked you.

When a question needs no tools, answer it directly and briefly. Put code in fenced blocks with a language tag. Do not claim something is done unless a tool result shows it.";

/// Every prompt this app has shipped as its default, oldest first — including the plain chatbot line
/// the very first build stored.
///
/// `migrated` replaces any of these with [`DEFAULT_SYSTEM_PROMPT`] and nothing else. The point is that
/// the shipped contract can keep changing without installs being stranded on the version they
/// happened to download, while a prompt someone wrote stays theirs no matter how close it reads to
/// one of these.
const PREVIOUS_SYSTEM_PROMPTS: &[&str] = &["You are a helpful assistant", SYSTEM_PROMPT_V2];

pub const APP_CONFIG_DIR: &str = "freecode";

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
    pub project: Project,
    pub local_model: LocalModel,
    pub remote_provider: RemoteProvider,
    /// The free-model picker's choice, by registry id. `None` means no free model is selected.
    /// The answer order is free first, then the remote provider, then the local engine — the picker
    /// is the most recent intent, and picking a local model clears this field.
    pub free_model: Option<String>,
    pub onboarding: Onboarding,
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
            project: Project::default(),
            local_model: LocalModel::default(),
            remote_provider: RemoteProvider::default(),
            free_model: None,
            onboarding: Onboarding::default(),
        }
    }
}

/// Where the setup flow pointed a run, kept beside the folder itself.
///
/// `Project::root` cannot answer this: an empty root is both "set up in this app's folder" and
/// "never decided", and re-running setup would then have to guess which one the user meant. The
/// place is the flow's own answer, and `folder` is only recorded once a chooser returned a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingPlace {
    #[default]
    Undecided,
    ThisMac,
    ProjectFolder,
    Later,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Onboarding {
    /// Written when the user finishes the flow or chooses "set up later". A build that adds a step
    /// can raise the schema instead of re-opening a flow the user already completed.
    pub completed: bool,
    pub place: OnboardingPlace,
}

impl Default for Onboarding {
    fn default() -> Self {
        Self {
            completed: false,
            place: OnboardingPlace::Undecided,
        }
    }
}

/// The folder a run works in. Empty means the root the host resolved — the checkout in a development
/// build, the app's data directory once installed — which is the only place a build with no picker
/// could reach. A chosen folder is stored as the user named it and resolved by the host, never joined
/// against the process's working directory, because a dev run and a bundle start in different places.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Project {
    pub root: String,
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
            system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
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
    /// `~/Library/Application Support/freecode/settings.json` on macOS, the platform
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

    fn migrated(mut self) -> Self {
        // Each build ships a new contract, and an install that never edited it should be answering on
        // the one this loop matches. Replacing only an exact stored default is what makes that safe:
        // trimming is deliberate, because a document that never carried user words moves forward,
        // while a prompt someone wrote stays theirs even if it is short.
        let stored = self.sampling.system_prompt.trim();
        if PREVIOUS_SYSTEM_PROMPTS.contains(&stored) {
            self.sampling.system_prompt = DEFAULT_SYSTEM_PROMPT.to_string();
        }
        self.schema_version = SETTINGS_SCHEMA_VERSION;
        self
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
    fn the_default_system_prompt_is_a_coding_contract() {
        let prompt = SamplingSettings::default().system_prompt;
        assert_eq!(prompt, DEFAULT_SYSTEM_PROMPT);
        assert!(!prompt.contains("helpful assistant"));
        assert!(prompt.contains("Never invent a file, path, symbol or API"));
    }

    #[test]
    fn the_shipped_prompt_is_migrated_and_a_users_own_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        // Both defaults this app has ever shipped move forward, so an install is not stranded on the
        // contract the build it happened to download carried.
        for previous in PREVIOUS_SYSTEM_PROMPTS {
            let mut old = Settings {
                schema_version: 1,
                ..Settings::default()
            };
            old.sampling.system_prompt = (*previous).into();
            old.save(&path).unwrap();
            let back = Settings::load(&path).unwrap();
            assert_eq!(back.sampling.system_prompt, DEFAULT_SYSTEM_PROMPT);
            assert_eq!(back.schema_version, SETTINGS_SCHEMA_VERSION);
        }

        let mut mine = Settings::default();
        mine.sampling.system_prompt = "Answer like a pirate.".into();
        mine.save(&path).unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().sampling.system_prompt,
            "Answer like a pirate."
        );

        // A prompt that is the shipped one with a word added is no longer the shipped one. The contract
        // moved on, but this is a document someone edited, and editing it back is their press to make.
        mine.sampling.system_prompt = format!("{SYSTEM_PROMPT_V2}\nBe terse.");
        mine.save(&path).unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().sampling.system_prompt,
            format!("{SYSTEM_PROMPT_V2}\nBe terse.")
        );

        // A cleared prompt is a choice too: the run sends no system turn, and loading must not
        // quietly fill the field back in.
        mine.sampling.system_prompt = String::new();
        mine.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap().sampling.system_prompt, "");
    }

    #[test]
    fn no_temp_file_survives_a_successful_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        Settings::default().save(&path).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn onboarding_starts_open_and_remembers_where_it_pointed() {
        let s = Settings::default();
        assert!(!s.onboarding.completed);
        assert_eq!(s.onboarding.place, OnboardingPlace::Undecided);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut done = Settings::default();
        done.onboarding.completed = true;
        done.onboarding.place = OnboardingPlace::ProjectFolder;
        done.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap().onboarding, done.onboarding);

        // A document written before the flow existed loads as a first run rather than as a file the
        // app cannot read: the flow is the one thing a fresh install has not answered yet.
        let old: Settings =
            serde_json::from_value(serde_json::json!({ "freeModel": null })).unwrap();
        assert!(!old.onboarding.completed);
        assert_eq!(old.onboarding.place, OnboardingPlace::Undecided);
    }
}
