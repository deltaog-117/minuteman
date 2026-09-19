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

use std::path::PathBuf;

use serde::Deserialize;

use crate::keymap::{KeyMap, RawKeyMap};
use crate::theme::{RawTheme, Theme};
use crate::ui::{RawUi, Ui};

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawConfig {
    keys: RawKeyMap,
    theme: RawTheme,
    ui: RawUi,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub keys: KeyMap,
    pub theme: Theme,
    pub ui: Ui,
}

impl Config {
    /// Loads the config from `~/.config/minuteman/config.toml` (XDG), falling back to built-in
    /// defaults if the file is absent or fails to parse. A malformed config must never prevent
    /// the app from starting.
    pub fn load() -> Self {
        let raw = Self::config_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| match toml::from_str::<RawConfig>(&text) {
                Ok(cfg) => Some(cfg),
                Err(err) => {
                    eprintln!("minuteman: failed to parse config.toml, using defaults: {err}");
                    None
                }
            })
            .unwrap_or_default();

        Self {
            keys: raw.keys.into(),
            theme: raw.theme.into(),
            ui: raw.ui.into(),
        }
    }

    fn config_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "minuteman")
            .map(|dirs| dirs.config_dir().join("config.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_falls_back_per_missing_field() {
        let raw: RawConfig = toml::from_str("[keys]\nmove_down = [\"n\"]\n").unwrap();
        assert_eq!(raw.keys.move_down, vec!["n".to_string()]);
        // move_up was not specified, so it keeps the default.
        assert_eq!(raw.keys.move_up, vec!["k".to_string()]);

        let theme: Theme = raw.theme.into();
        assert_eq!(theme.selection_bg, Theme::default().selection_bg);
    }

    /// The shipped `config.example.toml` must always parse and, being nothing but the built-in
    /// defaults spelled out explicitly, resolve to exactly `RawConfig::default()` — catches the
    /// example silently drifting out of sync with a future default change.
    #[test]
    fn shipped_example_config_parses_and_matches_defaults() {
        let text = include_str!("../../../config.example.toml");
        let raw: RawConfig = toml::from_str(text).unwrap();
        let keys: KeyMap = raw.keys.into();
        let default_keys: KeyMap = RawKeyMap::default().into();
        assert_eq!(keys, default_keys);

        let theme: Theme = raw.theme.into();
        assert_eq!(theme, Theme::default());

        let ui: Ui = raw.ui.into();
        assert_eq!(ui, Ui::default());
    }

    #[test]
    fn ui_table_selects_the_glyph_set() {
        let raw: RawConfig = toml::from_str("[ui]\nglyphs = \"nerd\"\n").unwrap();
        let ui: Ui = raw.ui.into();
        assert_eq!(ui.glyphs, crate::ui::GlyphSet::Nerd);
    }
}
