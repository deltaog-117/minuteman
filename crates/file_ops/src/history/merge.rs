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

//! Saving one session's history into a file other sessions also save to, without losing theirs.
//!
//! Model: a session holds a set of entries; the file holds another. Saving writes
//! `ours ∪ (theirs ∖ removed)`, where `removed` is every entry this session once held and no
//! longer does (undone entries move to the other stack and stay held; only a forgotten one, by
//! capacity or by a new change clearing the redo stack, leaves). Invariants:
//!
//! - an entry another session wrote is kept until this one has removed it, so no save erases
//!   what it never saw;
//! - an entry both hold is written as this session has it, side and place included, so the last
//!   session to save decides which stack a shared entry is on;
//! - each stack of the file holds at most `capacity` entries, the oldest going first.
//!
//! The merge works on the text, not on the changes in it, so an entry that cannot be brought back
//! here (its trash item is gone) is still carried for the session that can.
//!
//! What is not decided: an entry another session removed comes back if this one still holds it,
//! and the merged file reaches this session's memory only at its next launch.

use std::collections::HashSet;

use super::EntryId;
use super::store::{Block, DecodeError, VERSION, parse_blocks};

/// What one session remembers between its saves.
#[derive(Debug)]
pub struct Merger {
    capacity: usize,
    /// Held at the last save.
    held: HashSet<EntryId>,
    /// Held once and not any more (see the module docs).
    removed: HashSet<EntryId>,
}

impl Merger {
    /// `forgotten` is what loading left out (`Restored::forgotten`), so it is not written back.
    pub fn new(capacity: usize, forgotten: impl IntoIterator<Item = EntryId>) -> Self {
        Self {
            capacity: capacity.max(1),
            held: HashSet::new(),
            removed: forgotten.into_iter().collect(),
        }
    }

    /// The text to write for `ours` (what `History::encode` gave) over `theirs`, the file as it
    /// stands now, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// `theirs` was saved in a format this version does not read, or is not a history: the
    /// caller leaves the file alone. Nothing is remembered from `ours` then, so the next call
    /// compares against the same last save.
    pub fn merge(&mut self, ours: &str, theirs: Option<&str>) -> Result<String, DecodeError> {
        let ours = parse_blocks(ours)?;
        let theirs = theirs.map(parse_blocks).transpose()?;

        let held: HashSet<EntryId> = ours
            .undo
            .iter()
            .chain(&ours.redo)
            .map(|block| block.id)
            .collect();
        self.removed.extend(self.held.difference(&held));
        self.held = held;

        let (undo, redo) = match theirs {
            None => (ours.undo, ours.redo),
            Some(theirs) => {
                // A foreign redo entry goes under this session's own: the next redo stays one
                // that this session made.
                let mut redo = self.unseen(theirs.redo);
                redo.extend(ours.redo);
                (interleave(ours.undo, self.unseen(theirs.undo)), redo)
            }
        };
        Ok(render(
            keep_newest(undo, self.capacity),
            keep_newest(redo, self.capacity),
        ))
    }
}

impl Merger {
    /// The blocks of the file that this session has not seen or removed.
    fn unseen<'a>(&self, blocks: Vec<Block<'a>>) -> Vec<Block<'a>> {
        blocks
            .into_iter()
            .filter(|b| !self.held.contains(&b.id) && !self.removed.contains(&b.id))
            .collect()
    }
}

/// `ours` in its order with each of `foreign` put before the first of `ours` that is newer, or
/// at the end. `ours` is not sorted (a redone entry is old and last), so only `foreign` is.
fn interleave<'a>(ours: Vec<Block<'a>>, mut foreign: Vec<Block<'a>>) -> Vec<Block<'a>> {
    foreign.sort_by_key(|block| block.id);
    let mut foreign = foreign.into_iter().peekable();
    let mut merged = Vec::with_capacity(ours.len() + foreign.len());
    for block in ours {
        while let Some(older) = foreign.next_if(|f| f.id < block.id) {
            merged.push(older);
        }
        merged.push(block);
    }
    merged.extend(foreign);
    merged
}

