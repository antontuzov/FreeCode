//! The appearance slice of `Settings` the shell needs before first paint. The Settings surface
//! writes the whole document; this is the narrower view the window chrome is painted from.
//!
//! Two modes travel on the wire: `themeMode` is what the user chose (which may be `system`), while
//! `resolvedMode` is what the window should actually paint. Echoing only the resolved value back
//! into the setting would silently turn "System" into a fixed light or dark the first time the
//! panel saved.

use serde::{Deserialize, Serialize};
use smollm_config::{
    Appearance, ContentWidth, FontStyle, InterfaceStyle, Language, Settings, ThemeMode,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceView {
    pub theme_mode: ThemeMode,
    pub resolved_mode: ThemeMode,
    pub accent: InterfaceStyle,
    pub glass: bool,
    /// The shell paints every label from this catalog, so the choice belongs on the wire even
    /// though it is not a colour.
    pub language: Language,
    pub ui_font_size: u16,
    pub code_font_size: u16,
    /// The interface typeface and the width of the reading column are painted by the window too, so
    /// they travel with the rest of what first paint needs — waiting for a Settings reload to learn
    /// the typeface would flash the wrong one on every launch.
    pub font_style: FontStyle,
    pub content_width: ContentWidth,
    /// The two code-display toggles, for the same reason: an answer's fences are rendered by the
    /// shell, so a gutter either appears on the first answer after the change or it does not.
    pub show_line_numbers: bool,
    pub wrap_long_lines: bool,
    /// Which mascot tile the app wears, one-based, or `None` to roll one per launch. It is on the
    /// wire for the same reason as the rest of this view: the Dock tile is host-side, so the pin has
    /// to reach the process that draws it before the first frame, and again the moment it changes.
    pub app_icon: Option<u8>,
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
        language: appearance.language,
        ui_font_size: appearance.ui_font_size_px,
        code_font_size: appearance.code_font_size_px,
        font_style: appearance.font_style,
        content_width: appearance.content_width,
        show_line_numbers: appearance.show_line_numbers,
        wrap_long_lines: appearance.wrap_long_lines,
        app_icon: appearance.app_icon,
    }
}

/// Write the shell-facing controls back into the full document. Every other `Appearance` field —
/// terminal theme, the code theme names, the file-icon set — is left exactly as it was, because a
/// save from the shell must not reset what only the Settings surface renders.
pub fn apply(view: &AppearanceView, appearance: &mut Appearance) {
    appearance.theme_mode = view.theme_mode;
    appearance.interface_style = view.accent;
    appearance.glass_effects = view.glass;
    appearance.language = view.language;
    appearance.ui_font_size_px = view.ui_font_size;
    appearance.code_font_size_px = view.code_font_size;
    appearance.font_style = view.font_style;
    appearance.content_width = view.content_width;
    appearance.show_line_numbers = view.show_line_numbers;
    appearance.wrap_long_lines = view.wrap_long_lines;
    appearance.app_icon = view.app_icon;
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
    fn saving_the_shell_slice_lands_its_own_fields_and_leaves_the_rest_alone() {
        let mut settings = Settings::default();
        settings.appearance.font_style = smollm_config::FontStyle::Serif;
        settings.appearance.terminal_theme = smollm_config::TerminalTheme::Custom;
        settings.appearance.content_width = smollm_config::ContentWidth::Wide;
        settings.appearance.dark_code_theme = "vikia".into();
        settings.appearance.show_line_numbers = false;
        settings.appearance.wrap_long_lines = true;

        let mut current = settings.appearance.clone();
        let mut v = view(&current, false);
        v.accent = InterfaceStyle::Forest;
        v.glass = false;
        v.ui_font_size = 16;
        // Everything the window paints from has to survive the trip back, rather than being frozen
        // at whatever the document held when the view was built.
        v.font_style = smollm_config::FontStyle::Mono;
        v.content_width = smollm_config::ContentWidth::Full;
        v.show_line_numbers = true;
        v.wrap_long_lines = false;
        v.app_icon = Some(9);
        apply(&v, &mut current);

        assert_eq!(current.interface_style, InterfaceStyle::Forest);
        assert!(!current.glass_effects);
        assert_eq!(current.ui_font_size_px, 16);
        assert_eq!(current.font_style, smollm_config::FontStyle::Mono);
        assert_eq!(current.content_width, smollm_config::ContentWidth::Full);
        assert!(current.show_line_numbers);
        assert!(!current.wrap_long_lines);
        assert_eq!(current.app_icon, Some(9));
        // These two are still only rendered by the Settings surface, which writes them itself.
        assert_eq!(current.terminal_theme, settings.appearance.terminal_theme);
        assert_eq!(current.dark_code_theme, settings.appearance.dark_code_theme);
    }

    #[test]
    fn the_wire_names_match_the_i18n_and_css_token_values() {
        let v = view(&Appearance::default(), false);
        let raw = serde_json::to_value(&v).unwrap();
        assert_eq!(raw["accent"], "mint");
        assert_eq!(raw["resolvedMode"], "dark");
        assert_eq!(raw["language"], "en");
        assert_eq!(raw["uiFontSize"], 14);
        // The Settings page offers these exact strings, and tokens.css keys its rules off them.
        assert_eq!(raw["fontStyle"], "sans");
        assert_eq!(raw["contentWidth"], "standard");
        // The reference ships a guttered code block, so the default is numbers on, wrapping off.
        assert_eq!(raw["showLineNumbers"], true);
        assert_eq!(raw["wrapLongLines"], false);
        // Null rather than absent: the webview types this `number | null`, and a missing key would
        // read as undefined to a page that has to decide whether to show a pinned tile.
        assert_eq!(raw["appIcon"], serde_json::Value::Null);
        let pinned = AppearanceView {
            app_icon: Some(3),
            ..view(&Appearance::default(), false)
        };
        assert_eq!(serde_json::to_value(&pinned).unwrap()["appIcon"], 3);
    }
}
