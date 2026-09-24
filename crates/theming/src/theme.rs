// Minuteman - a fast, Ranger-inspired terminal file manager
// Copyright (C) 2026  Davi Oliveira Gonçalves
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use serde::{Deserialize, Serialize};

/// A resolved color palette — every field is always populated (via `named`/`Default`, then any
/// config overrides), so `tui` never has to guess at a fallback.
///
/// Colors are strings so a config can use either a basic name (`"cyan"`) or a `#rrggbb` /
/// `#rgb` hex value; `tui` resolves them (and quantizes hex down to 256 colors on terminals
/// without truecolor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub selection_bg: String,
    /// The selected row's text color, or `"keep"` to leave each entry in its own file-type color
    /// and only paint the row's background.
    pub selection_fg: String,
    /// Frame color of every pane that isn't the active one.
    pub border_fg: String,
    /// Frame color of the active pane — the one keystrokes currently act on.
    pub border_focused_fg: String,
    pub title_fg: String,
    /// Highlights: the active pane's title, the selection stripe, and the mark on marked rows.
    pub accent_fg: String,
    pub dir_fg: String,
    pub file_fg: String,
    pub source_fg: String,
    pub config_fg: String,
    pub doc_fg: String,
    pub archive_fg: String,
    pub media_fg: String,
    /// Syntax highlighting, in the text preview: `fn`/`let`/`if`/…
    pub syntax_keyword_fg: String,
    /// Syntax highlighting: string literals.
    pub syntax_string_fg: String,
    /// Syntax highlighting: comments.
    pub syntax_comment_fg: String,
    /// Syntax highlighting: numeric and character literals.
    pub syntax_number_fg: String,
    /// Syntax highlighting: a called or declared function's name.
    pub syntax_function_fg: String,
    /// Syntax highlighting: a type, class or struct name.
    pub syntax_type_fg: String,
    pub status_fg: String,
    /// Background of the status bar's segments (the mode pill has its own color).
    pub bar_bg: String,
    /// Destructive-action prompts (delete, overwrite) in the status bar.
    pub danger_fg: String,
    /// `"rounded"`, `"plain"`, `"double"`, or `"thick"`.
    pub border_type: String,
    /// Edge between status-bar segments: `"flat"` (works in any font), `"arrow"` (Powerline
    /// arrows; needs a Powerline/Nerd Font), or `"auto"` (arrows exactly when the glyph set is
    /// `nerd`).
    pub separator: String,
}

