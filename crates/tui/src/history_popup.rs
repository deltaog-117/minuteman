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

//! The history popup's state: the entries that can be taken back, then the ones that can be done
//! again, and which row the cursor is on. Like `extract_popup` this is pure, with no `Frame` and
//! no filesystem, so `overlay_view` is the only place that draws it and `main` the only place
//! that acts on what it asks for.
//!
//! Entries depend on the ones recorded after them, so a row never undoes one entry on its own:
//! choosing a row asks for every entry down to and including it ([`Walk`]), newest first, which
//! is the same order repeated `undo` presses would take. `u` is the exception the user asks for
//! by name: it runs just that one row ([`Pick`]) and leaves the entries around it alone.

use crossterm::event::KeyCode;
use file_ops::history::{EntryId, Summary};

/// Which stack a row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Undo,
    Redo,
}

impl Side {
    pub fn word(self) -> &'static str {
        match self {
            Side::Undo => "undo",
            Side::Redo => "redo",
        }
    }
}

/// What choosing a row asks for: `entries` steps of `side`, in order, stopping at the first that
/// does not finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub side: Side,
    /// At least one.
    pub entries: usize,
}

/// What `u` on a row asks for: that one entry, `depth` places from the nearest one of `side`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pick {
    pub side: Side,
    /// At least one; the nearest entry is 1, as in [`Row::depth`].
    pub depth: usize,
}

/// What a keystroke asks the caller to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Stay,
    Close,
    Run(Walk),
    Only(Pick),
}

/// One row as it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row<'a> {
    pub side: Side,
    /// How many entries choosing this row takes in all, counting from the nearest one.
    pub depth: usize,
    pub label: &'a str,
}

pub struct HistoryPopup {
    undo: Vec<String>,
    redo: Vec<String>,
    /// Each row's entry id, as in [`Summary`], so [`refresh`](Self::refresh) can keep the cursor
    /// on the entry it was on.
    undo_ids: Vec<EntryId>,
    redo_ids: Vec<EntryId>,
    cursor: usize,
}

impl HistoryPopup {
    /// A popup listing `summary`, or `None` when there is nothing to take back or do again.
    pub fn new(summary: Summary) -> Option<Self> {
        if summary.undo.is_empty() && summary.redo.is_empty() {
            return None;
        }
        Some(Self {
            undo: summary.undo,
            redo: summary.redo,
            undo_ids: summary.undo_ids,
            redo_ids: summary.redo_ids,
            cursor: 0,
        })
    }

    /// Lists `summary` instead, with the cursor still on the entry it was on: entries another
    /// session added can land above it, and an `Enter` must still act on the row that was shown.
    /// An empty summary is ignored, since a popup with no row has no cursor to keep.
    pub fn refresh(&mut self, summary: Summary) {
        if summary.undo.is_empty() && summary.redo.is_empty() {
            return;
        }
        let anchor = self.anchor();
        self.undo = summary.undo;
        self.redo = summary.redo;
        self.undo_ids = summary.undo_ids;
        self.redo_ids = summary.redo_ids;
        let found = anchor.and_then(|id| {
            let in_undo = self.undo_ids.iter().position(|other| *other == id);
            in_undo.or_else(|| {
                let at = self.redo_ids.iter().position(|other| *other == id)?;
                Some(self.undo.len() + at)
            })
        });
        // Entries are only ever added here, so a lost anchor means the one under the cursor was
        // taken off by this session; the nearest row is as good a place as any.
        self.cursor = found.unwrap_or(self.cursor).min(self.len() - 1);
    }

    /// The id of the entry under the cursor.
    fn anchor(&self) -> Option<EntryId> {
        match self.cursor.checked_sub(self.undo.len()) {
            None => self.undo_ids.get(self.cursor),
            Some(at) => self.redo_ids.get(at),
        }
        .copied()
    }

    pub fn len(&self) -> usize {
        self.undo.len() + self.redo.len()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn row(&self, index: usize) -> Option<Row<'_>> {
        if let Some(label) = self.undo.get(index) {
            return Some(Row {
                side: Side::Undo,
                depth: index + 1,
                label,
            });
        }
        let at = index.checked_sub(self.undo.len())?;
        self.redo.get(at).map(|label| Row {
            side: Side::Redo,
            depth: at + 1,
            label,
        })
    }

