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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub selection_bg: String,
    pub selection_fg: String,
    pub border_fg: String,
    pub title_fg: String,
    pub dir_fg: String,
    pub file_fg: String,
    pub status_fg: String,
}

impl Theme {
    /// Looks up a built-in palette by name (case-insensitive). An unrecognised name falls back
    /// to `"default"` — like a malformed config, a typo'd theme name must never block startup.
    fn named(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "dracula" => Self::dracula(),
            _ => Self::default(),
        }
    }

    fn dracula() -> Self {
        Self {
            selection_bg: "magenta".into(),
            selection_fg: "black".into(),
            border_fg: "magenta".into(),
            title_fg: "cyan".into(),
            dir_fg: "cyan".into(),
            file_fg: "white".into(),
            status_fg: "yellow".into(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            selection_bg: "blue".into(),
            selection_fg: "white".into(),
            border_fg: "gray".into(),
            title_fg: "white".into(),
            dir_fg: "blue".into(),
            file_fg: "white".into(),
            status_fg: "gray".into(),
        }
    }
}

/// Deserialized `[theme]` config. `name` selects a built-in base palette (`Theme::named`,
/// defaulting to `"default"`); any individually specified color field overrides that palette's
/// value for just that field. Every field is optional so a partial table only overrides what it
/// mentions, the same fallback shape `RawKeyMap` uses for keybindings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RawTheme {
    pub name: Option<String>,
    pub selection_bg: Option<String>,
    pub selection_fg: Option<String>,
    pub border_fg: Option<String>,
    pub title_fg: Option<String>,
    pub dir_fg: Option<String>,
    pub file_fg: Option<String>,
    pub status_fg: Option<String>,
}

impl From<RawTheme> for Theme {
    fn from(raw: RawTheme) -> Self {
        let base = Theme::named(raw.name.as_deref().unwrap_or("default"));
        Self {
            selection_bg: raw.selection_bg.unwrap_or(base.selection_bg),
            selection_fg: raw.selection_fg.unwrap_or(base.selection_fg),
            border_fg: raw.border_fg.unwrap_or(base.border_fg),
            title_fg: raw.title_fg.unwrap_or(base.title_fg),
            dir_fg: raw.dir_fg.unwrap_or(base.dir_fg),
            file_fg: raw.file_fg.unwrap_or(base.file_fg),
            status_fg: raw.status_fg.unwrap_or(base.status_fg),
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
