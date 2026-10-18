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
use theming::RawPluginSpec;

use crate::add_plugin_form::{self, AddPluginForm};

/// What a keystroke asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing outside the menu; keep it open.
    Stay,
    /// Switch the plugin on this row on if it is off, off if it is on.
    Toggle(usize),
    /// Ask to remove the plugin on this row (its program if bundled, its entry if the menu keeps it); the caller decides whether it may and
    /// hands the question back with [`PluginsPopup::ask_removal`].
    Remove(usize),
    /// The user said yes to the question asked for this row.
    ConfirmRemove(usize),
    /// The add form was filled in; the caller adds the plugin and either closes the form with
    /// [`PluginsPopup::close_form`] or hands back why it cannot with [`PluginsPopup::refuse`].
    Add(RawPluginSpec),
    /// The add form was given a folder; the caller reads its manifest and answers with
    /// [`PluginsPopup::fill`] or [`PluginsPopup::refuse`].
    Load(String),
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
    /// The form for adding a plugin, while it is open; it has every key until it closes.
    form: Option<AddPluginForm>,
}

impl PluginsPopup {
    pub fn new() -> Self {
        Self::default()
    }

    /// The row the cursor is on.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Puts the cursor back on a row after rows were taken away.
    pub fn clamp(&mut self, rows: usize) {
        self.cursor = self.cursor.min(rows.saturating_sub(1));
    }

    /// Shows `lines` as a question about deleting the program of the plugin on `row`.
    pub fn ask_removal(&mut self, row: usize, lines: Vec<String>) {
        self.asking = Some(Asking { row, lines });
    }

    /// The lines of the question being asked, if one is.
    pub fn question(&self) -> Option<&[String]> {
        self.asking.as_ref().map(|asking| asking.lines.as_slice())
    }

    /// The add form, while it is open.
    pub fn form(&self) -> Option<&AddPluginForm> {
        self.form.as_ref()
    }

    /// Closes the add form, back to the list.
    pub fn close_form(&mut self) {
        self.form = None;
    }

    /// Writes a read manifest into the add form's rows, or says why it cannot be.
    pub fn fill(&mut self, found: &theming::DropIn) {
        if let Some(form) = &mut self.form
            && let Err(why) = form.fill(found)
        {
            form.set_error(why);
        }
    }

    /// Keeps the add form open and says why its plugin was not added.
    pub fn refuse(&mut self, message: impl Into<String>) {
        if let Some(form) = &mut self.form {
            form.set_error(message);
        }
    }

    /// `j`/`k`/the arrows move with wraparound over `rows` rows; `Enter`, space, `h` and `l`
    /// switch the plugin under the cursor; `a` opens the form that adds one; `d` asks to remove it; `Esc`/`q` closes the
    /// menu. With no rows nothing but closing does anything. While a question is showing only `y`
    /// (yes) and `n` or `Esc` (no) are heard, so a key pressed for something else cannot delete.
    pub fn key(&mut self, code: KeyCode, rows: usize) -> Outcome {
        if let Some(form) = &mut self.form {
            return match form.key(code) {
                add_plugin_form::Key::Stay => Outcome::Stay,
                add_plugin_form::Key::Close => {
                    self.form = None;
                    Outcome::Stay
                }
                add_plugin_form::Key::Submit(spec) => Outcome::Add(spec),
                add_plugin_form::Key::Load(path) => Outcome::Load(path),
            };
        }
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
            KeyCode::Char('a') => {
                self.form = Some(AddPluginForm::new());
                Outcome::Stay
            }
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
    fn the_cursor_moves_back_onto_a_row_when_the_last_one_is_removed() {
        let mut popup = PluginsPopup::new();
        popup.key(KeyCode::Char('k'), 3);
        assert_eq!(popup.cursor(), 2);
        popup.clamp(2);
        assert_eq!(popup.cursor(), 1);
        popup.clamp(0);
        assert_eq!(popup.cursor(), 0);
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

    #[test]
    fn a_opens_the_form_which_takes_every_key_until_it_closes_or_adds() {
        let mut popup = PluginsPopup::new();
        // Even with no rows, and even for keys the list would act on.
        assert_eq!(popup.key(KeyCode::Char('a'), 0), Outcome::Stay);
        assert!(popup.form().is_some());
        // The form opens on the folder row; Tab moves on to the name.
        popup.key(KeyCode::Tab, 3);
        for c in ['q', 'd', 'j', 'a'] {
            assert_eq!(popup.key(KeyCode::Char(c), 3), Outcome::Stay);
        }
        assert_eq!(
            popup.form().unwrap().value(add_plugin_form::Row::Name),
            "qdja"
        );
        assert_eq!(popup.key(KeyCode::Esc, 3), Outcome::Stay);
        assert!(popup.form().is_none());
        assert_eq!(popup.key(KeyCode::Esc, 3), Outcome::Close);
    }

    #[test]
    fn a_filled_form_asks_to_add_and_a_refusal_keeps_it_open_with_the_reason() {
        let mut popup = PluginsPopup::new();
        popup.key(KeyCode::Char('a'), 1);
        popup.key(KeyCode::Tab, 1);
        for c in "blame".chars() {
            popup.key(KeyCode::Char(c), 1);
        }
        popup.key(KeyCode::Tab, 1);
        for c in "true".chars() {
            popup.key(KeyCode::Char(c), 1);
        }
        let Outcome::Add(spec) = popup.key(KeyCode::Enter, 1) else {
            panic!("expected an add");
        };
        assert_eq!(
            (spec.name.as_str(), spec.command.as_str()),
            ("blame", "true")
        );
        popup.refuse("that name is taken");
        assert_eq!(popup.form().unwrap().error(), Some("that name is taken"));
        popup.close_form();
        assert!(popup.form().is_none());
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
