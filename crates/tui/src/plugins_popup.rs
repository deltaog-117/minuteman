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
use crate::plugin_source::Candidate;

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
    /// `a` was pressed, or the review card was left: the caller looks for plugins to offer and
    /// answers with [`PluginsPopup::open_found`].
    OpenAdd,
    /// The user chose to pick a plugin's folder in the file browser. The menu steps aside
    /// ([`PluginsPopup::picking`]) until the caller ends the pick with [`PluginsPopup::pick_done`]
    /// or [`PluginsPopup::pick_cancel`].
    Browse,
    /// The add form or the review card asks to add the plugin; the caller adds it and either
    /// closes the add flow with [`PluginsPopup::close_form`] or hands back why it cannot with
    /// [`PluginsPopup::refuse`].
    Add(RawPluginSpec),
    /// The review card asks to start the plugin once, to see that it runs; the caller answers
    /// with [`PluginsPopup::refuse`] (why not) or [`PluginsPopup::report`] (it ran).
    Test(RawPluginSpec),
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

/// What the review card offers, left to right.
pub const REVIEW_ACTIONS: [&str; 3] = ["Start and test", "Add", "Cancel"];
const TEST: usize = 0;
const ADD: usize = 1;
const CANCEL: usize = 2;

/// The add flow: pick a plugin that was found, review it, and only for a plugin nothing found
/// describes, the form of eight rows to type.
#[derive(Debug)]
pub enum Add {
    /// The plugins found, then "Browse for a folder…" and "Manual…" as the last two rows.
    Found {
        items: Vec<Candidate>,
        cursor: usize,
        /// Why the last folder picked was not a plugin.
        notice: Option<String>,
    },
    /// What one plugin will do, with nothing to type.
    Review {
        item: Candidate,
        cursor: usize,
        /// What the last Start and test, or a refused add, said.
        result: Option<String>,
    },
    Form(AddPluginForm),
}

impl Add {
    /// How many rows the found list has: the plugins and the two ways to go on without one.
    pub fn found_rows(items: &[Candidate]) -> usize {
        items.len() + 2
    }
}

#[derive(Debug, Default)]
pub struct PluginsPopup {
    cursor: usize,
    asking: Option<Asking>,
    /// The add flow, while it is open; it has every key until it closes.
    add: Option<Add>,
    /// The file browser is choosing a plugin folder, and the menu is hidden meanwhile.
    picking: bool,
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

    /// The add flow, while it is open.
    pub fn add_flow(&self) -> Option<&Add> {
        self.add.as_ref()
    }

    /// Closes the add flow, back to the list.
    pub fn close_form(&mut self) {
        self.add = None;
    }

    /// Whether the file browser is choosing a plugin folder; the menu draws and hears nothing
    /// until [`pick_done`](Self::pick_done) or [`pick_cancel`](Self::pick_cancel).
    pub fn picking(&self) -> bool {
        self.picking
    }

    /// Opens the add flow on the plugins found.
    pub fn open_found(&mut self, items: Vec<Candidate>) {
        self.add = Some(Add::Found {
            items,
            cursor: 0,
            notice: None,
        });
    }

    /// The folder picker chose a plugin (review it) or something that is not one (say why on
    /// the found list).
    pub fn pick_done(&mut self, chosen: Result<Candidate, String>) {
        self.picking = false;
        match chosen {
            Ok(item) => self.review(item),
            Err(why) => {
                if let Some(Add::Found { notice, .. }) = &mut self.add {
                    *notice = Some(why);
                }
            }
        }
    }

    /// The pick was backed out of; the found list is as it was.
    pub fn pick_cancel(&mut self) {
        self.picking = false;
    }

    fn review(&mut self, item: Candidate) {
        // A plugin that cannot be added starts on Cancel, so `Enter` cannot be a wrong turn.
        let cursor = if item.issue.is_some() { CANCEL } else { ADD };
        self.add = Some(Add::Review {
            item,
            cursor,
            result: None,
        });
    }

    /// Writes a read manifest into the add form's rows, or says why it cannot be.
    pub fn fill(&mut self, found: &theming::DropIn) {
        if let Some(Add::Form(form)) = &mut self.add
            && let Err(why) = form.fill(found)
        {
            form.set_error(why);
        }
    }

    /// Keeps the add flow open and says why its plugin was not added or tested.
    pub fn refuse(&mut self, message: impl Into<String>) {
        self.report(message);
    }

    /// Shows a line under the review card, or the form's error line.
    pub fn report(&mut self, message: impl Into<String>) {
        match &mut self.add {
            Some(Add::Form(form)) => form.set_error(message),
            Some(Add::Review { result, .. }) => *result = Some(message.into()),
            _ => {}
        }
    }

