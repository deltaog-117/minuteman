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

//! The `Alt` layer for the mini-shell box: one held modifier that drives every shell command,
//! so no plain key ever has to be taken away from the shell running inside the pane.
//!
//! Keys are hardcoded rather than routed through `KeyMap`, which has no modifier awareness — the
//! same precedent as the `space` leader chord's own `r`/`m`/`hjkl`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::shell_layout::NudgeDir;

/// One thing an `Alt`-prefixed key asks the shell box to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AltCommand {
    /// `Alt+h/j/k/l`: nudge the whole box.
    Move(NudgeDir),
    /// `Alt+a/s/d/f`: grow the box's left/bottom/top/right edge outward.
    Grow(NudgeDir),
    /// `Alt+z/x/c/v`: move focus to the pane on that side.
    Focus(NudgeDir),
    /// `Alt+Shift+S`: split the focused pane into a new shell (or open the first one).
    Split,
    /// `Alt+t`: snap the box to the top centre of the screen.
    SnapTop,
    /// `Alt+b`: snap the box to the bottom centre of the screen.
    SnapBottom,
    /// `Alt+q`: close every shell.
    CloseAll,
    /// `Alt+e`: close only the pane under the pointer.
    CloseHovered,
}

/// The command `key` means, or `None` when it isn't an `Alt` command at all — in which case the
/// caller must treat it like any other key.
///
/// Split is `Alt+Shift+S`, not `Alt+s`, because `Alt+s` already grows the box downward; the two
/// are told apart by the character's case, so a bare `Alt+s` can never split by accident.
/// `Ctrl+Alt` combinations are left alone: they are not part of this scheme.
pub fn parse(key: KeyEvent) -> Option<AltCommand> {
    if !key.modifiers.contains(KeyModifiers::ALT) || key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    Some(match c {
        'h' => AltCommand::Move(NudgeDir::Left),
        'j' => AltCommand::Move(NudgeDir::Down),
        'k' => AltCommand::Move(NudgeDir::Up),
        'l' => AltCommand::Move(NudgeDir::Right),
        'a' => AltCommand::Grow(NudgeDir::Left),
        's' => AltCommand::Grow(NudgeDir::Down),
        'd' => AltCommand::Grow(NudgeDir::Up),
        'f' => AltCommand::Grow(NudgeDir::Right),
        'z' => AltCommand::Focus(NudgeDir::Left),
        'x' => AltCommand::Focus(NudgeDir::Down),
        'c' => AltCommand::Focus(NudgeDir::Up),
        'v' => AltCommand::Focus(NudgeDir::Right),
        'S' => AltCommand::Split,
        't' => AltCommand::SnapTop,
        'b' => AltCommand::SnapBottom,
        'q' => AltCommand::CloseAll,
        'e' => AltCommand::CloseHovered,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alt(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }

    #[test]
    fn every_documented_key_maps_to_its_command() {
        let table = [
            ('h', AltCommand::Move(NudgeDir::Left)),
            ('j', AltCommand::Move(NudgeDir::Down)),
            ('k', AltCommand::Move(NudgeDir::Up)),
            ('l', AltCommand::Move(NudgeDir::Right)),
            ('a', AltCommand::Grow(NudgeDir::Left)),
            ('s', AltCommand::Grow(NudgeDir::Down)),
            ('d', AltCommand::Grow(NudgeDir::Up)),
            ('f', AltCommand::Grow(NudgeDir::Right)),
            ('z', AltCommand::Focus(NudgeDir::Left)),
            ('x', AltCommand::Focus(NudgeDir::Down)),
            ('c', AltCommand::Focus(NudgeDir::Up)),
            ('v', AltCommand::Focus(NudgeDir::Right)),
            ('S', AltCommand::Split),
            ('t', AltCommand::SnapTop),
            ('b', AltCommand::SnapBottom),
            ('q', AltCommand::CloseAll),
            ('e', AltCommand::CloseHovered),
        ];
        for (c, expected) in table {
            assert_eq!(parse(alt(c)), Some(expected), "Alt+{c}");
        }
    }

    #[test]
    fn a_key_without_alt_is_never_a_command() {
        for c in "hjklasdfzxcvStbqe".chars() {
            let plain = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
            assert_eq!(parse(plain), None, "plain {c}");
        }
    }

    #[test]
    fn alt_shift_s_splits_but_a_bare_alt_s_only_grows() {
        let shifted = KeyEvent::new(KeyCode::Char('S'), KeyModifiers::ALT | KeyModifiers::SHIFT);
        assert_eq!(parse(shifted), Some(AltCommand::Split));
        assert_eq!(parse(alt('s')), Some(AltCommand::Grow(NudgeDir::Down)));
    }

    #[test]
    fn ctrl_alt_combinations_are_left_alone() {
        let ctrl_alt = KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::ALT | KeyModifiers::CONTROL,
        );
        assert_eq!(parse(ctrl_alt), None);
    }

    #[test]
    fn unbound_alt_keys_and_non_character_keys_are_not_commands() {
        assert_eq!(parse(alt('m')), None);
        assert_eq!(parse(alt('A')), None);
        assert_eq!(
            parse(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
            None
        );
    }
}
