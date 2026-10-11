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

//! Checking a growing index against the current query, a slice at a time. The walk keeps adding
//! entries while the user keeps typing, so the scan remembers how far it has looked and carries on
//! from there; a new query starts it over. Entries arrive nearest level first and are kept in
//! that order, so the first `SHOWN` matches are also the nearest ones and nothing needs sorting.

use plugin_api::TreeEntry;

use crate::query::Search;

/// How many matches are kept to be listed; the rest are only counted, since the panel shows a
/// screenful and the host asks for no more.
pub const SHOWN: usize = 200;

#[derive(Debug, Default)]
pub struct Scan {
    search: Search,
    /// The time ages are counted back from, taken when the query was set so one query is judged
    /// by one clock reading.
    now: u64,
    /// How many entries of the index have been checked against `search`.
    scanned: usize,
    /// Positions in the index of the first `SHOWN` matches, in index order.
    hits: Vec<usize>,
    matches: usize,
}

impl Scan {
    /// Starts over for `search`, judging ages as of `now`.
    pub fn set_query(&mut self, search: Search, now: u64) {
        *self = Self {
            search,
            now,
            ..Self::default()
        };
    }

    /// Checks at most `budget` more entries of `index` and says whether every one is checked.
    pub fn advance(&mut self, index: &[TreeEntry], budget: usize) -> bool {
        let end = index.len().min(self.scanned.saturating_add(budget));
        for (position, entry) in index.iter().enumerate().take(end).skip(self.scanned) {
            if self.search.matches(entry, self.now) {
                self.matches += 1;
                if self.hits.len() < SHOWN {
                    self.hits.push(position);
                }
            }
        }
        self.scanned = end;
        self.scanned == index.len()
    }

    /// The positions of the matches to list, nearest first.
    pub fn hits(&self) -> &[usize] {
        &self.hits
    }

    /// How many entries matched among those checked so far.
    pub fn matches(&self) -> usize {
        self.matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::path::PathBuf;

    fn entry(rel: &str) -> TreeEntry {
        TreeEntry {
            path: PathBuf::from("/root").join(rel),
            rel: rel.to_string(),
            is_dir: false,
            size: None,
            modified: None,
        }
    }

    fn scan_for(text: &str) -> Scan {
        let mut scan = Scan::default();
        scan.set_query(Search::parse(text).search, 0);
        scan
    }

    #[test]
    fn matches_come_back_in_index_order() {
        let index = [entry("a.rs"), entry("b.md"), entry("c.rs"), entry("d/e.rs")];
        let mut scan = scan_for("*.rs");
        assert!(scan.advance(&index, usize::MAX));
        assert_eq!(scan.hits(), [0, 2, 3]);
        assert_eq!(scan.matches(), 3);
    }

    #[test]
    fn a_slice_at_a_time_gives_the_same_answer_as_all_at_once() {
        let index: Vec<TreeEntry> = (0..50).map(|i| entry(&format!("f{i}.rs"))).collect();
        let mut sliced = scan_for("f1");
        while !sliced.advance(&index, 7) {}
        let mut whole = scan_for("f1");
        whole.advance(&index, usize::MAX);
        assert_eq!(sliced.hits(), whole.hits());
        assert_eq!(sliced.matches(), whole.matches());
    }

    #[test]
    fn entries_that_arrive_later_are_checked_from_where_it_stopped() {
        let mut index = vec![entry("a.rs"), entry("b.md")];
        let mut scan = scan_for("*.rs");
        assert!(scan.advance(&index, usize::MAX));
        index.push(entry("c.rs"));
        assert!(!scan.advance(&index, 0), "behind by one entry");
        assert!(scan.advance(&index, usize::MAX));
        assert_eq!(scan.hits(), [0, 2]);
    }

    #[test]
    fn a_new_query_starts_over() {
        let index = [entry("a.rs"), entry("b.md")];
        let mut scan = scan_for("*.rs");
        scan.advance(&index, usize::MAX);
        scan.set_query(Search::parse("*.md").search, 0);
        assert!(scan.hits().is_empty());
        assert_eq!(scan.matches(), 0);
        scan.advance(&index, usize::MAX);
        assert_eq!(scan.hits(), [1]);
    }

    #[test]
    fn only_the_first_shown_are_kept_but_all_are_counted() {
        let index: Vec<TreeEntry> = (0..SHOWN + 50).map(|i| entry(&format!("f{i}"))).collect();
        let mut scan = scan_for("f");
        scan.advance(&index, usize::MAX);
        assert_eq!(scan.hits().len(), SHOWN);
        assert_eq!(scan.hits()[SHOWN - 1], SHOWN - 1);
        assert_eq!(scan.matches(), SHOWN + 50);
    }

    proptest! {
        /// However the work is cut into slices, the answer is the same.
        #[test]
        fn slicing_never_changes_the_answer(
            names in prop::collection::vec("[a-c]{1,4}", 0..60),
            slice in 1usize..20,
        ) {
            let index: Vec<TreeEntry> = names.iter().map(|n| entry(n)).collect();
            let mut sliced = scan_for("a");
            while !sliced.advance(&index, slice) {}
            let mut whole = scan_for("a");
            whole.advance(&index, usize::MAX);
            prop_assert_eq!(sliced.hits(), whole.hits());
            prop_assert_eq!(sliced.matches(), whole.matches());
        }
    }
}
