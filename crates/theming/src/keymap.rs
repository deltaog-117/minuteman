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
    /// Mark the selection to be duplicated on the next `Paste`.
    Yank,
    /// Mark the selection to be relocated on the next `Paste`.
    Cut,
    Paste,
    /// Permanently delete the selection (asks for confirmation first — there is no trash yet).
    Delete,
    Rename,
    /// Prompts for a name; a trailing `/` creates a directory, otherwise a file.
    Create,
    /// Suspends the TUI and drops into a real, interactive `$SHELL` in the current directory.
    Shell,
    /// Opens the incremental filename-search prompt (`/`).
    Search,
    /// Opens the `:`-command prompt.
    Command,
    /// Toggles the current entry's mark, Ranger-style: pressed once per file to queue it for a
    /// later bulk action (e.g. `Delete`) instead of acting on it immediately.
    Select,
    /// Starts a chord. Inert with no shell pane open; while one is, the next key is `r` (enters
    /// a resize mode where `hjkl`/arrows repeatedly nudge the focused pane's divider, or the box
    /// itself if there's none along that axis, until `Esc` or any other key ends it), `m` (same,
    /// but for the box's position), or `t` (an immediate one-shot toggle of the focused pane's
    /// split orientation, side-by-side <-> stacked).
    Leader,
    /// While any shell pane is open, toggles whether keystrokes go to the focused pane or drive
    /// the browser — lets the pane(s) stay open and visible while browsing.
    ShellFocus,
    /// Splits the focused pane side by side (left/right); the new pane's shell starts in the
    /// current directory and takes focus.
    ShellSplitHorizontal,
    /// Splits the focused pane stacked (top/bottom); the new pane's shell starts in the current
    /// directory and takes focus.
    ShellSplitVertical,
    /// Cycles keyboard focus to the next pane — for when a mouse isn't available; clicking a
    /// pane directly focuses it too.
    ShellPaneNext,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RawKeyMap {
    pub move_down: Vec<String>,
    pub move_up: Vec<String>,
    pub enter: Vec<String>,
    pub leave: Vec<String>,
    pub quit: Vec<String>,
    pub yank: Vec<String>,
    pub cut: Vec<String>,
    pub paste: Vec<String>,
    pub delete: Vec<String>,
    pub rename: Vec<String>,
    pub create: Vec<String>,
    pub shell: Vec<String>,
    pub search: Vec<String>,
    pub command: Vec<String>,
    pub select: Vec<String>,
    pub leader: Vec<String>,
    pub shell_focus: Vec<String>,
    pub shell_split_horizontal: Vec<String>,
    pub shell_split_vertical: Vec<String>,
    pub shell_pane_next: Vec<String>,
}

impl Default for RawKeyMap {
    fn default() -> Self {
        Self {
            move_down: vec!["j".into()],
            move_up: vec!["k".into()],
            enter: vec!["l".into(), "enter".into()],
            leave: vec!["h".into()],
            quit: vec!["q".into()],
            yank: vec!["y".into()],
            cut: vec!["m".into()],
            paste: vec!["p".into()],
            delete: vec!["d".into()],
            rename: vec!["r".into()],
            create: vec!["n".into()],
            shell: vec!["s".into()],
            search: vec!["/".into()],
            command: vec![":".into()],
            select: vec!["v".into()],
            leader: vec!["space".into()],
            shell_focus: vec!["tab".into()],
            shell_split_horizontal: vec!["%".into()],
            shell_split_vertical: vec!["\"".into()],
            shell_pane_next: vec!["o".into()],
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
        bind_all(&raw.yank, Action::Yank);
        bind_all(&raw.cut, Action::Cut);
        bind_all(&raw.paste, Action::Paste);
        bind_all(&raw.delete, Action::Delete);
        bind_all(&raw.rename, Action::Rename);
        bind_all(&raw.create, Action::Create);
        bind_all(&raw.shell, Action::Shell);
        bind_all(&raw.search, Action::Search);
        bind_all(&raw.command, Action::Command);
        bind_all(&raw.select, Action::Select);
        bind_all(&raw.leader, Action::Leader);
        bind_all(&raw.shell_focus, Action::ShellFocus);
        bind_all(&raw.shell_split_horizontal, Action::ShellSplitHorizontal);
        bind_all(&raw.shell_split_vertical, Action::ShellSplitVertical);
        bind_all(&raw.shell_pane_next, Action::ShellPaneNext);

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
        assert_eq!(keymap.resolve(KeyCode::Char('y')), Some(Action::Yank));
        assert_eq!(keymap.resolve(KeyCode::Char('m')), Some(Action::Cut));
        assert_eq!(keymap.resolve(KeyCode::Char('p')), Some(Action::Paste));
        assert_eq!(keymap.resolve(KeyCode::Char('d')), Some(Action::Delete));
        assert_eq!(keymap.resolve(KeyCode::Char('r')), Some(Action::Rename));
        assert_eq!(keymap.resolve(KeyCode::Char('n')), Some(Action::Create));
        assert_eq!(keymap.resolve(KeyCode::Char('s')), Some(Action::Shell));
        assert_eq!(keymap.resolve(KeyCode::Char('/')), Some(Action::Search));
        assert_eq!(keymap.resolve(KeyCode::Char(':')), Some(Action::Command));
        assert_eq!(keymap.resolve(KeyCode::Char('v')), Some(Action::Select));
        assert_eq!(keymap.resolve(KeyCode::Char(' ')), Some(Action::Leader));
        assert_eq!(keymap.resolve(KeyCode::Tab), Some(Action::ShellFocus));
        assert_eq!(
            keymap.resolve(KeyCode::Char('%')),
            Some(Action::ShellSplitHorizontal)
        );
        assert_eq!(
            keymap.resolve(KeyCode::Char('"')),
            Some(Action::ShellSplitVertical)
        );
        assert_eq!(
            keymap.resolve(KeyCode::Char('o')),
            Some(Action::ShellPaneNext)
        );
        assert_eq!(keymap.resolve(KeyCode::Char('z')), None);
    }

    #[test]
    fn unrecognised_key_strings_are_skipped_not_fatal() {
        assert_eq!(parse_key("ctrl-something-weird"), None);
        assert_eq!(parse_key(""), None);
    }
}