    fn add_key(&mut self, code: KeyCode) -> Outcome {
        let Some(add) = &mut self.add else {
            return Outcome::Stay;
        };
        match add {
            Add::Form(form) => match form.key(code) {
                add_plugin_form::Key::Stay => Outcome::Stay,
                add_plugin_form::Key::Close => {
                    self.add = None;
                    Outcome::Stay
                }
                add_plugin_form::Key::Submit(spec) => Outcome::Add(spec),
                add_plugin_form::Key::Load(path) => Outcome::Load(path),
            },
            Add::Found {
                items,
                cursor,
                notice,
            } => {
                let rows = Add::found_rows(items);
                *cursor = (*cursor).min(rows - 1);
                match code {
                    KeyCode::Esc => self.add = None,
                    KeyCode::Char('j') | KeyCode::Down => {
                        *cursor = (*cursor + 1) % rows;
                        *notice = None;
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        *cursor = (*cursor + rows - 1) % rows;
                        *notice = None;
                    }
                    KeyCode::Tab => {
                        self.picking = true;
                        return Outcome::Browse;
                    }
                    KeyCode::Enter => {
                        let at = *cursor;
                        if at < items.len() {
                            let item = items[at].clone();
                            self.review(item);
                        } else if at == items.len() {
                            self.picking = true;
                            return Outcome::Browse;
                        } else {
                            self.add = Some(Add::Form(AddPluginForm::new()));
                        }
                    }
                    _ => {}
                }
                Outcome::Stay
            }
            Add::Review { item, cursor, .. } => {
                let last = REVIEW_ACTIONS.len();
                match code {
                    KeyCode::Esc => {
                        // Back to what was found, with the plugin's own row still there.
                        self.add = None;
                        return Outcome::OpenAdd;
                    }
                    KeyCode::Char('l' | 'j') | KeyCode::Right | KeyCode::Tab | KeyCode::Down => {
                        *cursor = (*cursor + 1) % last;
                    }
                    KeyCode::Char('h' | 'k') | KeyCode::Left | KeyCode::BackTab | KeyCode::Up => {
                        *cursor = (*cursor + last - 1) % last;
                    }
                    KeyCode::Char('t') => *cursor = TEST,
                    KeyCode::Enter => {
                        let spec = item.found.spec.clone();
                        return match (*cursor, &item.issue) {
                            (CANCEL, _) => {
                                self.add = None;
                                Outcome::OpenAdd
                            }
                            (_, Some(why)) => {
                                let why = why.clone();
                                self.report(why);
                                Outcome::Stay
                            }
                            (TEST, None) => Outcome::Test(spec),
                            _ => Outcome::Add(spec),
                        };
                    }
                    _ => {}
                }
                if let Add::Review { result, .. } = add {
                    *result = None;
                }
                Outcome::Stay
            }
        }
    }