impl Theme {
    /// Looks up a built-in palette by name (case-insensitive). An unrecognised name falls back
    /// to the default — like a malformed config, a typo'd theme name must never block startup.
    /// `pub` so the settings popup can cycle between palettes live, in memory.
    pub fn named(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "classic" => Self::classic(),
            "dracula" => Self::dracula(),
            "catppuccin" => Self::catppuccin(),
            "catppuccin-latte" => Self::catppuccin_latte(),
            "nord" => Self::nord(),
            _ => Self::default(),
        }
    }

    /// What "nothing configured" resolves to: `is_dark` comes from a live OSC 11 query of the
    /// terminal's own background color (see `crates/tui/src/image_preview.rs`, which bundles
    /// that query into its existing graphics-capability probe), classified by [`Theme::is_dark`].
    /// `None` means the terminal never answered — most terminals that don't support the query
    /// simply stay silent, so this is not a "light" signal — and falls back to the original
    /// neon-cyberpunk look rather than guessing. An explicit `[theme]` (a `name`, or even just one
    /// overridden field) always wins over this; see `Config::theme_is_customized`.
    pub fn auto(is_dark: Option<bool>) -> Self {
        match is_dark {
            Some(true) => Self::catppuccin(),
            Some(false) => Self::catppuccin_latte(),
            None => Self::default(),
        }
    }

    /// Classifies an OSC-11-reported background color as dark (`true`) or light, by perceptual
    /// luminance (ITU-R BT.601).
    pub fn is_dark(r: u8, g: u8, b: u8) -> bool {
        let luminance = 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b);
        luminance < 128.0
    }

    /// Layers `overrides` on top of `self`, field by field — for live, in-session edits (the
    /// appearance popup, in `tui`) rather than picking a whole named base palette. A field left
    /// `None` in `overrides` keeps `self`'s value.
    pub fn overlay_raw(&self, overrides: &RawTheme) -> Theme {
        Theme {
            selection_bg: overrides
                .selection_bg
                .clone()
                .unwrap_or_else(|| self.selection_bg.clone()),
            selection_fg: overrides
                .selection_fg
                .clone()
                .unwrap_or_else(|| self.selection_fg.clone()),
            border_fg: overrides
                .border_fg
                .clone()
                .unwrap_or_else(|| self.border_fg.clone()),
            border_focused_fg: overrides
                .border_focused_fg
                .clone()
                .unwrap_or_else(|| self.border_focused_fg.clone()),
            title_fg: overrides
                .title_fg
                .clone()
                .unwrap_or_else(|| self.title_fg.clone()),
            accent_fg: overrides
                .accent_fg
                .clone()
                .unwrap_or_else(|| self.accent_fg.clone()),
            dir_fg: overrides
                .dir_fg
                .clone()
                .unwrap_or_else(|| self.dir_fg.clone()),
            file_fg: overrides
                .file_fg
                .clone()
                .unwrap_or_else(|| self.file_fg.clone()),
            source_fg: overrides
                .source_fg
                .clone()
                .unwrap_or_else(|| self.source_fg.clone()),
            config_fg: overrides
                .config_fg
                .clone()
                .unwrap_or_else(|| self.config_fg.clone()),
            doc_fg: overrides
                .doc_fg
                .clone()
                .unwrap_or_else(|| self.doc_fg.clone()),
            archive_fg: overrides
                .archive_fg
                .clone()
                .unwrap_or_else(|| self.archive_fg.clone()),
            media_fg: overrides
                .media_fg
                .clone()
                .unwrap_or_else(|| self.media_fg.clone()),
            syntax_keyword_fg: overrides
                .syntax_keyword_fg
                .clone()
                .unwrap_or_else(|| self.syntax_keyword_fg.clone()),
            syntax_string_fg: overrides
                .syntax_string_fg
                .clone()
                .unwrap_or_else(|| self.syntax_string_fg.clone()),
            syntax_comment_fg: overrides
                .syntax_comment_fg
                .clone()
                .unwrap_or_else(|| self.syntax_comment_fg.clone()),
            syntax_number_fg: overrides
                .syntax_number_fg
                .clone()
                .unwrap_or_else(|| self.syntax_number_fg.clone()),
            syntax_function_fg: overrides
                .syntax_function_fg
                .clone()
                .unwrap_or_else(|| self.syntax_function_fg.clone()),
            syntax_type_fg: overrides
                .syntax_type_fg
                .clone()
                .unwrap_or_else(|| self.syntax_type_fg.clone()),
            status_fg: overrides
                .status_fg
                .clone()
                .unwrap_or_else(|| self.status_fg.clone()),
            bar_bg: overrides
                .bar_bg
                .clone()
                .unwrap_or_else(|| self.bar_bg.clone()),
            danger_fg: overrides
                .danger_fg
                .clone()
                .unwrap_or_else(|| self.danger_fg.clone()),
            border_type: overrides
                .border_type
                .clone()
                .unwrap_or_else(|| self.border_type.clone()),
            separator: overrides
                .separator
                .clone()
                .unwrap_or_else(|| self.separator.clone()),
        }
    }

    /// The original 16-color look, from before the neon palette became the default.
    fn classic() -> Self {
        Self {
            selection_bg: "blue".into(),
            selection_fg: "white".into(),
            border_fg: "gray".into(),
            border_focused_fg: "white".into(),
            title_fg: "white".into(),
            accent_fg: "white".into(),
            dir_fg: "blue".into(),
            file_fg: "white".into(),
            source_fg: "white".into(),
            config_fg: "white".into(),
            doc_fg: "white".into(),
            archive_fg: "white".into(),
            media_fg: "white".into(),
            syntax_keyword_fg: "magenta".into(),
            syntax_string_fg: "green".into(),
            syntax_comment_fg: "cyan".into(),
            syntax_number_fg: "red".into(),
            syntax_function_fg: "yellow".into(),
            syntax_type_fg: "blue".into(),
            status_fg: "gray".into(),
            bar_bg: "reset".into(),
            danger_fg: "red".into(),
            border_type: "plain".into(),
            separator: "auto".into(),
        }
    }

    fn dracula() -> Self {
        Self {
            selection_bg: "magenta".into(),
            selection_fg: "black".into(),
            border_fg: "magenta".into(),
            border_focused_fg: "cyan".into(),
            title_fg: "cyan".into(),
            accent_fg: "yellow".into(),
            dir_fg: "cyan".into(),
            file_fg: "white".into(),
            source_fg: "green".into(),
            config_fg: "yellow".into(),
            doc_fg: "magenta".into(),
            archive_fg: "red".into(),
            media_fg: "magenta".into(),
            // Dracula's own published syntax-highlighting colors.
            syntax_keyword_fg: "#ff79c6".into(),
            syntax_string_fg: "#f1fa8c".into(),
            syntax_comment_fg: "#6272a4".into(),
            syntax_number_fg: "#bd93f9".into(),
            syntax_function_fg: "#50fa7b".into(),
            syntax_type_fg: "#8be9fd".into(),
            status_fg: "yellow".into(),
            bar_bg: "reset".into(),
            danger_fg: "red".into(),
            border_type: "rounded".into(),
            separator: "auto".into(),
        }
    }

    /// Catppuccin's dark flavor (Mocha): mauve and pink accents on a deep blue-black.
    fn catppuccin() -> Self {
        Self {
            selection_bg: "#45475a".into(),
            selection_fg: "#cdd6f4".into(),
            border_fg: "#585b70".into(),
            border_focused_fg: "#cba6f7".into(),
            title_fg: "#b4befe".into(),
            accent_fg: "#f5c2e7".into(),
            dir_fg: "#89b4fa".into(),
            file_fg: "#cdd6f4".into(),
            source_fg: "#a6e3a1".into(),
            config_fg: "#f9e2af".into(),
            doc_fg: "#cba6f7".into(),
            archive_fg: "#fab387".into(),
            media_fg: "#f5c2e7".into(),
            // Catppuccin's own published syntax-highlighting mapping (Mocha).
            syntax_keyword_fg: "#cba6f7".into(),
            syntax_string_fg: "#a6e3a1".into(),
            syntax_comment_fg: "#6c7086".into(),
            syntax_number_fg: "#fab387".into(),
            syntax_function_fg: "#89b4fa".into(),
            syntax_type_fg: "#f9e2af".into(),
            status_fg: "#a6adc8".into(),
            bar_bg: "#181825".into(),
            danger_fg: "#f38ba8".into(),
            border_type: "rounded".into(),
            separator: "auto".into(),
        }
    }

    /// Catppuccin's light flavor (Latte) — the same family, inverted for a light background.
    /// Only reachable by name (`"catppuccin-latte"`) or via [`Theme::auto`] on a light terminal;
    /// it isn't in the settings popup's cycle, which sticks to dark-background palettes.
    fn catppuccin_latte() -> Self {
        Self {
            selection_bg: "#bcc0cc".into(),
            selection_fg: "#4c4f69".into(),
            border_fg: "#acb0be".into(),
            border_focused_fg: "#8839ef".into(),
            title_fg: "#7287fd".into(),
            accent_fg: "#ea76cb".into(),
            dir_fg: "#1e66f5".into(),
            file_fg: "#4c4f69".into(),
            source_fg: "#40a02b".into(),
            config_fg: "#df8e1d".into(),
            doc_fg: "#8839ef".into(),
            archive_fg: "#fe640b".into(),
            media_fg: "#ea76cb".into(),
            // Catppuccin's own published syntax-highlighting mapping (Latte).
            syntax_keyword_fg: "#8839ef".into(),
            syntax_string_fg: "#40a02b".into(),
            syntax_comment_fg: "#9ca0b0".into(),
            syntax_number_fg: "#fe640b".into(),
            syntax_function_fg: "#1e66f5".into(),
            syntax_type_fg: "#df8e1d".into(),
            status_fg: "#6c6f85".into(),
            bar_bg: "#e6e9ef".into(),
            danger_fg: "#d20f39".into(),
            border_type: "rounded".into(),
            separator: "auto".into(),
        }
    }

    /// Nord: cool blue-gray frost tones on a deep slate background.
    fn nord() -> Self {
        Self {
            selection_bg: "#434c5e".into(),
            selection_fg: "#eceff4".into(),
            border_fg: "#4c566a".into(),
            border_focused_fg: "#88c0d0".into(),
            title_fg: "#81a1c1".into(),
            accent_fg: "#b48ead".into(),
            dir_fg: "#88c0d0".into(),
            file_fg: "#d8dee9".into(),
            source_fg: "#a3be8c".into(),
            config_fg: "#ebcb8b".into(),
            doc_fg: "#8fbcbb".into(),
            archive_fg: "#d08770".into(),
            media_fg: "#b48ead".into(),
            // Nord's own published syntax-highlighting reference: keywords/types/operators in
            // nord9, functions in nord8, strings in nord14, numbers/constants in nord15,
            // comments in nord3 — the same hex values already used above for other elements.
            syntax_keyword_fg: "#81a1c1".into(),
            syntax_string_fg: "#a3be8c".into(),
            syntax_comment_fg: "#4c566a".into(),
            syntax_number_fg: "#b48ead".into(),
            syntax_function_fg: "#88c0d0".into(),
            syntax_type_fg: "#81a1c1".into(),
            status_fg: "#81a1c1".into(),
            bar_bg: "#3b4252".into(),
            danger_fg: "#bf616a".into(),
            border_type: "rounded".into(),
            separator: "auto".into(),
        }
    }
}

