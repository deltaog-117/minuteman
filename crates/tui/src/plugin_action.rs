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

//! Which of a plugin's `run_action` requests the browser runs. An accepted one goes through the
//! same handling as a key bound to that action (see `Browse`), so a plugin's `paste` is the `p`
//! key's paste, confirmations and all. It is accepted only where that key would reach that
//! handling: in plain browsing, with no popup, view, chord, prompt, shell or job ahead of it. The
//! user's own state always wins, and the plugin is told what stood in the way.

use crossterm::event::KeyCode;
use plugin_host::ActionRequest;
use theming::Action;

use crate::hud::Mode;

/// What one turn of the browser's own key handling acts on.
pub enum Browse {
    /// A key press, resolved through the keymap there.
    Key(KeyCode),
    /// An action a plugin asked for, with its request to answer once it has run.
    Plugin(Action, ActionRequest),
}

/// The name of every action a plugin may run, for `list_actions`.
pub fn runnable_names() -> impl Iterator<Item = String> {
    Action::ALL
        .into_iter()
        .filter(|action| !action.waits_for_key())
        .map(|action| action.name().to_string())
}

/// Takes `request` for the browser to run, or answers it with the reason it cannot run now.
/// `mode` is what the next key would do and `modal_open` whether a popup or view owns the keys.
pub fn accept(request: ActionRequest, mode: Mode, modal_open: bool) -> Option<Browse> {
    match check(request.name(), mode, modal_open) {
        Ok(action) => Some(Browse::Plugin(action, request)),
        Err(reason) => {
            request.refuse(reason);
            None
        }
    }
}

fn check(name: &str, mode: Mode, modal_open: bool) -> Result<Action, &'static str> {
    let action = Action::from_name(name).ok_or("no such action; `list_actions` names them")?;
    if action.waits_for_key() {
        return Err("it only starts a chord, which waits for a second key");
    }
    if modal_open {
        return Err("a popup or view is open");
    }
    match mode {
        Mode::Normal => Ok(action),
        Mode::Shell => Err("a shell pane has the keyboard"),
        Mode::Leader | Mode::Resize | Mode::Move | Mode::BookmarkJump | Mode::BookmarkSet => {
            Err("a key chord is waiting for its next key")
        }
        Mode::PickPlugin => Err("the plugin menu is choosing a folder"),
        Mode::Prompt { .. } => Err("a prompt is open"),
        Mode::Busy => Err("a job is running"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY_OTHER_MODE: [Mode; 10] = [
        Mode::Shell,
        Mode::Leader,
        Mode::Resize,
        Mode::Move,
        Mode::BookmarkJump,
        Mode::BookmarkSet,
        Mode::PickPlugin,
        Mode::Busy,
        Mode::Prompt {
            label: "rename",
            destructive: false,
            text_input: true,
        },
        Mode::Prompt {
            label: "delete",
            destructive: true,
            text_input: false,
        },
    ];

    #[test]
    fn plain_browsing_runs_every_listed_action() {
        for name in runnable_names() {
            let action = check(&name, Mode::Normal, false);
            assert_eq!(action.map(Action::name), Ok(name.as_str()));
        }
    }

    #[test]
    fn anything_ahead_of_the_browser_refuses_every_action() {
        for name in runnable_names() {
            assert!(
                check(&name, Mode::Normal, true).is_err(),
                "{name} under a popup"
            );
            for mode in EVERY_OTHER_MODE {
                assert!(check(&name, mode, false).is_err(), "{name} in {mode:?}");
            }
        }
    }

    #[test]
    fn chord_starters_are_not_listed_and_are_refused_even_in_plain_browsing() {
        let listed: Vec<String> = runnable_names().collect();
        for name in ["bookmark_jump", "bookmark_set", "leader"] {
            assert!(!listed.iter().any(|n| n == name));
            assert!(check(name, Mode::Normal, false).is_err());
        }
        assert_eq!(listed.len(), Action::ALL.len() - 3);
    }

    #[test]
    fn an_unknown_name_says_where_the_names_are() {
        assert_eq!(
            check("Paste", Mode::Normal, false),
            Err("no such action; `list_actions` names them")
        );
    }

    #[test]
    fn the_reason_names_what_is_in_the_way() {
        assert_eq!(check("paste", Mode::Busy, false), Err("a job is running"));
        assert_eq!(
            check("paste", Mode::Shell, false),
            Err("a shell pane has the keyboard")
        );
        assert_eq!(
            check("paste", Mode::Normal, true),
            Err("a popup or view is open")
        );
    }
}
