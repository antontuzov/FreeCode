//! Voice settings: the controls on Settings › Voice, all of which save immediately.
//!
//! Nothing in this build records or plays speech; the document carries the preferences a later build
//! reads, and the page says so on every row rather than promising recording once the engine lands.

use serde::{Deserialize, Serialize};

/// Microphone selection. `Automatic` follows the current system device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InputDevice {
    #[default]
    Automatic,
    Named(String),
}

/// A key combination rendered as chips in the UI, e.g. `⌘ ⇧ L`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub key: String,
}

impl Default for Shortcut {
    fn default() -> Self {
        Self {
            cmd: true,
            shift: true,
            alt: false,
            ctrl: false,
            key: "l".into(),
        }
    }
}

impl Shortcut {
    pub fn display(&self) -> String {
        let mut out = String::new();
        if self.cmd {
            out.push('⌘');
        }
        if self.ctrl {
            out.push('⌃');
        }
        if self.alt {
            out.push('⌥');
        }
        if self.shift {
            out.push('⇧');
        }
        let mut key: Vec<char> = self.key.chars().collect();
        if let Some(first) = key.first_mut() {
            *first = first.to_ascii_uppercase();
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&key.into_iter().collect::<String>());
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Voice {
    pub input_device: InputDevice,
    pub voice_input: bool,
    pub recognition_polish: bool,
    pub voiceprint_recognition: bool,
    /// DJI Mic Mini 2 collaboration kit: press once to record, again to stop and send.
    pub send_automatically: bool,
    pub common_word_corrections: Vec<String>,
    pub live_voice_shortcut: Shortcut,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            input_device: InputDevice::Automatic,
            // Every toggle in the Voice screenshot is on.
            voice_input: true,
            recognition_polish: true,
            voiceprint_recognition: true,
            send_automatically: true,
            common_word_corrections: Vec::new(),
            live_voice_shortcut: Shortcut::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_toggles_are_on_by_default_as_in_the_screenshot() {
        let v = Voice::default();
        assert!(
            v.voice_input
                && v.recognition_polish
                && v.voiceprint_recognition
                && v.send_automatically
        );
        assert_eq!(v.input_device, InputDevice::Automatic);
    }

    #[test]
    fn shortcut_renders_modifier_chips() {
        assert_eq!(Shortcut::default().display(), "⌘⇧ L");
    }
}
