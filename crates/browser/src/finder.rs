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

//! What the fuzzy finder (`f`) searches and how it ranks it: a flat index of every entry beneath a
//! root, gathered nearest level first, and a ranking of that index against what is typed. Unlike
//! `search`, which jumps to the nearest match, this keeps the whole index so a keystroke only
//! re-scores it, and it matches the path as well as the name: `src/main` finds `src/main.rs`, and
//! a name that holds the typed letters beats a path that merely does.
//!
//! The index is built through `Vfs::list_dir` alone, one directory per call, handing each
//! directory's entries to a callback as it goes. That is what lets the caller show results while
//! the walk is still running, cancel it between directories, and run it over a remote backend.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use shared::Vfs;

use crate::is_hidden;
use crate::search::{Limits, Query};

/// How many ranked results are kept for display; the rest are matches the user can narrow to by
/// typing more.
pub const SHOWN: usize = 200;

/// Added to a match on the entry's own name, so that `main` ranks `src/main.rs` above
/// `domain/a/i/n.rs`. Larger than any difference the path's own scoring makes except a whole-piece
/// match (`Query::score` adds 1,000 for one), which a name match then still outranks on a tie.
const NAME_BONUS: i32 = 500;

/// One entry of the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    /// The path below the root the walk began at, with `/` separators: what is matched and shown.
    pub rel: String,
    pub is_dir: bool,
}

impl Candidate {
    /// The entry's own name: the last piece of `rel`.
    pub fn name(&self) -> &str {
        self.rel.rsplit('/').next().unwrap_or(&self.rel)
    }

    /// The folders above the entry, relative to the root, or an empty string at the top level.
    pub fn parent_rel(&self) -> &str {
        self.rel.rfind('/').map_or("", |slash| &self.rel[..slash])
    }
}

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

/// A running ranking of a growing index against one query. Scoring is incremental: `extend` only
/// scores what arrived since, and `set_query` starts over, so a walk that is still delivering
/// directories costs one pass over each entry in total, not one per batch.
#[derive(Debug, Default)]
pub struct Ranking {
    query: Option<Query>,
    /// `(score, position in the index)` of every entry that matched. Unused for an empty query,
    /// which keeps everything in the order the walk met it.
    scored: Vec<(i32, usize)>,
    /// How much of the index has been looked at.
    scanned: usize,
}

impl Ranking {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ranks against `query` from here on; `None` (nothing typed) lists the index as met. The
    /// caller then `extend`s with the whole index again.
    pub fn set_query(&mut self, query: Option<Query>) {
        self.query = query;
        self.scored.clear();
        self.scanned = 0;
    }

    /// Takes in the part of `index` not yet looked at.
    pub fn extend(&mut self, index: &[Candidate]) {
        if let Some(query) = &self.query {
            for (position, candidate) in index.iter().enumerate().skip(self.scanned) {
                if let Some(s) = score(query, &candidate.rel) {
                    self.scored.push((s, position));
                }
            }
        }
        self.scanned = index.len();
    }

    /// How many entries match: all of the index for an empty query.
    pub fn matches(&self) -> usize {
        match self.query {
            Some(_) => self.scored.len(),
            None => self.scanned,
        }
    }

    /// The positions in the index of the best `limit` matches, best first. Equal scores keep the
    /// order the walk met them in, so a nearer entry wins a tie.
    pub fn top(&self, limit: usize) -> Vec<usize> {
        if self.query.is_none() {
            return (0..self.scanned.min(limit)).collect();
        }
        let by_best = |a: &(i32, usize), b: &(i32, usize)| b.0.cmp(&a.0).then(a.1.cmp(&b.1));
        let mut kept = self.scored.clone();
        if kept.len() > limit {
            kept.select_nth_unstable_by(limit, by_best);
            kept.truncate(limit);
        }
        kept.sort_by(by_best);
        kept.into_iter().map(|(_, position)| position).collect()
    }
}

