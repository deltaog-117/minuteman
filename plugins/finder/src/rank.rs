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

//! How the fuzzy finder ranks what it has indexed: a running ranking of a growing index against
//! what is typed, built to be worked on in small slices. It matches the path as well as the name:
//! `src/main` finds `src/main.rs`, and a name that holds the typed letters beats a path that merely
//! does.

use plugin_api::TreeEntry;
use shared::Query;

/// How many ranked results are kept for display; the rest are matches the user can narrow to by
/// typing more.
pub const SHOWN: usize = 200;

/// Added to a match on the entry's own name, so that `main` ranks `src/main.rs` above
/// `domain/a/i/n.rs`. Larger than any difference the path's own scoring makes except a whole-piece
/// match (`Query::score` adds 1,000 for one), which a name match then still outranks on a tie.
const NAME_BONUS: i32 = 500;

/// How well `rel` fits `query`, or `None` if it does not hold the letters in order. A fit on the
/// entry's own name scores above one that only works across the whole path.
pub fn score(query: &Query, rel: &str) -> Option<i32> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let on_name = query.score(name).map(|s| s + NAME_BONUS);
    if name.len() == rel.len() {
        return on_name;
    }
    on_name.max(query.score(rel))
}

/// What is known about one typed query: every entry of the index below `scanned` that matches it,
/// scored.
#[derive(Debug)]
struct Level {
    query: Query,
    /// `(score, position in the index)`, in the order the entries were scored.
    scored: Vec<(i32, usize)>,
    /// Entries of the index below this are accounted for, once `from_parent` is `None`.
    scanned: usize,
    /// While this level is being narrowed from the one beneath it, how many of that level's
    /// matches have been looked at; `None` when that is done (or was never needed).
    from_parent: Option<usize>,
}

/// A running ranking of a growing index against what is typed, built to be worked on in small
/// slices. Typing another letter does not start over: everything that matches `abc` also matches
/// `ab`, so the new query only looks at the previous query's matches, which shrink quickly. Each
/// typed prefix keeps its own level, so deleting a letter returns to the earlier one for free.
/// Entries that arrive while the walk is still running are scored once, when `advance` reaches
/// them. Nothing is skipped or approximated: the answer is the one scoring every entry against
/// the final query would give.
#[derive(Debug, Default)]
pub struct Ranking {
    /// One level per prefix of the current query that has been worked on, shortest first. Empty
    /// for an empty query, which lists the index as the walk met it.
    levels: Vec<Level>,
}

impl Ranking {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ranks against `query` from here on; `None` (nothing typed) lists the index as met. Levels
    /// that are not a prefix of `query` are dropped; the closest one that is, and is finished
    /// narrowing, is what the new level narrows from.
    pub fn set_query(&mut self, query: Option<Query>) {
        let Some(query) = query else {
            self.levels.clear();
            return;
        };
        loop {
            match self.levels.last() {
                Some(last) if last.query == query => return,
                Some(last) if last.from_parent.is_none() && query.extends(&last.query) => {
                    let scanned = last.scanned;
                    self.levels.push(Level {
                        query,
                        scored: Vec::new(),
                        scanned,
                        from_parent: Some(0),
                    });
                    return;
                }
                Some(_) => {
                    self.levels.pop();
                }
                None => {
                    self.levels.push(Level {
                        query,
                        scored: Vec::new(),
                        scanned: 0,
                        from_parent: None,
                    });
                    return;
                }
            }
        }
    }

    /// Scores at most `budget` entries toward the current query, from `index` as it is now, and
    /// says whether it has caught up with all of it. Call it again with a larger `index` as the
    /// walk delivers more.
    pub fn advance(&mut self, index: &[TreeEntry], budget: usize) -> bool {
        let Some(at) = self.levels.len().checked_sub(1) else {
            return true;
        };
        let (parents, last) = self.levels.split_at_mut(at);
        let level = &mut last[0];
        let mut left = budget;
        if let Some(cursor) = level.from_parent {
            match parents.last() {
                Some(parent) => {
                    let take = left.min(parent.scored.len() - cursor);
                    for &(_, position) in &parent.scored[cursor..cursor + take] {
                        if let Some(s) = score(&level.query, &index[position].rel) {
                            level.scored.push((s, position));
                        }
                    }
                    left -= take;
                    let done = cursor + take;
                    level.from_parent = (done < parent.scored.len()).then_some(done);
                    if level.from_parent.is_some() {
                        return false;
                    }
                }
                None => level.from_parent = None,
            }
        }
        let take = left.min(index.len().saturating_sub(level.scanned));
        for (position, candidate) in index.iter().enumerate().skip(level.scanned).take(take) {
            if let Some(s) = score(&level.query, &candidate.rel) {
                level.scored.push((s, position));
            }
        }
        level.scanned += take;
        level.scanned >= index.len()
    }