fn keep_newest(mut blocks: Vec<Block<'_>>, capacity: usize) -> Vec<Block<'_>> {
    let excess = blocks.len().saturating_sub(capacity);
    blocks.drain(..excess);
    blocks
}

fn render(undo: Vec<Block<'_>>, redo: Vec<Block<'_>>) -> String {
    let mut out = format!("mman-history {VERSION}\n");
    for (name, blocks) in [("undo", undo), ("redo", redo)] {
        out.push_str(&format!("@ {name}\n"));
        for block in blocks {
            out.push_str(&format!("# {}\n", block.id));
            for line in block.lines {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str("-\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::history::{Change, History};

    fn history_of(paths: &[&str]) -> History {
        let mut history = History::default();
        for path in paths {
            history.record(Change::Created {
                path: (*path).into(),
                is_dir: false,
            });
            history.commit();
        }
        history
    }

    fn paths_in(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| line.strip_prefix("C\t"))
            .map(|rest| rest.split('\t').next().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn two_sessions_saving_in_turn_keep_both_sets_of_entries() {
        let (mut a, mut b) = (Merger::new(100, []), Merger::new(100, []));
        let (first, second) = (history_of(&["/h/a1", "/h/a2"]), history_of(&["/h/b1"]));

        let file = a.merge(&first.encode(), None).unwrap();
        let file = b.merge(&second.encode(), Some(&file)).unwrap();
        let file = a.merge(&first.encode(), Some(&file)).unwrap();

        let mut found = paths_in(&file);
        found.sort();
        assert_eq!(found, ["/h/a1", "/h/a2", "/h/b1"]);
    }

    #[test]
    fn what_this_session_removed_is_not_written_back() {
        let mut merger = Merger::new(100, []);
        let mut history = history_of(&["/h/x"]);
        let file = merger.merge(&history.encode(), None).unwrap();
        history = History::default();

        let file = merger.merge(&history.encode(), Some(&file)).unwrap();

        assert!(paths_in(&file).is_empty());
    }

    #[test]
    fn what_loading_forgot_is_not_written_back() {
        let history = history_of(&["/h/x"]);
        let file = history.encode();
        let id = parse_blocks(&file).unwrap().undo[0].id;
        let mut merger = Merger::new(100, [id]);

        let merged = merger
            .merge(&History::default().encode(), Some(&file))
            .unwrap();

        assert!(paths_in(&merged).is_empty());
    }

    #[test]
    fn a_file_in_a_newer_format_is_refused_and_not_remembered() {
        let mut merger = Merger::new(100, []);
        let history = history_of(&["/h/x"]);

        let error = merger
            .merge(&history.encode(), Some("mman-history 9\n"))
            .unwrap_err();

        assert_eq!(error, DecodeError::Version("9".into()));
        assert!(merger.held.is_empty());
    }

    #[test]
    fn foreign_entries_are_put_among_ours_by_time() {
        let early = history_of(&["/h/early"]);
        let late = history_of(&["/h/late"]);
        let theirs = early.encode();
        let mut merger = Merger::new(100, []);

        let merged = merger.merge(&late.encode(), Some(&theirs)).unwrap();

        assert_eq!(paths_in(&merged), ["/h/early", "/h/late"]);
    }

    #[test]
    fn the_oldest_entries_go_first_past_the_capacity() {
        let theirs = history_of(&["/h/1", "/h/2"]).encode();
        let ours = history_of(&["/h/3"]).encode();
        let mut merger = Merger::new(2, []);

        let merged = merger.merge(&ours, Some(&theirs)).unwrap();

        assert_eq!(paths_in(&merged), ["/h/2", "/h/3"]);
    }

    #[test]
    fn a_file_from_before_entries_had_names_is_carried_whole() {
        let old = "mman-history 1\n@ undo\nC\t/h/a\tf\n-\nC\t/h/b\tf\n-\n@ redo\n";
        let mut merger = Merger::new(100, []);

        let merged = merger
            .merge(&history_of(&["/h/c"]).encode(), Some(old))
            .unwrap();

        assert_eq!(paths_in(&merged), ["/h/a", "/h/b", "/h/c"]);
        assert!(merged.starts_with("mman-history 2\n"));
    }

    proptest! {
        #[test]
        fn a_merge_reads_back_and_never_exceeds_the_capacity(
            mine in prop::collection::vec("[a-z]{1,6}", 0..8),
            theirs in prop::collection::vec("[a-z]{1,6}", 0..8),
            capacity in 1usize..6,
        ) {
            let as_paths = |names: &[String]| -> Vec<String> {
                names.iter().map(|n| format!("/h/{n}")).collect()
            };
            let (mine, theirs) = (as_paths(&mine), as_paths(&theirs));
            let ours = history_of(&mine.iter().map(String::as_str).collect::<Vec<_>>()).encode();
            let file = history_of(&theirs.iter().map(String::as_str).collect::<Vec<_>>()).encode();
            let mut merger = Merger::new(capacity, []);

            let merged = merger.merge(&ours, Some(&file)).unwrap();

            let blocks = parse_blocks(&merged).unwrap();
            prop_assert_eq!(blocks.stray, 0);
            prop_assert!(blocks.undo.len() <= capacity);
            // Merging the result over itself changes nothing.
            let again = Merger::new(capacity, []).merge(&merged, Some(&merged)).unwrap();
            prop_assert_eq!(again, merged);
        }
    }
}
