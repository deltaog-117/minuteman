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
use serde::de::DeserializeOwned;

use crate::appearance::{Font, RawFont, RawStyles, Styles};
use crate::keymap::{KeyMap, RawKeyMap};
use crate::theme::{RawTheme, Theme};
use crate::ui::{RawUi, Ui};

/// `config.toml`. Its `[theme]` and `[ui]` predate `appearance.toml` and are still honored, but
/// anything `appearance.toml` sets wins.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawConfig {
    keys: RawKeyMap,
    theme: RawTheme,
    ui: RawUi,
}

/// `appearance.toml`: the whole look in one file.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawAppearance {
    pub(crate) theme: RawTheme,
    pub(crate) ui: RawUi,
    pub(crate) style: RawStyles,
    pub(crate) font: RawFont,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub keys: KeyMap,
    pub theme: Theme,
    pub ui: Ui,
    /// Bold, italic, ... per interface element.
    pub styles: Styles,
    /// The font `init-terminal` prints; not something the TUI itself can apply.
    pub font: Font,
}

impl Config {
    /// Loads `config.toml` and `appearance.toml` from `~/.config/minuteman/` (XDG), falling back
    /// to built-in defaults for whatever is absent or fails to parse. A malformed file must never
    /// prevent the app from starting.
    pub fn load() -> Self {
        let read = |path: Option<PathBuf>| path.and_then(|p| std::fs::read_to_string(p).ok());
        Self::from_sources(
            read(Self::config_path()).as_deref(),
            read(Self::appearance_path()).as_deref(),
        )
    }

    /// Builds a config from the text of the two files (each `None` when absent). Split out from
    /// `load` so the layering can be tested without touching the filesystem.
    pub fn from_sources(config: Option<&str>, appearance: Option<&str>) -> Self {
        let config: RawConfig = parse("config.toml", config);
        let appearance: RawAppearance = parse("appearance.toml", appearance);

        Self {
            keys: config.keys.into(),
            theme: config.theme.overlay(appearance.theme).into(),
            ui: config.ui.overlay(appearance.ui).into(),
            styles: appearance.style.into(),
            font: appearance.font.into(),
        }
    }

    fn config_path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("config.toml"))
    }

    fn appearance_path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("appearance.toml"))
    }

    fn config_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "minuteman")
            .map(|dirs| dirs.config_dir().to_path_buf())
    }
}

/// Parses `text` as `T`, or returns `T::default()` — with a warning on stderr — if it is
/// malformed. Absent text is just the default.
fn parse<T: DeserializeOwned + Default>(file: &str, text: Option<&str>) -> T {
    match text.map(toml::from_str::<T>) {
        None => T::default(),
        Some(Ok(value)) => value,
        Some(Err(err)) => {
            eprintln!("minuteman: failed to parse {file}, using defaults: {err}");
            T::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::Mods;
    use crate::ui::GlyphSet;

    #[test]
    fn partial_config_falls_back_per_missing_field() {
        let raw: RawConfig = toml::from_str("[keys]\nmove_down = [\"n\"]\n").unwrap();
        assert_eq!(raw.keys.move_down, vec!["n".to_string()]);
        // move_up was not specified, so it keeps the default.
        assert_eq!(raw.keys.move_up, vec!["k".to_string()]);
    }

    /// The shipped `config.example.toml` must always parse and, being nothing but the built-in
    /// default keys spelled out explicitly, resolve to exactly the default keymap — catches the
    /// example silently drifting out of sync with a future default change.
    #[test]
    fn shipped_example_config_parses_and_matches_defaults() {
        let text = include_str!("../../../config.example.toml");
        let raw: RawConfig = toml::from_str(text).unwrap();
        let keys: KeyMap = raw.keys.into();
        let default_keys: KeyMap = RawKeyMap::default().into();
        assert_eq!(keys, default_keys);
    }

    /// The same drift guard for `appearance.example.toml`, covering all four of its tables.
    #[test]
    fn shipped_appearance_example_parses_and_matches_defaults() {
        let text = include_str!("../../../appearance.example.toml");
        let config = Config::from_sources(None, Some(text));
        assert_eq!(config.theme, Theme::default());
        assert_eq!(config.ui, Ui::default());
        assert_eq!(config.styles, Styles::default());
        assert_eq!(config.font, Font::default());
    }

    /// Every `[style]` element the code knows must be spelled out in the example, so a new
    /// element can't be added without documenting it.
    #[test]
    fn the_appearance_example_lists_every_style_element() {
        let text = include_str!("../../../appearance.example.toml");
        for element in crate::appearance::STYLE_ELEMENTS {
            assert!(
                text.lines()
                    .any(|l| l.starts_with(&format!("{element} = "))),
                "appearance.example.toml lacks `{element}`"
            );
        }
    }

    #[test]
    fn appearance_toml_carries_all_four_tables() {
        let config = Config::from_sources(
            None,
            Some(
                "[theme]\nname = \"classic\"\n[ui]\nglyphs = \"nerd\"\n\
                 [style]\ndoc = [\"italic\"]\n[font]\nfamily = \"Iosevka\"\nsize = 13.0\n",
            ),
        );
        assert_eq!(config.theme.selection_bg, "blue");
        assert_eq!(config.ui.glyphs, GlyphSet::Nerd);
        assert!(config.styles.doc.italic);
        assert_eq!(
            (config.font.family.as_str(), config.font.size),
            ("Iosevka", 13.0)
        );
    }

    #[test]
    fn a_theme_left_in_config_toml_still_works() {
        let config = Config::from_sources(Some("[theme]\nname = \"dracula\"\n"), None);
        // Dracula's magenta selection, not neon's violet.
        assert_eq!(config.theme.selection_bg, "magenta");
        assert_ne!(config.theme, Theme::default());
    }

    #[test]
    fn appearance_wins_over_a_legacy_theme_field_by_field() {
        let config = Config::from_sources(
            Some("[theme]\nname = \"dracula\"\nborder_fg = \"red\"\n[ui]\nglyphs = \"ascii\"\n"),
            Some("[theme]\nborder_fg = \"green\"\n"),
        );
        assert_eq!(
            config.theme.border_fg, "green",
            "appearance overrides config"
        );
        // Untouched by appearance.toml, so the config.toml palette shows through.
        assert_eq!(config.theme.selection_bg, "magenta");
        assert_eq!(config.ui.glyphs, GlyphSet::Ascii);
    }

    #[test]
    fn a_malformed_appearance_file_falls_back_to_defaults_without_touching_keys() {
        let config = Config::from_sources(
            Some("[keys]\nmove_down = [\"n\"]\n"),
            Some("this is [not valid toml"),
        );
        assert_eq!(config.theme, Theme::default());
        assert_eq!(config.styles, Styles::default());
        let down: KeyMap = RawKeyMap {
            move_down: vec!["n".into()],
            ..RawKeyMap::default()
        }
        .into();
        assert_eq!(config.keys, down, "the good config.toml still applies");
    }

    #[test]
    fn no_files_at_all_is_the_built_in_look() {
        let config = Config::from_sources(None, None);
        assert_eq!(config.theme, Theme::default());
        assert_eq!(config.styles.dir, Mods::parse(&["bold"]));
        assert_eq!(config.font, Font::default());
    }
}