/// How a walk ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Walked {
    /// Every entry within the limits was handed over. `truncated` says a limit cut the walk
    /// short, so entries may exist beyond what was indexed.
    Done { truncated: bool },
    /// `cancel` was set before the walk finished.
    Cancelled,
}

/// Walks `root` nearest level first and hands each directory's entries to `sink` as soon as that
/// directory is listed. A directory that can't be listed is skipped, as `find` would. Hidden
/// entries are neither indexed nor entered unless `show_hidden`, so the finder cannot offer
/// something the user has chosen not to see.
pub fn walk(
    vfs: &dyn Vfs,
    root: &Path,
    show_hidden: bool,
    limits: Limits,
    cancel: &AtomicBool,
    mut sink: impl FnMut(Vec<Candidate>),
) -> Walked {
    let mut queue = VecDeque::from([(root.to_path_buf(), 1)]);
    let mut listed = 0usize;
    let mut truncated = false;

    while let Some((dir, depth)) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            return Walked::Cancelled;
        }
        let Ok(entries) = vfs.list_dir(&dir) else {
            continue;
        };
        let mut batch = Vec::with_capacity(entries.len());
        for entry in entries {
            if !show_hidden && is_hidden(&entry.name) {
                continue;
            }
            listed += 1;
            if listed > limits.max_entries {
                if !batch.is_empty() {
                    sink(batch);
                }
                return Walked::Done { truncated: true };
            }
            let rel = entry
                .path
                .strip_prefix(root)
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .into_owned();
            if entry.is_dir {
                match depth < limits.max_depth {
                    true => queue.push_back((entry.path.clone(), depth + 1)),
                    false => truncated = true,
                }
            }
            batch.push(Candidate {
                path: entry.path,
                rel,
                is_dir: entry.is_dir,
            });
        }
        if !batch.is_empty() {
            sink(batch);
        }
    }
    Walked::Done { truncated }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use shared::MemVfs;

    fn candidate(rel: &str) -> Candidate {
        Candidate {
            path: PathBuf::from("/root").join(rel),
            rel: rel.to_string(),
            is_dir: false,
        }
    }

    fn ranked(query: &str, rels: &[&str]) -> Vec<String> {
        let index: Vec<Candidate> = rels.iter().map(|rel| candidate(rel)).collect();
        let mut ranking = Ranking::new();
        ranking.set_query(Query::new(query));
        ranking.extend(&index);
        ranking
            .top(SHOWN)
            .into_iter()
            .map(|position| index[position].rel.clone())
            .collect()
    }

    fn walked(vfs: &MemVfs, limits: Limits) -> (Vec<String>, Walked) {
        let mut rels = Vec::new();
        let outcome = walk(
            vfs,
            Path::new("/root"),
            false,
            limits,
            &AtomicBool::new(false),
            |batch| rels.extend(batch.into_iter().map(|c| c.rel)),
        );
        (rels, outcome)
    }

    #[test]
    fn a_candidate_knows_its_name_and_its_folder() {
        let c = candidate("src/ui/main.rs");
        assert_eq!(c.name(), "main.rs");
        assert_eq!(c.parent_rel(), "src/ui");
        let top = candidate("main.rs");
        assert_eq!(top.name(), "main.rs");
        assert_eq!(top.parent_rel(), "");
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
        ranking.extend(&index);
        assert_eq!(ranking.top(2), [0, 1]);
        assert_eq!(ranking.matches(), 3);
    }

    #[test]
    fn equal_scores_keep_the_order_the_walk_met_them_in() {
        assert_eq!(ranked("a", &["x/a", "y/a", "z/a"]), ["x/a", "y/a", "z/a"]);
    }

    #[test]
    fn extending_scores_only_what_arrived_and_agrees_with_scoring_it_all_at_once() {
        let rels = ["src/main.rs", "docs/guide.md", "main.c", "src/lib/main.rs"];
        let index: Vec<Candidate> = rels.iter().map(|rel| candidate(rel)).collect();
        let mut whole = Ranking::new();
        whole.set_query(Query::new("main"));
        whole.extend(&index);
        let mut stepped = Ranking::new();
        stepped.set_query(Query::new("main"));
        stepped.extend(&index[..2]);
        stepped.extend(&index);
        assert_eq!(stepped.top(SHOWN), whole.top(SHOWN));
        assert_eq!(stepped.matches(), whole.matches());
    }

    #[test]
    fn changing_the_query_starts_the_ranking_over() {
        let index = [candidate("alpha"), candidate("beta")];
        let mut ranking = Ranking::new();
        ranking.set_query(Query::new("alpha"));
        ranking.extend(&index);
        assert_eq!(ranking.top(SHOWN), [0]);
        ranking.set_query(Query::new("beta"));
        ranking.extend(&index);
        assert_eq!(ranking.top(SHOWN), [1]);
    }

    #[test]
    fn the_walk_hands_over_each_directory_nearest_first() {
        let vfs = MemVfs::new();
        vfs.add_file("/root/top.txt", b"");
        vfs.add_file("/root/a/b/deep.txt", b"");
        vfs.add_file("/root/a/mid.txt", b"");
        let (rels, outcome) = walked(&vfs, Limits::default());
        assert_eq!(outcome, Walked::Done { truncated: false });
        let at = |rel: &str| rels.iter().position(|r| r == rel).unwrap();
        assert!(at("top.txt") < at("a/mid.txt"));
        assert!(at("a/mid.txt") < at("a/b/deep.txt"));
        assert!(
            rels.contains(&"a".to_string()),
            "directories are indexed too"
        );
    }

    #[test]
    fn hidden_entries_are_neither_indexed_nor_entered() {
        let vfs = MemVfs::new();
        vfs.add_file("/root/.secret/inside.txt", b"");
        vfs.add_file("/root/.dot", b"");
        vfs.add_file("/root/shown.txt", b"");
        let (rels, _) = walked(&vfs, Limits::default());
        assert_eq!(rels, ["shown.txt"]);
    }

    #[test]
    fn a_limit_ends_the_walk_and_says_so() {
        let vfs = MemVfs::new();
        for i in 0..10 {
            vfs.add_file(format!("/root/f{i}"), b"");
        }
        let (rels, outcome) = walked(
            &vfs,
            Limits {
                max_depth: 16,
                max_entries: 4,
            },
        );
        assert_eq!(rels.len(), 4);
        assert_eq!(outcome, Walked::Done { truncated: true });

        vfs.add_file("/root/d1/d2/deep", b"");
        let (rels, outcome) = walked(
            &vfs,
            Limits {
                max_depth: 2,
                max_entries: 1000,
            },
        );
        assert!(!rels.contains(&"d1/d2/deep".to_string()));
        assert_eq!(outcome, Walked::Done { truncated: true });
    }

    #[test]
    fn a_cancelled_walk_stops_before_listing_anything() {
        let vfs = MemVfs::new();
        vfs.add_file("/root/f", b"");
        let mut seen = 0;
        let outcome = walk(
            &vfs,
            Path::new("/root"),
            false,
            Limits::default(),
            &AtomicBool::new(true),
            |batch| seen += batch.len(),
        );
        assert_eq!(outcome, Walked::Cancelled);
        assert_eq!(seen, 0);
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
            let index: Vec<Candidate> = rels.iter().map(|rel| candidate(rel)).collect();
            let typed = Query::new(&query).unwrap();
            let mut ranking = Ranking::new();
            ranking.set_query(Some(typed.clone()));
            ranking.extend(&index);
            let top = ranking.top(limit);
            prop_assert!(top.len() <= limit);
            let scores: Vec<i32> = top
                .iter()
                .map(|&p| score(&typed, &index[p].rel).expect("a result must match"))
                .collect();
            prop_assert!(scores.windows(2).all(|pair| pair[0] >= pair[1]));
            let expected = index.iter().filter(|c| score(&typed, &c.rel).is_some()).count();
            prop_assert_eq!(ranking.matches(), expected);
            prop_assert_eq!(top.len(), expected.min(limit));
        }
    }
}