    /// The rows from `first` on, as many as `count`.
    pub fn rows(&self, first: usize, count: usize) -> impl Iterator<Item = Row<'_>> {
        (first..first.saturating_add(count)).map_while(|index| self.row(index))
    }

    /// Where the visible part of the list starts when `capacity` rows fit, keeping the cursor in
    /// view. Worked out from the cursor alone, so the popup holds no scroll state that could
    /// disagree with what is drawn or hit-tested.
    pub fn first_visible(&self, capacity: usize) -> usize {
        let capacity = capacity.max(1);
        let latest = self.len().saturating_sub(capacity);
        self.cursor.saturating_sub(capacity / 2).min(latest)
    }

    /// Moves the cursor to `index` (a click on that row).
    pub fn focus(&mut self, index: usize) {
        if index < self.len() {
            self.cursor = index;
        }
    }

    /// What choosing the row under the cursor asks for.
    pub fn walk(&self) -> Option<Walk> {
        self.row(self.cursor).map(|row| Walk {
            side: row.side,
            entries: row.depth,
        })
    }

    /// What `u` on the row under the cursor asks for.
    pub fn pick(&self) -> Option<Pick> {
        self.row(self.cursor).map(|row| Pick {
            side: row.side,
            depth: row.depth,
        })
    }

    /// Up, down, `j` and `k` move, wrapping; `Home` and `End` jump; `Enter` runs the row under the
    /// cursor and everything between; `u` runs only that row; `Esc` and `q` close.
    pub fn key(&mut self, code: KeyCode) -> Key {
        let last = self.len() - 1;
        match code {
            KeyCode::Esc | KeyCode::Char('q') => return Key::Close,
            KeyCode::Enter => return self.walk().map_or(Key::Stay, Key::Run),
            KeyCode::Char('u') => return self.pick().map_or(Key::Stay, Key::Only),
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = if self.cursor == last {
                    0
                } else {
                    self.cursor + 1
                };
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.checked_sub(1).unwrap_or(last);
            }
            KeyCode::Home | KeyCode::Char('g') => self.cursor = 0,
            KeyCode::End | KeyCode::Char('G') => self.cursor = last,
            _ => {}
        }
        Key::Stay
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn popup(undo: &[&str], redo: &[&str]) -> HistoryPopup {
        HistoryPopup::new(Summary {
            undo: undo.iter().map(|s| (*s).to_owned()).collect(),
            redo: redo.iter().map(|s| (*s).to_owned()).collect(),
            ..Summary::default()
        })
        .unwrap()
    }

    /// Entries named by their labels, each with an id of its own that `entry` gives again for the
    /// same label within one test.
    fn summary(ids: &mut Vec<(String, EntryId)>, undo: &[&str], redo: &[&str]) -> Summary {
        let mut id_of = |label: &str| {
            if let Some((_, id)) = ids.iter().find(|(name, _)| name == label) {
                return *id;
            }
            let id = EntryId::fresh();
            ids.push((label.to_owned(), id));
            id
        };
        Summary {
            undo: undo.iter().map(|s| (*s).to_owned()).collect(),
            redo: redo.iter().map(|s| (*s).to_owned()).collect(),
            undo_ids: undo.iter().map(|s| id_of(s)).collect(),
            redo_ids: redo.iter().map(|s| id_of(s)).collect(),
        }
    }

    #[test]
    fn an_entry_added_above_the_cursor_leaves_the_cursor_on_the_same_entry() {
        let mut ids = Vec::new();
        let mut popup = HistoryPopup::new(summary(&mut ids, &["b", "a"], &["c"])).unwrap();
        popup.key(KeyCode::Down);
        assert_eq!(popup.row(popup.cursor()).unwrap().label, "a");

        popup.refresh(summary(&mut ids, &["new", "b", "a"], &["c"]));

        assert_eq!(popup.row(popup.cursor()).unwrap().label, "a");
        assert_eq!(
            popup.pick(),
            Some(Pick {
                side: Side::Undo,
                depth: 3
            })
        );
    }

    #[test]
    fn the_cursor_on_a_redo_row_follows_it_when_the_undo_list_grows() {
        let mut ids = Vec::new();
        let mut popup = HistoryPopup::new(summary(&mut ids, &["a"], &["d", "c"])).unwrap();
        popup.key(KeyCode::End);
        assert_eq!(popup.row(popup.cursor()).unwrap().label, "c");

        popup.refresh(summary(&mut ids, &["b", "a"], &["d", "c"]));

        assert_eq!(popup.row(popup.cursor()).unwrap().label, "c");
    }

    #[test]
    fn refreshing_with_nothing_listed_changes_nothing() {
        let mut popup = popup(&["a"], &[]);

        popup.refresh(Summary::default());

        assert_eq!(popup.len(), 1);
    }

    #[test]
    fn there_is_no_popup_when_both_stacks_are_empty() {
        assert!(HistoryPopup::new(Summary::default()).is_none());
    }

    #[test]
    fn rows_list_the_undo_entries_then_the_redo_entries_each_counted_from_its_own_stack() {
        let popup = popup(&["create b", "create a"], &["create c"]);

        let rows: Vec<_> = popup
            .rows(0, 10)
            .map(|r| (r.side, r.depth, r.label))
            .collect();

        assert_eq!(
            rows,
            [
                (Side::Undo, 1, "create b"),
                (Side::Undo, 2, "create a"),
                (Side::Redo, 1, "create c"),
            ]
        );
    }

    #[test]
    fn enter_asks_for_every_entry_down_to_the_row() {
        let mut popup = popup(&["c", "b", "a"], &["d"]);
        popup.key(KeyCode::Down);
        popup.key(KeyCode::Down);
        assert_eq!(
            popup.key(KeyCode::Enter),
            Key::Run(Walk {
                side: Side::Undo,
                entries: 3
            })
        );

        popup.key(KeyCode::Down);
        assert_eq!(
            popup.key(KeyCode::Enter),
            Key::Run(Walk {
                side: Side::Redo,
                entries: 1
            })
        );
    }

    #[test]
    fn u_asks_for_only_the_row_under_the_cursor() {
        let mut popup = popup(&["c", "b", "a"], &["d"]);
        popup.key(KeyCode::Down);
        assert_eq!(
            popup.key(KeyCode::Char('u')),
            Key::Only(Pick {
                side: Side::Undo,
                depth: 2
            })
        );

        popup.key(KeyCode::End);
        assert_eq!(
            popup.key(KeyCode::Char('u')),
            Key::Only(Pick {
                side: Side::Redo,
                depth: 1
            })
        );
    }

    #[test]
    fn the_cursor_wraps_and_jumps() {
        let mut popup = popup(&["b", "a"], &["c"]);
        popup.key(KeyCode::Up);
        assert_eq!(popup.cursor(), 2);
        popup.key(KeyCode::Down);
        assert_eq!(popup.cursor(), 0);
        popup.key(KeyCode::End);
        assert_eq!(popup.cursor(), 2);
        popup.key(KeyCode::Home);
        assert_eq!(popup.cursor(), 0);
    }

    #[test]
    fn escape_and_q_close_and_other_keys_do_nothing() {
        let mut popup = popup(&["a"], &[]);
        assert_eq!(popup.key(KeyCode::Esc), Key::Close);
        assert_eq!(popup.key(KeyCode::Char('q')), Key::Close);
        assert_eq!(popup.key(KeyCode::Char('x')), Key::Stay);
    }

    proptest! {
        #[test]
        fn the_cursor_is_always_inside_the_visible_window(
            undo in 0usize..60,
            redo in 0usize..60,
            capacity in 1usize..20,
            moves in proptest::collection::vec(0u8..4, 0..80),
        ) {
            prop_assume!(undo + redo > 0);
            let labels = |n: usize| (0..n).map(|i| i.to_string()).collect();
            let mut popup = HistoryPopup::new(Summary {
                undo: labels(undo),
                redo: labels(redo),
                ..Summary::default()
            })
            .unwrap();
            for step in moves {
                popup.key([KeyCode::Down, KeyCode::Up, KeyCode::Home, KeyCode::End][usize::from(step)]);
                let first = popup.first_visible(capacity);
                prop_assert!(first <= popup.cursor());
                prop_assert!(popup.cursor() < first + capacity);
                prop_assert!(popup.rows(first, capacity).count() == capacity.min(popup.len()));
            }
        }
    }
}