/// The built-in default: a cyberpunk neon palette — cyan and magenta on the terminal's own dark
/// background, with dim indigo frames so the active pane's cyan reads as a glow.
impl Default for Theme {
    fn default() -> Self {
        Self {
            selection_bg: "#2b1a4f".into(),
            selection_fg: "keep".into(),
            border_fg: "#3d4270".into(),
            border_focused_fg: "#00f0ff".into(),
            title_fg: "#8a8fd6".into(),
            accent_fg: "#ff2bd6".into(),
            dir_fg: "#00d9ff".into(),
            file_fg: "#c8ccff".into(),
            source_fg: "#39ff88".into(),
            config_fg: "#ffd60a".into(),
            doc_fg: "#b69cff".into(),
            archive_fg: "#ff7a3d".into(),
            media_fg: "#ff5cf0".into(),
            syntax_keyword_fg: "#ff2bd6".into(),
            syntax_string_fg: "#39ff88".into(),
            syntax_comment_fg: "#7a80b8".into(),
            syntax_number_fg: "#ffd60a".into(),
            syntax_function_fg: "#00d9ff".into(),
            syntax_type_fg: "#b69cff".into(),
            status_fg: "#7a80b8".into(),
            bar_bg: "#1a1f3d".into(),
            danger_fg: "#ff3860".into(),
            border_type: "rounded".into(),
            separator: "auto".into(),
        }
    }
}