    /// `j`/`k`/the arrows move with wraparound over `rows` rows; `Enter`, space, `h` and `l`
    /// switch the plugin under the cursor; `a` opens the form that adds one; `d` asks to remove it; `Esc`/`q` closes the
    /// menu. With no rows nothing but closing does anything. While a question is showing only `y`
    /// (yes) and `n` or `Esc` (no) are heard, so a key pressed for something else cannot delete.
    pub fn key(&mut self, code: KeyCode, rows: usize) -> Outcome {
        if self.picking {
            return Outcome::Stay;
        }
        if self.add.is_some() {
            return self.add_key(code);
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
            KeyCode::Char('a') => Outcome::OpenAdd,
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

    fn form_of(popup: &PluginsPopup) -> Option<&AddPluginForm> {
        match popup.add_flow() {
            Some(Add::Form(form)) => Some(form),
            _ => None,
        }
    }

    fn candidate(name: &str, issue: Option<&str>) -> Candidate {
        let dir = std::path::PathBuf::from("/p").join(name);
        Candidate {
            found: theming::DropIn {
                dir,
                spec: RawPluginSpec {
                    name: name.into(),
                    command: "true".into(),
                    ..RawPluginSpec::default()
                },
                description: String::new(),
            },
            issue: issue.map(Into::into),
        }
    }

    fn open(popup: &mut PluginsPopup, names: &[&str]) {
        assert_eq!(popup.key(KeyCode::Char('a'), 1), Outcome::OpenAdd);
        popup.open_found(names.iter().map(|n| candidate(n, None)).collect());
    }

    #[test]
    fn manual_opens_the_form_which_takes_every_key_until_it_closes_or_adds() {
        let mut popup = PluginsPopup::new();
        // Even with no rows, and even for keys the list would act on.
        assert_eq!(popup.key(KeyCode::Char('a'), 0), Outcome::OpenAdd);
        popup.open_found(vec![]);
        assert!(form_of(&popup).is_none());
        // With nothing found the rows are Browse and Manual.
        popup.key(KeyCode::Down, 3);
        assert_eq!(popup.key(KeyCode::Enter, 3), Outcome::Stay);
        assert!(form_of(&popup).is_some());
        // The form opens on the folder row; Tab moves on to the name.
        popup.key(KeyCode::Tab, 3);
        for c in ['q', 'd', 'j', 'a'] {
            assert_eq!(popup.key(KeyCode::Char(c), 3), Outcome::Stay);
        }
        assert_eq!(
            form_of(&popup).unwrap().value(add_plugin_form::Row::Name),
            "qdja"
        );
        assert_eq!(popup.key(KeyCode::Esc, 3), Outcome::Stay);
        assert!(form_of(&popup).is_none());
        assert_eq!(popup.key(KeyCode::Esc, 3), Outcome::Close);
    }

    #[test]
    fn a_filled_form_asks_to_add_and_a_refusal_keeps_it_open_with_the_reason() {
        let mut popup = PluginsPopup::new();
        open(&mut popup, &[]);
        popup.key(KeyCode::Down, 1);
        popup.key(KeyCode::Enter, 1);
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
        assert_eq!(form_of(&popup).unwrap().error(), Some("that name is taken"));
        popup.close_form();
        assert!(form_of(&popup).is_none());
    }

    #[test]
    fn enter_on_a_found_plugin_reviews_it_and_enter_again_adds_it() {
        let mut popup = PluginsPopup::new();
        open(&mut popup, &["blame", "wc"]);
        popup.key(KeyCode::Char('j'), 1);
        assert_eq!(popup.key(KeyCode::Enter, 1), Outcome::Stay);
        let Some(Add::Review { item, cursor, .. }) = popup.add_flow() else {
            panic!("expected the review card");
        };
        assert_eq!((item.found.spec.name.as_str(), *cursor), ("wc", ADD));
        let Outcome::Add(spec) = popup.key(KeyCode::Enter, 1) else {
            panic!("expected an add");
        };
        assert_eq!(spec.name, "wc");
    }

    #[test]
    fn the_review_card_tests_with_t_and_a_cancel_returns_to_the_found_list() {
        let mut popup = PluginsPopup::new();
        open(&mut popup, &["blame"]);
        popup.key(KeyCode::Enter, 1);
        popup.key(KeyCode::Char('t'), 1);
        assert!(matches!(popup.key(KeyCode::Enter, 1), Outcome::Test(_)));
        popup.report("blame started");
        let Some(Add::Review { result, .. }) = popup.add_flow() else {
            panic!("expected the review card");
        };
        assert_eq!(result.as_deref(), Some("blame started"));
        popup.key(KeyCode::Tab, 1);
        popup.key(KeyCode::Tab, 1);
        assert_eq!(popup.key(KeyCode::Enter, 1), Outcome::OpenAdd);
        assert!(popup.add_flow().is_none());
    }

    #[test]
    fn a_plugin_with_an_issue_cannot_be_tested_or_added() {
        let mut popup = PluginsPopup::new();
        popup.open_found(vec![candidate("blame", Some("key b is taken"))]);
        popup.key(KeyCode::Enter, 1);
        // It starts on Cancel, and moving to the other two buttons only reports the issue.
        for _ in 0..2 {
            popup.key(KeyCode::Tab, 1);
            assert_eq!(popup.key(KeyCode::Enter, 1), Outcome::Stay);
            let Some(Add::Review { result, .. }) = popup.add_flow() else {
                panic!("expected the review card");
            };
            assert_eq!(result.as_deref(), Some("key b is taken"));
        }
    }

    #[test]
    fn browsing_hides_the_menu_until_a_folder_is_chosen_or_the_pick_is_dropped() {
        for via in [KeyCode::Tab, KeyCode::Enter] {
            let mut popup = PluginsPopup::new();
            open(&mut popup, &[]);
            assert_eq!(popup.key(via, 1), Outcome::Browse);
            assert!(popup.picking());
            // Heard by nothing while the browser has the keyboard.
            assert_eq!(popup.key(KeyCode::Esc, 1), Outcome::Stay);
            popup.pick_cancel();
            assert!(!popup.picking());
            assert!(matches!(popup.add_flow(), Some(Add::Found { .. })));
            popup.key(via, 1);
            popup.pick_done(Ok(candidate("blame", None)));
            assert!(!popup.picking());
            assert!(matches!(popup.add_flow(), Some(Add::Review { .. })));
        }
    }

    #[test]
    fn a_folder_that_is_not_a_plugin_is_explained_on_the_found_list() {
        let mut popup = PluginsPopup::new();
        open(&mut popup, &[]);
        popup.key(KeyCode::Tab, 1);
        popup.pick_done(Err("no plugin.toml in there".into()));
        let Some(Add::Found { notice, .. }) = popup.add_flow() else {
            panic!("expected the found list");
        };
        assert_eq!(notice.as_deref(), Some("no plugin.toml in there"));
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
