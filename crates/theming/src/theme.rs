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

use serde::Deserialize;

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
            _ => Self::default(),
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
            status_fg: "yellow".into(),
            bar_bg: "reset".into(),
            danger_fg: "red".into(),
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
#[derive(Debug, Clone, Default, Deserialize)]
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
    pub status_fg: Option<String>,
    pub bar_bg: Option<String>,
    pub danger_fg: Option<String>,
    pub border_type: Option<String>,
    pub separator: Option<String>,
}

impl RawTheme {
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
        Self {
            selection_bg: raw.selection_bg.unwrap_or(base.selection_bg),
            selection_fg: raw.selection_fg.unwrap_or(base.selection_fg),
            border_fg: raw.border_fg.unwrap_or(base.border_fg),
            border_focused_fg: raw.border_focused_fg.unwrap_or(base.border_focused_fg),
            title_fg: raw.title_fg.unwrap_or(base.title_fg),
            accent_fg: raw.accent_fg.unwrap_or(base.accent_fg),
            dir_fg: raw.dir_fg.unwrap_or(base.dir_fg),
            file_fg: raw.file_fg.unwrap_or(base.file_fg),
            source_fg: raw.source_fg.unwrap_or(base.source_fg),
            config_fg: raw.config_fg.unwrap_or(base.config_fg),
            doc_fg: raw.doc_fg.unwrap_or(base.doc_fg),
            archive_fg: raw.archive_fg.unwrap_or(base.archive_fg),
            media_fg: raw.media_fg.unwrap_or(base.media_fg),
            status_fg: raw.status_fg.unwrap_or(base.status_fg),
            bar_bg: raw.bar_bg.unwrap_or(base.bar_bg),
            danger_fg: raw.danger_fg.unwrap_or(base.danger_fg),
            border_type: raw.border_type.unwrap_or(base.border_type),
            separator: raw.separator.unwrap_or(base.separator),
        }
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
    fn unknown_theme_name_falls_back_to_default_palette() {
        let raw = RawTheme {
            name: Some("nonexistent".into()),
            ..Default::default()
        };
        let theme: Theme = raw.into();
        assert_eq!(theme, Theme::default());
    }
}