/// Deserialized `[theme]` config. `name` selects a built-in base palette (`Theme::named`,
/// defaulting to the neon one); any individually specified field overrides that palette's
/// value for just that field. Every field is optional so a partial table only overrides what it
/// mentions, the same fallback shape `RawKeyMap` uses for keybindings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct RawTheme {
    pub name: Option<String>,
    pub selection_bg: Option<String>,
    pub selection_fg: Option<String>,
    pub border_fg: Option<String>,
    pub border_focused_fg: Option<String>,
    pub title_fg: Option<String>,
    pub accent_fg: Option<String>,
    pub dir_fg: Option<String>,
    pub file_fg: Option<String>,
    pub source_fg: Option<String>,
    pub config_fg: Option<String>,
    pub doc_fg: Option<String>,
    pub archive_fg: Option<String>,
    pub media_fg: Option<String>,
    pub syntax_keyword_fg: Option<String>,
    pub syntax_string_fg: Option<String>,
    pub syntax_comment_fg: Option<String>,
    pub syntax_number_fg: Option<String>,
    pub syntax_function_fg: Option<String>,
    pub syntax_type_fg: Option<String>,
    pub status_fg: Option<String>,
    pub bar_bg: Option<String>,
    pub danger_fg: Option<String>,
    pub border_type: Option<String>,
    pub separator: Option<String>,
}