    /// How many entries match so far: all of an index of `index_len` for an empty query.
    pub fn matches(&self, index_len: usize) -> usize {
        match self.levels.last() {
            Some(level) => level.scored.len(),
            None => index_len,
        }
    }

    /// The positions in the index of the best `limit` matches, best first. Equal scores keep the
    /// order the walk met them in, so a nearer entry wins a tie.
    pub fn top(&self, index_len: usize, limit: usize) -> Vec<usize> {
        let Some(level) = self.levels.last() else {
            return (0..index_len.min(limit)).collect();
        };
        let by_best = |a: &(i32, usize), b: &(i32, usize)| b.0.cmp(&a.0).then(a.1.cmp(&b.1));
        let mut kept = level.scored.clone();
        if kept.len() > limit {
            kept.select_nth_unstable_by(limit, by_best);
            kept.truncate(limit);
        }
        kept.sort_by(by_best);
        kept.into_iter().map(|(_, position)| position).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::path::PathBuf;

    fn candidate(rel: &str) -> TreeEntry {
        TreeEntry {
            path: PathBuf::from("/root").join(rel),
            rel: rel.to_string(),
            is_dir: false,
        }
    }

    fn ranked(query: &str, rels: &[&str]) -> Vec<String> {
        let index: Vec<TreeEntry> = rels.iter().map(|rel| candidate(rel)).collect();
        let mut ranking = Ranking::new();
        ranking.set_query(Query::new(query));
        assert!(ranking.advance(&index, usize::MAX));
        ranking
            .top(index.len(), SHOWN)
            .into_iter()
            .map(|position| index[position].rel.clone())
            .collect()
    }

    #[test]
    fn a_match_on_the_name_beats_one_scattered_over_the_path() {
        assert_eq!(
            ranked("main", &["domain/a/i/n.rs", "src/main.rs"]),
            ["src/main.rs", "domain/a/i/n.rs"]
        );
    }

    #[test]
    fn a_typed_path_matches_across_folders() {
        assert_eq!(
            ranked("src/main", &["main.rs", "src/main.rs", "lib/main.rs"]),
            ["src/main.rs"]
        );
    }

    #[test]
    fn letters_in_order_are_enough() {
        assert_eq!(ranked("rdme", &["README.md", "main.rs"]), ["README.md"]);
    }

    #[test]
    fn nothing_typed_lists_the_index_as_the_walk_met_it() {
        let index = [candidate("b"), candidate("a"), candidate("c")];
        let mut ranking = Ranking::new();
        ranking.set_query(None);
        assert!(ranking.advance(&index, 1), "nothing to rank");
        assert_eq!(ranking.top(index.len(), 2), [0, 1]);
        assert_eq!(ranking.matches(index.len()), 3);
    }

    #[test]
    fn equal_scores_keep_the_order_the_walk_met_them_in() {
        assert_eq!(ranked("a", &["x/a", "y/a", "z/a"]), ["x/a", "y/a", "z/a"]);
    }

    #[test]
    fn work_is_done_in_slices_and_arrivals_are_scored_when_reached() {
        let rels = ["src/main.rs", "docs/guide.md", "main.c", "src/lib/main.rs"];
        let index: Vec<TreeEntry> = rels.iter().map(|rel| candidate(rel)).collect();
        let mut stepped = Ranking::new();
        stepped.set_query(Query::new("main"));
        assert!(!stepped.advance(&index, 1), "one entry of four is not all");
        assert!(
            stepped.advance(&index[..2], 5),
            "caught up with a shorter index"
        );
        assert!(stepped.advance(&index, 5));
        let mut whole = Ranking::new();
        whole.set_query(Query::new("main"));
        whole.advance(&index, usize::MAX);
        assert_eq!(stepped.top(4, SHOWN), whole.top(4, SHOWN));
        assert_eq!(stepped.matches(4), 3);
    }

    #[test]
    fn typing_more_narrows_the_matches_and_deleting_returns_to_them() {
        let index = [
            candidate("main.rs"),
            candidate("manual.md"),
            candidate("mod.rs"),
        ];
        let mut ranking = Ranking::new();
        ranking.set_query(Query::new("m"));
        ranking.advance(&index, usize::MAX);
        assert_eq!(ranking.matches(3), 3);
        ranking.set_query(Query::new("ma"));
        // Only the three matches of `m` are looked at, not the index.
        assert!(!ranking.advance(&index, 2));
        assert!(ranking.advance(&index, 2));
        assert_eq!(ranking.matches(3), 2);
        ranking.set_query(Query::new("m"));
        assert!(
            ranking.advance(&index, 0),
            "the earlier level is still complete"
        );
        assert_eq!(ranking.matches(3), 3);
    }

    #[test]
    fn a_new_query_that_is_not_an_extension_starts_over() {
        let index = [candidate("alpha"), candidate("beta")];
        let mut ranking = Ranking::new();
        ranking.set_query(Query::new("alpha"));
        ranking.advance(&index, usize::MAX);
        assert_eq!(ranking.top(2, SHOWN), [0]);
        ranking.set_query(Query::new("beta"));
        ranking.advance(&index, usize::MAX);
        assert_eq!(ranking.top(2, SHOWN), [1]);
    }

    #[test]
    fn a_query_typed_before_the_last_one_finished_still_ranks_every_entry() {
        let index: Vec<TreeEntry> = (0..50).map(|i| candidate(&format!("ab{i}c"))).collect();
        let mut ranking = Ranking::new();
        ranking.set_query(Query::new("a"));
        assert!(!ranking.advance(&index, 10), "`a` is not finished");
        ranking.set_query(Query::new("ab"));
        assert!(ranking.advance(&index, usize::MAX));
        assert_eq!(ranking.matches(50), 50);
    }

    proptest! {
        /// Whatever is typed, every result matches it, no more than `limit` come back, and they
        /// are ordered best first.
        #[test]
        fn results_always_match_are_bounded_and_ordered(
            query in "[a-c/.]{1,4}",
            rels in proptest::collection::vec("[a-c/.]{1,10}", 0..40),
            limit in 1usize..20,
        ) {
            let index: Vec<TreeEntry> = rels.iter().map(|rel| candidate(rel)).collect();
            let typed = Query::new(&query).unwrap();
            let mut ranking = Ranking::new();
            ranking.set_query(Some(typed.clone()));
            ranking.advance(&index, usize::MAX);
            let top = ranking.top(index.len(), limit);
            prop_assert!(top.len() <= limit);
            let scores: Vec<i32> = top
                .iter()
                .map(|&p| score(&typed, &index[p].rel).expect("a result must match"))
                .collect();
            prop_assert!(scores.windows(2).all(|pair| pair[0] >= pair[1]));
            let expected = index.iter().filter(|c| score(&typed, &c.rel).is_some()).count();
            prop_assert_eq!(ranking.matches(index.len()), expected);
            prop_assert_eq!(top.len(), expected.min(limit));
        }

        /// Typing and deleting letters in any order, with the index growing and the work cut
        /// into small slices in between, ends with the ranking scoring every entry against the
        /// final query would give: narrowing and slicing lose nothing.
        #[test]
        fn narrowing_and_slicing_rank_exactly_as_ranking_from_scratch(
            rels in proptest::collection::vec("[a-c/.]{1,8}", 0..30),
            steps in proptest::collection::vec((any::<bool>(), 0usize..3, 0usize..6), 1..14),
        ) {
            let index: Vec<TreeEntry> = rels.iter().map(|rel| candidate(rel)).collect();
            let mut ranking = Ranking::new();
            let mut text = String::new();
            for (i, (push, letter, budget)) in steps.iter().enumerate() {
                if *push {
                    text.push(['a', 'b', 'c'][*letter]);
                } else {
                    text.pop();
                }
                ranking.set_query(Query::new(&text));
                let visible = index.len() * (i + 1) / steps.len();
                ranking.advance(&index[..visible], *budget);
            }
            ranking.advance(&index, usize::MAX);
            let mut scratch = Ranking::new();
            scratch.set_query(Query::new(&text));
            scratch.advance(&index, usize::MAX);
            prop_assert_eq!(ranking.matches(index.len()), scratch.matches(index.len()));
            prop_assert_eq!(
                ranking.top(index.len(), SHOWN),
                scratch.top(index.len(), SHOWN)
            );
        }
    }
}
