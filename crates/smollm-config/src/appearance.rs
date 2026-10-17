//! Appearance settings: the four interface styles, light/dark, glass and the code editor facet.
//!
//! Every field here maps to one control on Settings › Appearance, and each one resolves to CSS
//! custom properties in `apps/desktop/src/theme`. No component may hardcode a hex value: a theme
//! change must be a pure token swap.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    #[default]
    Dark,
    System,
}

/// The four interface styles, in the card order shown in the reference screenshot. The selected
/// card paints the accent token palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InterfaceStyle {
    Forest,
    #[default]
    Mint,
    Bee,
    Parchment,
}

impl InterfaceStyle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Forest => "Forest",
            Self::Mint => "Mint",
            Self::Bee => "Bee",
            Self::Parchment => "Parchment",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TerminalTheme {
    #[default]
    FollowTheme,
    Light,
    Dark,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FontStyle {
    #[default]
    Sans,
    Serif,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ContentWidth {
    #[default]
    Standard,
    Wide,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FileIconSet {
    #[default]
    Qoder,
    Seti,
    Vscode,
    None,
}

/// `en` plus `zh`: the coding view's task-summary box ships in Chinese in the reference, so i18n is
/// not a stretch goal — it is load-bearing for the demo content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Zh,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Appearance {
    pub theme_mode: ThemeMode,
    pub interface_style: InterfaceStyle,
    /// Dialog mask blur, glass layers and similar effects. Off means solid backgrounds, and every
    /// surface must still render correctly.
    pub glass_effects: bool,
    pub terminal_theme: TerminalTheme,
    pub use_built_in_browser_for_terminal_links: bool,
    pub language: Language,
    pub font_style: FontStyle,
    pub content_width: ContentWidth,
    pub file_icons: FileIconSet,
    /// Which of the twelve mascot tiles the app wears, one-based. `None` lets each launch roll a new
    /// one; `Some` pins it. Pinning belongs on disk rather than in the webview because the Dock tile
    /// is drawn by the host, which never sees the page's own state.
    pub app_icon: Option<u8>,

    // Developer/code facet of the same page.
    pub ui_font_size_px: u16,
    pub light_code_theme: String,
    pub dark_code_theme: String,
    pub show_line_numbers: bool,
    pub wrap_long_lines: bool,
    pub code_font_size_px: u16,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::Dark,
            interface_style: InterfaceStyle::Mint,
            glass_effects: true,
            terminal_theme: TerminalTheme::FollowTheme,
            use_built_in_browser_for_terminal_links: false,
            language: Language::En,
            font_style: FontStyle::Sans,
            content_width: ContentWidth::Standard,
            file_icons: FileIconSet::Qoder,
            app_icon: None,
            ui_font_size_px: 14,
            light_code_theme: "github-light".into(),
            dark_code_theme: "github-dark".into(),
            show_line_numbers: true,
            wrap_long_lines: false,
            code_font_size_px: 12,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_match_the_reference_screenshots() {
        let a = Appearance::default();
        assert_eq!(a.interface_style, InterfaceStyle::Mint);
        assert!(a.glass_effects);
        assert!(!a.use_built_in_browser_for_terminal_links);
        assert_eq!(a.ui_font_size_px, 14);
        assert_eq!(a.code_font_size_px, 12);
        assert!(a.show_line_numbers);
        assert!(!a.wrap_long_lines);
    }

    #[test]
    fn the_icon_is_pinned_only_when_the_user_pins_it() {
        // The default is the per-launch roll, so a document written before the picker existed parses
        // and keeps rolling rather than freezing every install on one duck.
        assert_eq!(Appearance::default().app_icon, None);
        let a: Appearance = serde_json::from_value(json!({ "appIcon": 7 })).unwrap();
        assert_eq!(a.app_icon, Some(7));
        let a: Appearance = serde_json::from_value(json!({ "appIcon": null })).unwrap();
        assert_eq!(a.app_icon, None);
    }

    #[test]
    fn partial_json_fills_missing_fields_instead_of_failing() {
        let a: Appearance = serde_json::from_value(json!({ "interfaceStyle": "bee" })).unwrap();
        assert_eq!(a.interface_style, InterfaceStyle::Bee);
        assert_eq!(a.font_style, FontStyle::Sans);
    }
}