impl RawTheme {
    /// A fully-specified `RawTheme` (every field `Some`, `name` left `None`) snapshotting a
    /// resolved `Theme` — how the appearance popup's "Save theme" turns the live look into
    /// something a [`CustomTheme`](crate::CustomTheme) can store and later reproduce exactly,
    /// without depending on whichever built-in palette it happened to start from.
    pub fn from_theme(theme: &Theme) -> RawTheme {
        RawTheme {
            name: None,
            selection_bg: Some(theme.selection_bg.clone()),
            selection_fg: Some(theme.selection_fg.clone()),
            border_fg: Some(theme.border_fg.clone()),
            border_focused_fg: Some(theme.border_focused_fg.clone()),
            title_fg: Some(theme.title_fg.clone()),
            accent_fg: Some(theme.accent_fg.clone()),
            dir_fg: Some(theme.dir_fg.clone()),
            file_fg: Some(theme.file_fg.clone()),
            source_fg: Some(theme.source_fg.clone()),
            config_fg: Some(theme.config_fg.clone()),
            doc_fg: Some(theme.doc_fg.clone()),
            archive_fg: Some(theme.archive_fg.clone()),
            media_fg: Some(theme.media_fg.clone()),
            syntax_keyword_fg: Some(theme.syntax_keyword_fg.clone()),
            syntax_string_fg: Some(theme.syntax_string_fg.clone()),
            syntax_comment_fg: Some(theme.syntax_comment_fg.clone()),
            syntax_number_fg: Some(theme.syntax_number_fg.clone()),
            syntax_function_fg: Some(theme.syntax_function_fg.clone()),
            syntax_type_fg: Some(theme.syntax_type_fg.clone()),
            status_fg: Some(theme.status_fg.clone()),
            bar_bg: Some(theme.bar_bg.clone()),
            danger_fg: Some(theme.danger_fg.clone()),
            border_type: Some(theme.border_type.clone()),
            separator: Some(theme.separator.clone()),
        }
    }

    /// `top` wins wherever it sets a field; otherwise `self` shows through. Lets `appearance.toml`
    /// override a `[theme]` still left in `config.toml`, one field at a time.
    pub fn overlay(self, top: RawTheme) -> RawTheme {
        RawTheme {
            name: top.name.or(self.name),
            selection_bg: top.selection_bg.or(self.selection_bg),
            selection_fg: top.selection_fg.or(self.selection_fg),
            border_fg: top.border_fg.or(self.border_fg),
            border_focused_fg: top.border_focused_fg.or(self.border_focused_fg),
            title_fg: top.title_fg.or(self.title_fg),
            accent_fg: top.accent_fg.or(self.accent_fg),
            dir_fg: top.dir_fg.or(self.dir_fg),
            file_fg: top.file_fg.or(self.file_fg),
            source_fg: top.source_fg.or(self.source_fg),
            config_fg: top.config_fg.or(self.config_fg),
            doc_fg: top.doc_fg.or(self.doc_fg),
            archive_fg: top.archive_fg.or(self.archive_fg),
            media_fg: top.media_fg.or(self.media_fg),
            syntax_keyword_fg: top.syntax_keyword_fg.or(self.syntax_keyword_fg),
            syntax_string_fg: top.syntax_string_fg.or(self.syntax_string_fg),
            syntax_comment_fg: top.syntax_comment_fg.or(self.syntax_comment_fg),
            syntax_number_fg: top.syntax_number_fg.or(self.syntax_number_fg),
            syntax_function_fg: top.syntax_function_fg.or(self.syntax_function_fg),
            syntax_type_fg: top.syntax_type_fg.or(self.syntax_type_fg),
            status_fg: top.status_fg.or(self.status_fg),
            bar_bg: top.bar_bg.or(self.bar_bg),
            danger_fg: top.danger_fg.or(self.danger_fg),
            border_type: top.border_type.or(self.border_type),
            separator: top.separator.or(self.separator),
        }
    }
}

