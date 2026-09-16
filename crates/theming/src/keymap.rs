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

use std::collections::HashMap;

use crossterm::event::KeyCode;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    MoveDown,
    MoveUp,
    Enter,
    Leave,
    Quit,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RawKeyMap {
    pub move_down: Vec<String>,
    pub move_up: Vec<String>,
    pub enter: Vec<String>,
    pub leave: Vec<String>,
    pub quit: Vec<String>,
}

impl Default for RawKeyMap {
    fn default() -> Self {
        Self {
            move_down: vec!["j".into()],
            move_up: vec!["k".into()],
            enter: vec!["l".into(), "enter".into()],
            leave: vec!["h".into()],
            quit: vec!["q".into()],
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct KeyMap {
    bindings: HashMap<KeyCode, Action>,
}

impl KeyMap {
    pub fn resolve(&self, code: KeyCode) -> Option<Action> {
        self.bindings.get(&code).copied()
    }
}

impl From<RawKeyMap> for KeyMap {
    fn from(raw: RawKeyMap) -> Self {
        let mut bindings = HashMap::new();
        let mut bind_all = |keys: &[String], action: Action| {
            for key in keys {
                if let Some(code) = parse_key(key) {
                    bindings.insert(code, action);
                }
            }
        };

        bind_all(&raw.move_down, Action::MoveDown);
        bind_all(&raw.move_up, Action::MoveUp);
        bind_all(&raw.enter, Action::Enter);
        bind_all(&raw.leave, Action::Leave);
        bind_all(&raw.quit, Action::Quit);

        Self { bindings }
    }
}

/// Parses a single config key string (e.g. `"j"`, `"enter"`, `"space"`) into a crossterm
/// `KeyCode`. Unrecognised or multi-character (non-named) strings are skipped rather than
/// causing a hard error — a typo in one binding shouldn't crash startup.
fn parse_key(s: &str) -> Option<KeyCode> {
    match s.to_lowercase().as_str() {
        "enter" | "return" => Some(KeyCode::Enter),
        "esc" | "escape" => Some(KeyCode::Esc),
        "space" => Some(KeyCode::Char(' ')),
        "tab" => Some(KeyCode::Tab),
        "backspace" => Some(KeyCode::Backspace),
        other => {
            let mut chars = other.chars();
            let first = chars.next()?;
            if chars.next().is_none() {
                Some(KeyCode::Char(first))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bindings_resolve_expected_actions() {
        let keymap: KeyMap = RawKeyMap::default().into();
        assert_eq!(keymap.resolve(KeyCode::Char('j')), Some(Action::MoveDown));
        assert_eq!(keymap.resolve(KeyCode::Char('k')), Some(Action::MoveUp));
        assert_eq!(keymap.resolve(KeyCode::Char('l')), Some(Action::Enter));
        assert_eq!(keymap.resolve(KeyCode::Enter), Some(Action::Enter));
        assert_eq!(keymap.resolve(KeyCode::Char('h')), Some(Action::Leave));
        assert_eq!(keymap.resolve(KeyCode::Char('q')), Some(Action::Quit));
        assert_eq!(keymap.resolve(KeyCode::Char('z')), None);
    }

    #[test]
    fn unrecognised_key_strings_are_skipped_not_fatal() {
        assert_eq!(parse_key("ctrl-something-weird"), None);
        assert_eq!(parse_key(""), None);
    }
}
