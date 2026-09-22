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

//! The settings popup's state: which row the cursor is on, and what a key means. Like
//! `context_menu`, this is pure — no `Frame`, no terminal — so `overlay_view` is the only place
//! that draws it and `main` is the only place that applies what it asks for.
//!
//! The popup itself never touches `Config`; it only reports which row was cycled and leaves
//! applying that (and deciding what it means for the running session) to the caller. This is
//! session-only, in-memory state — it is never written back to `config.toml`/`appearance.toml`.

use crossterm::event::KeyCode;
use theming::ColumnLayout;

/// The built-in palettes the "Theme" row cycles through, in order.
pub const THEME_NAMES: [&str; 3] = ["neon", "classic", "dracula"];

/// The current value of every row, for `overlay_view::render_settings` to draw. Built fresh each
/// frame from `main`'s live session state, not stored on the popup itself.
pub struct SettingsView {
    pub columns: ColumnLayout,
    pub theme_name: &'static str,
    pub show_hud: bool,
    pub show_command_bar: bool,
}

impl SettingsView {
    /// The row's value as the popup displays it.
    pub fn value(&self, row: Row) -> String {
        match row {
            Row::Columns => match self.columns {
                ColumnLayout::ThreePane => "three-pane".to_string(),
                ColumnLayout::TwoPane => "two-pane".to_string(),
            },
            Row::Theme => self.theme_name.to_string(),
            Row::Hud => on_off(self.show_hud),
            Row::CommandBar => on_off(self.show_command_bar),
        }
    }
}

fn on_off(value: bool) -> String {
    if value { "on" } else { "off" }.to_string()
}

/// One row of the popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Columns,
    Theme,
    Hud,
    CommandBar,
}

const ROWS: [Row; 4] = [Row::Columns, Row::Theme, Row::Hud, Row::CommandBar];

impl Row {
    pub fn label(self) -> &'static str {
        match self {
            Row::Columns => "Columns",
            Row::Theme => "Theme",
            Row::Hud => "HUD",
            Row::CommandBar => "Command bar",
        }
    }
}

/// What a keystroke asks the caller to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing outside the popup; keep it open.
    Stay,
    /// Cycle this row's value to the next one.
    Cycle(Row),
    Close,
}

pub struct SettingsPopup {
    cursor: usize,
}

impl SettingsPopup {
    pub fn new() -> Self {
        Self { cursor: 0 }
    }

    pub fn rows(&self) -> &'static [Row] {
        &ROWS
    }

    /// The row the cursor is currently on.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// `j`/`k`/the arrows move the cursor with wraparound; `h`/`l`/enter/left/right cycle the row
    /// under it; `Esc`/`q` closes the popup. Any other key does nothing.
    pub fn key(&mut self, code: KeyCode) -> Outcome {
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.cursor = (self.cursor + 1) % ROWS.len();
                Outcome::Stay
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = (self.cursor + ROWS.len() - 1) % ROWS.len();
                Outcome::Stay
            }
            KeyCode::Char('h' | 'l') | KeyCode::Left | KeyCode::Right | KeyCode::Enter => {
                Outcome::Cycle(ROWS[self.cursor])
            }
            KeyCode::Esc | KeyCode::Char('q') => Outcome::Close,
            _ => Outcome::Stay,
        }
    }
}

impl Default for SettingsPopup {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn j_and_k_move_the_cursor_with_wraparound() {
        let mut popup = SettingsPopup::new();
        assert_eq!(popup.cursor(), 0);
        assert_eq!(popup.key(KeyCode::Char('j')), Outcome::Stay);
        assert_eq!(popup.cursor(), 1);
        assert_eq!(popup.key(KeyCode::Char('k')), Outcome::Stay);
        assert_eq!(popup.cursor(), 0);
        assert_eq!(popup.key(KeyCode::Char('k')), Outcome::Stay);
        assert_eq!(
            popup.cursor(),
            ROWS.len() - 1,
            "up from the top wraps to the bottom"
        );
    }

    #[test]
    fn h_l_and_enter_all_cycle_the_row_under_the_cursor() {
        let mut popup = SettingsPopup::new();
        assert_eq!(popup.key(KeyCode::Char('l')), Outcome::Cycle(Row::Columns));
        assert_eq!(popup.key(KeyCode::Char('h')), Outcome::Cycle(Row::Columns));
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Cycle(Row::Columns));
        popup.key(KeyCode::Char('j'));
        assert_eq!(popup.key(KeyCode::Char('l')), Outcome::Cycle(Row::Theme));
    }

    #[test]
    fn esc_and_q_close_it() {
        let mut popup = SettingsPopup::new();
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Close);
        assert_eq!(popup.key(KeyCode::Char('q')), Outcome::Close);
    }

    #[test]
    fn an_unbound_key_does_nothing() {
        let mut popup = SettingsPopup::new();
        assert_eq!(popup.key(KeyCode::Tab), Outcome::Stay);
        assert_eq!(popup.cursor(), 0);
    }

    proptest::proptest! {
        /// Whatever sequence of moves the cursor sees, it always names one of the four rows.
        #[test]
        fn the_cursor_always_stays_in_bounds(moves in proptest::collection::vec(0u8..2, 0..200)) {
            let mut popup = SettingsPopup::new();
            for m in moves {
                let code = if m == 0 { KeyCode::Char('j') } else { KeyCode::Char('k') };
                popup.key(code);
                proptest::prop_assert!(popup.cursor() < ROWS.len());
            }
        }
    }
}
