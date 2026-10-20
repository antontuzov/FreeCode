//! Appearance settings: the interface styles, light/dark, glass and the code editor facet.
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

/// The interface styles, in the card order shown in the reference screenshot. The selected card
/// paints the accent token palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InterfaceStyle {
    Forest,
    #[default]
    Mint,
    Bee,
    Parchment,
    /// The reference app's own chrome: a cool periwinkle wash under a green action colour. Unlike the
    /// other four, its wash is not its accent's hue — see `--style-rain-tint` in tokens.css.
    Rain,
    /// The achromatic window: no green anywhere, and the one family whose action square inverts with
    /// the mode rather than merely lifting, because a near-black cannot lift on a near-black page.
    Ink,
    Rose,
    Slate,
}

impl InterfaceStyle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Forest => "Forest",
            Self::Mint => "Mint",
            Self::Bee => "Bee",
            Self::Parchment => "Parchment",
            Self::Rain => "Rain",
            Self::Ink => "Ink",
            Self::Rose => "Rose",
            Self::Slate => "Slate",
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

/// `en` plus `ru`, the two catalogs the shell ships. Chinese was the second one until 2026-10-20, when
/// the person using the app asked for their own language instead; the reference's task-summary box is
/// the only thing that ever needed the Chinese one, and it is demo content, not a contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Ru,
}

/// Reads `language` so that a file naming a catalog this build no longer holds — `"zh"` on every
/// machine that chose Chinese before it was replaced — opens in English rather than failing the whole
/// document. A lost language choice is recoverable on the Appearance page; a settings file the app
/// cannot read loses every other choice in it too.
fn language_lenient<'de, D>(deserializer: D) -> Result<Language, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    Ok(match raw.as_str() {
        "ru" => Language::Ru,
        _ => Language::En,
    })
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
    #[serde(deserialize_with = "language_lenient")]
    pub language: Language,
    pub font_style: FontStyle,
    pub content_width: ContentWidth,
    pub file_icons: FileIconSet,
    /// Which of the mascot tiles the app wears, one-based. `None` lets each launch roll a new
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

    #[test]
    fn a_language_this_build_dropped_opens_in_english() {
        // `zh` is what every install that chose Chinese before Russian replaced it has on disk, and
        // unlike a bad `interfaceStyle` it must not fail the document: losing the language is one row
        // to put back on the Appearance page, losing the file is every row at once.
        let a: Appearance =
            serde_json::from_value(json!({ "language": "zh", "themeMode": "light" })).unwrap();
        assert_eq!(a.language, Language::En);
        assert_eq!(a.theme_mode, ThemeMode::Light);
        let a: Appearance = serde_json::from_value(json!({ "language": "ru" })).unwrap();
        assert_eq!(a.language, Language::Ru);
        // And it is the value that goes back out, so a re-save stops carrying the dead code.
        let s = serde_json::to_string(&a).unwrap();
        assert!(s.contains("\"language\":\"ru\""), "{s}");
    }

    #[test]
    fn every_style_the_page_offers_parses() {
        // The Appearance page cards these eight, and the settings document is the only path from a
        // click to a stored style. An id the enum does not know does not fall back to a default — it
        // fails the whole document, so one drifted list loses every appearance setting at once.
        for id in [
            "forest",
            "mint",
            "bee",
            "parchment",
            "rain",
            "ink",
            "rose",
            "slate",
        ] {
            let a: Appearance = serde_json::from_value(json!({ "interfaceStyle": id }))
                .unwrap_or_else(|e| {
                    panic!("interfaceStyle {id} is offered by the page but does not parse: {e}")
                });
            assert!(!a.interface_style.label().is_empty());
        }
    }
}
