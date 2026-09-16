use std::path::PathBuf;

use serde::Deserialize;

use crate::keymap::{KeyMap, RawKeyMap};
use crate::theme::Theme;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawConfig {
    keys: RawKeyMap,
    theme: Theme,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub keys: KeyMap,
    pub theme: Theme,
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
            theme: raw.theme,
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
        assert_eq!(raw.theme.selection_bg, "blue");
    }
}
