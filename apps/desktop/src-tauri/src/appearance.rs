//! The appearance slice of `Settings` the shell renders, and the only way the UI can change it.
//!
//! Two modes travel on the wire: `themeMode` is what the user chose (which may be `system`), while
//! `resolvedMode` is what the window should actually paint. Echoing only the resolved value back
//! into the setting would silently turn "System" into a fixed light or dark the first time the
//! panel saved.

use serde::{Deserialize, Serialize};
use smollm_config::{Appearance, InterfaceStyle, Settings, ThemeMode};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceView {
    pub theme_mode: ThemeMode,
    pub resolved_mode: ThemeMode,
    pub accent: InterfaceStyle,
    pub glass: bool,
    pub ui_font_size: u16,
    pub code_font_size: u16,
}

/// `System` asks the host; anything else is already an answer.
pub fn resolve(mode: ThemeMode, os_prefers_light: bool) -> ThemeMode {
    match mode {
        ThemeMode::System if os_prefers_light => ThemeMode::Light,
        ThemeMode::System => ThemeMode::Dark,
        other => other,
    }
}

pub fn view(appearance: &Appearance, os_prefers_light: bool) -> AppearanceView {
    AppearanceView {
        theme_mode: appearance.theme_mode,
        resolved_mode: resolve(appearance.theme_mode, os_prefers_light),
        accent: appearance.interface_style,
        glass: appearance.glass_effects,
        ui_font_size: appearance.ui_font_size_px,
        code_font_size: appearance.code_font_size_px,
    }
}

/// Write the five shell controls back into the full document. Every other `Appearance` field —
/// terminal theme, code theme, font style — is left exactly as it was, because those pages are not
/// wired yet and a save from the shell must not reset them.
pub fn apply(view: &AppearanceView, appearance: &mut Appearance) {
    appearance.theme_mode = view.theme_mode;
    appearance.interface_style = view.accent;
    appearance.glass_effects = view.glass;
    appearance.ui_font_size_px = view.ui_font_size;
    appearance.code_font_size_px = view.code_font_size;
}

/// Load the whole document, or default it when nothing is stored yet. A stored file that cannot be
/// parsed is an error, never a silent reset.
pub fn load_settings() -> Result<Settings, String> {
    match Settings::default_path() {
        None => Ok(Settings::default()),
        Some(path) => Settings::load(&path).map_err(|e| e.to_string()),
    }
}

pub fn save_settings(settings: &Settings) -> Result<(), String> {
    let path = Settings::default_path().ok_or("no writable config directory on this platform")?;
    settings.save(&path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_resolves_to_the_os_answer_without_forgetting_the_choice() {
        let appearance = Appearance {
            theme_mode: ThemeMode::System,
            ..Appearance::default()
        };
        let light = view(&appearance, true);
        assert_eq!(light.theme_mode, ThemeMode::System);
        assert_eq!(light.resolved_mode, ThemeMode::Light);
        assert_eq!(view(&appearance, false).resolved_mode, ThemeMode::Dark);
    }

    #[test]
    fn an_explicit_mode_is_passed_through() {
        let appearance = Appearance {
            theme_mode: ThemeMode::Light,
            ..Appearance::default()
        };
        assert_eq!(view(&appearance, true).resolved_mode, ThemeMode::Light);
        assert_eq!(view(&appearance, false).resolved_mode, ThemeMode::Light);
    }

    #[test]
    fn saving_the_shell_slice_leaves_the_unwired_controls_alone() {
        let mut settings = Settings::default();
        settings.appearance.font_style = smollm_config::FontStyle::Serif;
        settings.appearance.terminal_theme = smollm_config::TerminalTheme::Custom;
        settings.appearance.dark_code_theme = "vikia".into();
        settings.appearance.show_line_numbers = false;

        let mut current = settings.appearance.clone();
        let mut v = view(&current, false);
        v.accent = InterfaceStyle::Forest;
        v.glass = false;
        v.ui_font_size = 16;
        apply(&v, &mut current);

        assert_eq!(current.interface_style, InterfaceStyle::Forest);
        assert!(!current.glass_effects);
        assert_eq!(current.ui_font_size_px, 16);
        assert_eq!(current.font_style, settings.appearance.font_style);
        assert_eq!(current.terminal_theme, settings.appearance.terminal_theme);
        assert_eq!(current.dark_code_theme, settings.appearance.dark_code_theme);
        assert_eq!(
            current.show_line_numbers,
            settings.appearance.show_line_numbers
        );
    }

    #[test]
    fn the_wire_names_match_the_i18n_and_css_token_values() {
        let v = view(&Appearance::default(), false);
        let raw = serde_json::to_value(&v).unwrap();
        assert_eq!(raw["accent"], "mint");
        assert_eq!(raw["resolvedMode"], "dark");
        assert_eq!(raw["uiFontSize"], 14);
    }
}