impl From<RawTheme> for Theme {
    fn from(raw: RawTheme) -> Self {
        let base = Theme::named(raw.name.as_deref().unwrap_or("default"));
        base.overlay_raw(&raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_raw_theme_resolves_to_default_palette() {
        let theme: Theme = RawTheme::default().into();
        assert_eq!(theme, Theme::default());
    }

    #[test]
    fn named_theme_selects_base_palette() {
        let raw = RawTheme {
            name: Some("dracula".into()),
            ..Default::default()
        };
        let theme: Theme = raw.into();
        assert_eq!(theme, Theme::dracula());
    }

    #[test]
    fn classic_palette_keeps_the_original_sixteen_color_look() {
        let raw = RawTheme {
            name: Some("classic".into()),
            ..Default::default()
        };
        let theme: Theme = raw.into();
        assert_eq!(theme, Theme::classic());
        assert_eq!(theme.selection_bg, "blue");
        assert_eq!(theme.border_type, "plain");
        assert_ne!(theme, Theme::default());
    }

    #[test]
    fn default_palette_is_the_truecolor_neon_one() {
        let theme = Theme::default();
        assert!(theme.border_focused_fg.starts_with('#'));
        assert_eq!(theme.selection_fg, "keep");
        assert_eq!(theme.border_type, "rounded");
        // Automatic: arrows only once the user has opted into the Nerd glyph set, since a
        // missing arrow glyph looks broken.
        assert_eq!(theme.separator, "auto");
    }

    #[test]
    fn individual_field_overrides_the_named_palette() {
        let raw = RawTheme {
            name: Some("dracula".into()),
            border_fg: Some("green".into()),
            ..Default::default()
        };
        let theme: Theme = raw.into();
        assert_eq!(theme.border_fg, "green");
        // Everything else still comes from the dracula palette.
        assert_eq!(theme.selection_bg, Theme::dracula().selection_bg);
    }

    #[test]
    fn overlay_raw_layers_onto_an_existing_theme_not_a_named_base() {
        let base = Theme::nord();
        let overrides = RawTheme {
            accent_fg: Some("#123456".into()),
            ..Default::default()
        };
        let layered = base.overlay_raw(&overrides);
        assert_eq!(layered.accent_fg, "#123456");
        // Everything else stays exactly the base's — no named palette is consulted.
        assert_eq!(layered.border_fg, base.border_fg);
        assert_eq!(layered.selection_bg, base.selection_bg);

        // An empty overlay changes nothing.
        assert_eq!(base.overlay_raw(&RawTheme::default()), base);
    }

    #[test]
    fn unknown_theme_name_falls_back_to_default_palette() {
        let raw = RawTheme {
            name: Some("nonexistent".into()),
            ..Default::default()
        };
        let theme: Theme = raw.into();
        assert_eq!(theme, Theme::default());
    }

    #[test]
    fn catppuccin_and_nord_are_reachable_by_name() {
        let mocha: Theme = RawTheme {
            name: Some("catppuccin".into()),
            ..Default::default()
        }
        .into();
        assert_eq!(mocha.selection_bg, "#45475a");
        assert_ne!(mocha, Theme::default());

        let latte: Theme = RawTheme {
            name: Some("catppuccin-latte".into()),
            ..Default::default()
        }
        .into();
        assert_eq!(latte.bar_bg, "#e6e9ef");
        assert_ne!(latte, mocha);

        let nord: Theme = RawTheme {
            name: Some("nord".into()),
            ..Default::default()
        }
        .into();
        assert_eq!(nord.border_focused_fg, "#88c0d0");
    }

    #[test]
    fn auto_picks_catppuccin_by_detected_darkness_and_falls_back_when_unknown() {
        assert_eq!(Theme::auto(Some(true)), Theme::catppuccin());
        assert_eq!(Theme::auto(Some(false)), Theme::catppuccin_latte());
        assert_eq!(Theme::auto(None), Theme::default());
    }

    #[test]
    fn is_dark_classifies_by_luminance() {
        assert!(Theme::is_dark(0, 0, 0));
        assert!(Theme::is_dark(0x1e, 0x1e, 0x2e)); // Catppuccin Mocha's own background
        assert!(!Theme::is_dark(255, 255, 255));
        assert!(!Theme::is_dark(0xef, 0xf1, 0xf5)); // Catppuccin Latte's own background
    }

    #[test]
    fn from_theme_round_trips_through_raw_regardless_of_base_palette() {
        for theme in [
            Theme::default(),
            Theme::classic(),
            Theme::dracula(),
            Theme::nord(),
        ] {
            let raw = RawTheme::from_theme(&theme);
            assert_eq!(raw.name, None, "a saved snapshot names no base palette");
            let resolved: Theme = raw.into();
            assert_eq!(
                resolved, theme,
                "every field was captured, so no base palette bleeds through"
            );
        }
    }
}
