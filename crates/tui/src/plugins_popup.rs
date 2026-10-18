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

//! The plugin menu's state: which row the cursor is on, and what a key means. Like
//! `settings_popup` this is pure — no `Frame`, no terminal, no processes — so `overlay_view` is
//! the only place that draws it and `app` is the only place that starts or stops a plugin.

use crossterm::event::KeyCode;

/// What a keystroke asks the caller to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing outside the menu; keep it open.
    Stay,
    /// Switch the plugin on this row on if it is off, off if it is on.
    Toggle(usize),
    /// Ask to delete the program of the plugin on this row; the caller decides whether it may and
    /// hands the question back with [`PluginsPopup::ask_removal`].
    Remove(usize),
    /// The user said yes to the question asked for this row.
    ConfirmRemove(usize),
    Close,
}

/// The question waiting for a `y`.
#[derive(Debug)]
struct Asking {
    row: usize,
    lines: Vec<String>,
}

#[derive(Debug, Default)]
pub struct PluginsPopup {
    cursor: usize,
    asking: Option<Asking>,
}

impl PluginsPopup {
    pub fn new() -> Self {
        Self::default()
    }

    /// The row the cursor is on.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Shows `lines` as a question about deleting the program of the plugin on `row`.
    pub fn ask_removal(&mut self, row: usize, lines: Vec<String>) {
        self.asking = Some(Asking { row, lines });
    }

    /// The lines of the question being asked, if one is.
    pub fn question(&self) -> Option<&[String]> {
        self.asking.as_ref().map(|asking| asking.lines.as_slice())
    }

    /// `j`/`k`/the arrows move with wraparound over `rows` rows; `Enter`, space, `h` and `l`
    /// switch the plugin under the cursor; `d` asks to delete its program; `Esc`/`q` closes the
    /// menu. With no rows nothing but closing does anything. While a question is showing only `y`
    /// (yes) and `n` or `Esc` (no) are heard, so a key pressed for something else cannot delete.
    pub fn key(&mut self, code: KeyCode, rows: usize) -> Outcome {
        if let Some(asking) = &self.asking {
            return match code {
                KeyCode::Char('y' | 'Y') => {
                    let row = asking.row;
                    self.asking = None;
                    Outcome::ConfirmRemove(row)
                }
                KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                    self.asking = None;
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            };
        }
        match code {
            KeyCode::Esc | KeyCode::Char('q') => Outcome::Close,
            _ if rows == 0 => Outcome::Stay,
            KeyCode::Char('j') | KeyCode::Down => {
                self.cursor = (self.cursor + 1) % rows;
                Outcome::Stay
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = (self.cursor + rows - 1) % rows;
                Outcome::Stay
            }
            KeyCode::Char('d') => Outcome::Remove(self.cursor.min(rows - 1)),
            KeyCode::Enter | KeyCode::Char(' ' | 'h' | 'l') | KeyCode::Left | KeyCode::Right => {
                Outcome::Toggle(self.cursor.min(rows - 1))
            }
            _ => Outcome::Stay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn j_and_k_move_with_wraparound() {
        let mut popup = PluginsPopup::new();
        assert_eq!(popup.key(KeyCode::Char('k'), 3), Outcome::Stay);
        assert_eq!(popup.cursor(), 2);
        assert_eq!(popup.key(KeyCode::Char('j'), 3), Outcome::Stay);
        assert_eq!(popup.cursor(), 0);
    }

    #[test]
    fn enter_space_and_the_side_keys_toggle_the_row_under_the_cursor() {
        let mut popup = PluginsPopup::new();
        popup.key(KeyCode::Down, 4);
        for code in [
            KeyCode::Enter,
            KeyCode::Char(' '),
            KeyCode::Char('h'),
            KeyCode::Char('l'),
            KeyCode::Left,
            KeyCode::Right,
        ] {
            assert_eq!(popup.key(code, 4), Outcome::Toggle(1));
        }
    }

    #[test]
    fn esc_and_q_close_even_with_no_rows_and_other_keys_do_nothing() {
        let mut popup = PluginsPopup::new();
        assert_eq!(popup.key(KeyCode::Esc, 0), Outcome::Close);
        assert_eq!(popup.key(KeyCode::Char('q'), 0), Outcome::Close);
        assert_eq!(popup.key(KeyCode::Enter, 0), Outcome::Stay);
        assert_eq!(popup.key(KeyCode::Char('z'), 3), Outcome::Stay);
    }

    #[test]
    fn d_asks_and_only_y_confirms_while_the_question_shows() {
        let mut popup = PluginsPopup::new();
        popup.key(KeyCode::Down, 4);
        assert_eq!(popup.key(KeyCode::Char('d'), 4), Outcome::Remove(1));
        popup.ask_removal(1, vec!["Delete?".into()]);
        assert!(popup.question().is_some());
        for code in [
            KeyCode::Enter,
            KeyCode::Char(' '),
            KeyCode::Char('q'),
            KeyCode::Char('d'),
        ] {
            assert_eq!(popup.key(code, 4), Outcome::Stay);
            assert!(popup.question().is_some());
        }
        assert_eq!(popup.key(KeyCode::Char('y'), 4), Outcome::ConfirmRemove(1));
        assert!(popup.question().is_none());
    }

    #[test]
    fn n_and_esc_drop_the_question_without_closing_the_menu() {
        for code in [KeyCode::Char('n'), KeyCode::Esc] {
            let mut popup = PluginsPopup::new();
            popup.ask_removal(0, vec!["Delete?".into()]);
            assert_eq!(popup.key(code, 2), Outcome::Stay);
            assert!(popup.question().is_none());
            assert_eq!(popup.key(KeyCode::Esc, 2), Outcome::Close);
        }
    }

    proptest! {
        #[test]
        fn the_cursor_and_any_toggle_stay_in_range(
            rows in 0usize..8,
            moves in proptest::collection::vec(0u8..4, 0..100),
        ) {
            let mut popup = PluginsPopup::new();
            for m in moves {
                let code = [KeyCode::Char('j'), KeyCode::Char('k'), KeyCode::Enter, KeyCode::Char('x')][usize::from(m)];
                if let Outcome::Toggle(row) = popup.key(code, rows) {
                    prop_assert!(row < rows);
                }
                prop_assert!(rows == 0 || popup.cursor() < rows);
            }
        }
    }
}
