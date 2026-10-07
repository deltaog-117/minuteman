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

//! Fuzzy search for an entry by name, nearest first: it looks in a directory, then in every
//! directory beneath it one level at a time, and answers with the best-scoring matches of the
//! first level that has any. So `aerend` typed in `~` lands on `~/Desktop/aerend` before
//! `~/Desktop/old/backup/aerend`, and within a level a tighter match (`aerend`) beats a looser one
//! (`a-e-r-e-n-d`). A name matches when it holds the typed letters in order, not necessarily side
//! by side: `aernd` finds `aerend`.
//!
//! Everything goes through `Vfs::list_dir`, one directory per call, and `cancel` is checked
//! between calls. That is what lets the caller run this off the render thread and drop a stale
//! search the moment the query changes, and what will let a remote backend serve it later: no
//! path is ever opened or `stat`ed here, only listed.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use shared::{DirEntryInfo, Vfs};

use crate::is_hidden;

/// How many of the best matches a search keeps, for stepping through with the arrow keys.
pub const KEPT: usize = 20;

/// What to look for: a non-empty run of letters that must appear, in order, in an entry's name.
/// Non-empty by construction, because an empty query matches everything and would "find" the
/// first entry of the directory it started in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// As typed, for the small bonus a matching capital earns.
    typed: Vec<char>,
    lower: Vec<char>,
}

// Scoring. A name's score is the best way of laying the typed letters over it; higher is better.
// Only the differences matter, and the weights are tuned so that: a run of letters side by side
// beats scattered ones, a letter at the start of a name or of a word beats one in the middle, and
// any name holding the whole query as one piece beats every name that does not.
const MATCH: i32 = 16;
const ADJACENT: i32 = 12;
const WORD_START: i32 = 10;
const NAME_START: i32 = 8;
const SAME_CASE: i32 = 2;
const GAP: i32 = 1;
const LEADING_GAP: i32 = 1;
const LEADING_GAP_CAP: i32 = 6;
const WHOLE_PIECE: i32 = 1_000;
/// Stands for "cannot be matched this way"; far enough below any score to survive additions.
const NONE: i32 = i32::MIN / 4;

fn fold(c: char) -> char {
    // Nearly every name is ASCII, and the general path builds an iterator per character.
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

fn is_separator(c: char) -> bool {
    matches!(c, '.' | '_' | '-' | ' ' | '/' | '(' | '[')
}

impl Query {
    /// `None` for an empty `text`.
    pub fn new(text: &str) -> Option<Self> {
        let typed: Vec<char> = text.chars().collect();
        if typed.is_empty() {
            return None;
        }
        let lower = typed.iter().copied().map(fold).collect();
        Some(Self { typed, lower })
    }

    /// Whether this query is `shorter` with more letters after it (or the same letters), ignoring
    /// case. Everything that matches this query then also matches `shorter`, because a name that
    /// holds `abc` in order holds `ab` in order. That lets a ranking narrow as letters are typed
    /// instead of starting over.
    pub fn extends(&self, shorter: &Query) -> bool {
        self.lower.starts_with(&shorter.lower)
    }

    pub fn matches(&self, name: &str) -> bool {
        self.score(name).is_some()
    }

    /// How well `name` fits, or `None` if it does not hold the letters in order. Capitals in the
    /// query count only as a small bonus, so `README` still finds `readme`.
    pub fn score(&self, name: &str) -> Option<i32> {
        // Most names in a big folder fail here, so it must not allocate.
        if !self.is_subsequence_of(name.chars().map(fold)) {
            return None;
        }
        let chars: Vec<char> = name.chars().collect();
        let lower: Vec<char> = chars.iter().copied().map(fold).collect();

        let m = self.lower.len();
        let n = lower.len();
        // `previous[j]`: the best score with the letters so far laid out and the last of them on
        // `name[j]`. Each row is filled left to right with a running best of the gap-discounted
        // scores before it, so a name costs `m * n`, not `m * n * n`.
        let mut previous = vec![NONE; n];
        for (i, &want) in self.lower.iter().enumerate() {
            let mut current = vec![NONE; n];
            let mut gapped = NONE;
            for j in 0..n {
                // `gapped` = best of `previous[k] - (j - 1 - k) * GAP` for `k <= j - 2`.
                if j >= 2 {
                    gapped = (gapped - GAP).max(previous[j - 2] - GAP);
                } else {
                    gapped = NONE;
                }
                if lower[j] != want {
                    continue;
                }
                let before = if i == 0 {
                    -(j as i32).min(LEADING_GAP_CAP) * LEADING_GAP
                } else {
                    let adjacent = if j >= 1 && previous[j - 1] > NONE {
                        previous[j - 1] + ADJACENT
                    } else {
                        NONE
                    };
                    adjacent.max(gapped)
                };
                if before <= NONE {
                    continue;
                }
                let mut here = MATCH + before;
                if j == 0 {
                    here += NAME_START;
                }
                if j == 0
                    || is_separator(chars[j - 1])
                    || (chars[j - 1].is_lowercase() && chars[j].is_uppercase())
                {
                    here += WORD_START;
                }
                if chars[j] == self.typed[i] {
                    here += SAME_CASE;
                }
                current[j] = here;
            }
            previous = current;
        }

        let best = previous.into_iter().max().filter(|&score| score > NONE)?;
        let whole = lower
            .windows(m)
            .any(|window| window == self.lower.as_slice());
        // Among equal fits the shorter name is the closer one.
        Some(best + if whole { WHOLE_PIECE } else { 0 } - n as i32)
    }

    fn is_subsequence_of(&self, folded: impl Iterator<Item = char>) -> bool {
        let mut wanted = self.lower.iter();
        let mut next = wanted.next();
        for c in folded {
            if Some(&c) == next {
                next = wanted.next();
            }
        }
        next.is_none()
    }
}

/// One scored candidate: the score, its position in the order it was met (so equal scores keep
/// the listing's order), and where it is.
type Scored = (i32, usize, PathBuf);

/// The best `KEPT` of `scored`, best first.
fn best(mut scored: Vec<Scored>) -> Vec<PathBuf> {
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(KEPT)
        .map(|(_, _, path)| path)
        .collect()
}

/// The best matches among `entries` — one directory's listing, as the browser holds it — best
/// first and at most `KEPT`. This is what a search's first level is, so a hit right here shows at
/// once, without a walk.
pub fn rank_entries(query: &Query, entries: &[DirEntryInfo]) -> Vec<PathBuf> {
    best(
        entries
            .iter()
            .enumerate()
            .filter_map(|(order, entry)| {
                query
                    .score(&entry.name)
                    .map(|score| (score, order, entry.path.clone()))
            })
            .collect(),
    )
}

/// Bounds on one search, so a search from `/` can't walk the whole disk. Depth counts directory
/// levels below the root (1 is the root's own entries); the entry cap counts every entry
/// listed, which is what the walk pays for. Depth also cuts a symlink loop short, since
/// `Vfs::list_dir` reports a link to a directory as a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_depth: usize,
    pub max_entries: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 16,
            max_entries: 200_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The best matches of the nearest level that had any, best first; never empty.
    Found(Vec<PathBuf>),
    /// Nothing matched. `truncated` says a limit stopped the walk early, so a match may exist
    /// beyond it.
    NotFound { truncated: bool },
    /// `cancel` was set before the walk finished.
    Cancelled,
}

/// Breadth-first search of `root` for entries matching `query`. A level is finished before the
/// answer is given, so the best match of the nearest level wins, not merely the first one met. A
/// directory that can't be listed is skipped, as it would be by `find`. Hidden entries are neither
/// matched nor entered unless `show_hidden`, so a search can't land on something the user has
/// chosen not to see. If the entry limit stops the walk partway through a level that already
/// has matches, those are the answer.
pub fn find_below(
    vfs: &dyn Vfs,
    root: &Path,
    query: &Query,
    show_hidden: bool,
    limits: Limits,
    cancel: &AtomicBool,
) -> Outcome {
    let mut queue = VecDeque::from([(root.to_path_buf(), 1)]);
    let mut level = 1;
    let mut hits: Vec<Scored> = Vec::new();
    let mut listed = 0usize;
    let mut truncated = false;

    while let Some((dir, depth)) = queue.pop_front() {
        if depth > level {
            // The level before this one is done; if it had any match, that is the answer.
            if !hits.is_empty() {
                return Outcome::Found(best(hits));
            }
            level = depth;
        }
        if cancel.load(Ordering::Relaxed) {
            return Outcome::Cancelled;
        }
        let Ok(entries) = vfs.list_dir(&dir) else {
            continue;
        };
        for entry in entries {
            if !show_hidden && is_hidden(&entry.name) {
                continue;
            }
            listed += 1;
            if listed > limits.max_entries {
                return match hits.is_empty() {
                    true => Outcome::NotFound { truncated: true },
                    false => Outcome::Found(best(hits)),
                };
            }
            if let Some(score) = query.score(&entry.name) {
                hits.push((score, hits.len(), entry.path.clone()));
            }
            if entry.is_dir {
                match depth < limits.max_depth {
                    true => queue.push_back((entry.path, depth + 1)),
                    false => truncated = true,
                }
            }
        }
    }
    match hits.is_empty() {
        true => Outcome::NotFound { truncated },
        false => Outcome::Found(best(hits)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use shared::{DirEntryInfo, VfsError};
    use std::collections::BTreeMap;

    /// A directory tree held in memory. Only listing is implemented, which is all the search is
    /// allowed to use — any other call panics, so a search that reached for one would fail here.
    #[derive(Default)]
    struct TreeVfs {
        dirs: BTreeMap<PathBuf, Vec<DirEntryInfo>>,
    }

    impl TreeVfs {
        /// Adds `path` (relative to `/root`), creating the directories above it.
        fn add(&mut self, path: &str, is_dir: bool) {
            let mut parent = PathBuf::from("/root");
            let parts: Vec<&str> = path.split('/').collect();
            for (i, part) in parts.iter().enumerate() {
                let entry_path = parent.join(part);
                let last = i + 1 == parts.len();
                let entry_is_dir = !last || is_dir;
                let listing = self.dirs.entry(parent.clone()).or_default();
                match listing.iter_mut().find(|e| e.path == entry_path) {
                    // A later path through an earlier "file" makes it a directory, as it would
                    // have to be on a real disk.
                    Some(existing) => existing.is_dir |= entry_is_dir,
                    None => listing.push(DirEntryInfo {
                        name: (*part).into(),
                        path: entry_path.clone(),
                        is_dir: entry_is_dir,
                        size: 0,
                        modified: None,
                        mode: None,
                    }),
                }
                if entry_is_dir {
                    self.dirs.entry(entry_path.clone()).or_default();
                }
                parent = entry_path;
            }
        }
    }

    impl Vfs for TreeVfs {
        fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError> {
            let mut entries = self
                .dirs
                .get(path)
                .cloned()
                .ok_or_else(|| VfsError::NotFound(path.to_path_buf()))?;
            entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
            Ok(entries)
        }
        fn metadata(&self, _: &Path) -> Result<shared::Metadata, VfsError> {
            unimplemented!("a search only lists")
        }
        fn symlink_metadata(&self, _: &Path) -> Result<shared::Metadata, VfsError> {
            unimplemented!("a search only lists")
        }
        fn read_link(&self, _: &Path) -> Result<PathBuf, VfsError> {
            unimplemented!("a search only lists")
        }
        fn open_read(&self, _: &Path) -> Result<Box<dyn shared::ReadSeek>, VfsError> {
            unimplemented!("a search only lists")
        }
        fn scan_dir(&self, _: &Path) -> Result<Vec<shared::ScanEntry>, VfsError> {
            unimplemented!("a search only lists")
        }
        fn local_path(&self, _: &Path) -> Option<PathBuf> {
            None
        }
        fn create_write(&self, _: &Path) -> Result<Box<dyn shared::WriteSeek>, VfsError> {
            unimplemented!("a search only lists")
        }
        fn create_symlink(&self, _: &Path, _: &Path) -> Result<(), VfsError> {
            unimplemented!("a search only lists")
        }
        fn set_mode(&self, _: &Path, _: u32) -> Result<(), VfsError> {
            unimplemented!("a search only lists")
        }
        fn replace(&self, _: &Path, _: &Path) -> Result<(), VfsError> {
            unimplemented!("a search only lists")
        }
        fn is_dir(&self, _: &Path) -> bool {
            unimplemented!("a search only lists")
        }
        fn exists(&self, _: &Path) -> bool {
            unimplemented!("a search only lists")
        }
        fn create_dir(&self, _: &Path) -> Result<(), VfsError> {
            unimplemented!("a search only lists")
        }
        fn create_file(&self, _: &Path) -> Result<(), VfsError> {
            unimplemented!("a search only lists")
        }
        fn touch(&self, _: &Path) -> Result<(), VfsError> {
            unimplemented!("a search only lists")
        }
        fn copy_file(&self, _: &Path, _: &Path) -> Result<(), VfsError> {
            unimplemented!("a search only lists")
        }
        fn rename(&self, _: &Path, _: &Path) -> Result<(), VfsError> {
            unimplemented!("a search only lists")
        }
        fn remove_file(&self, _: &Path) -> Result<(), VfsError> {
            unimplemented!("a search only lists")
        }
        fn remove_dir_all(&self, _: &Path) -> Result<(), VfsError> {
            unimplemented!("a search only lists")
        }
    }

    fn search(tree: &TreeVfs, text: &str, show_hidden: bool, limits: Limits) -> Outcome {
        let query = Query::new(text).expect("tests use non-empty queries");
        find_below(
            tree,
            Path::new("/root"),
            &query,
            show_hidden,
            limits,
            &AtomicBool::new(false),
        )
    }

    fn found(path: &str) -> Outcome {
        Outcome::Found(vec![PathBuf::from("/root").join(path)])
    }

    fn found_all(paths: &[&str]) -> Outcome {
        Outcome::Found(
            paths
                .iter()
                .map(|path| PathBuf::from("/root").join(path))
                .collect(),
        )
    }

    #[test]
    fn an_empty_query_is_not_a_query() {
        assert_eq!(Query::new(""), None);
        assert!(Query::new("a").is_some());
    }

    #[test]
    fn matching_ignores_case() {
        let query = Query::new("AeReNd").unwrap();
        assert!(query.matches("aerend"));
        assert!(query.matches("My-AEREND-notes"));
        assert!(!query.matches("aeren"));
    }

    #[test]
    fn letters_must_appear_in_order_but_need_not_touch() {
        let query = Query::new("aernd").unwrap();
        assert!(query.matches("aerend"));
        assert!(query.matches("a-e-r-n-d.txt"));
        assert!(!query.matches("dnreaa"), "right letters, wrong order");
        assert!(!query.matches("aern"), "a letter is missing");
        assert!(
            !Query::new("aa").unwrap().matches("a"),
            "a letter is used once"
        );
    }

    fn score(query: &str, name: &str) -> i32 {
        Query::new(query)
            .unwrap()
            .score(name)
            .expect("should match")
    }

    #[test]
    fn a_tighter_fit_scores_higher() {
        assert!(
            score("aerend", "aerend") > score("aerend", "aerend.txt"),
            "shorter wins"
        );
        assert!(
            score("aer", "aerend") > score("aer", "a-e-r"),
            "side by side beats scattered"
        );
        assert!(score("rend", "aerend") > score("rend", "r_e_n_d"));
        assert!(
            score("ab", "ab_cd") > score("ab", "xab_cd"),
            "the start of a name beats the middle"
        );
    }

    #[test]
    fn a_word_start_beats_the_middle_of_a_word() {
        assert!(
            score("nt", "my-notes") > score("nt", "mynotes"),
            "after a separator"
        );
        assert!(
            score("mn", "myNotes") > score("mn", "mynotes"),
            "a capital after lowercase"
        );
    }

    #[test]
    fn a_whole_piece_beats_any_scattered_match() {
        // The scattered name has every bonus going for it; the whole-piece one still wins.
        assert!(score("test", "xx_test_xx") > score("test", "t-e-s-t"));
        assert!(score("note", "my-notebook") > score("note", "n_o_t_e"));
    }

    #[test]
    fn a_matching_capital_is_a_small_bonus_not_a_requirement() {
        assert!(score("README", "README") > score("README", "readme"));
        assert!(Query::new("README").unwrap().matches("readme"));
        assert!(Query::new("readme").unwrap().matches("README"));
    }

    #[test]
    fn multibyte_names_score_without_panicking() {
        assert!(Query::new("日語").unwrap().matches("日本語.txt"));
        assert!(Query::new("é").unwrap().matches("café"));
        assert!(!Query::new("z").unwrap().matches("日本語"));
    }

    #[test]
    fn a_match_in_the_root_itself_wins() {
        let mut tree = TreeVfs::default();
        tree.add("deep/er/aerend", true);
        tree.add("aerend.txt", false);
        assert_eq!(
            search(&tree, "aerend", false, Limits::default()),
            found("aerend.txt")
        );
    }

    #[test]
    fn within_a_level_the_best_match_comes_first_and_the_rest_follow_in_order() {
        let mut tree = TreeVfs::default();
        // Listed in this order (dirs first, then by name): a_b_c_d, abcd, abcdef.
        tree.add("a_b_c_d", false);
        tree.add("abcd", false);
        tree.add("abcdef", false);
        assert_eq!(
            search(&tree, "abcd", false, Limits::default()),
            found_all(&["abcd", "abcdef", "a_b_c_d"])
        );
    }

    #[test]
    fn the_whole_nearest_level_is_ranked_not_just_the_first_hit_met() {
        let mut tree = TreeVfs::default();
        // `a` sorts first, but its match is looser than the one in `z` at the same depth.
        tree.add("a/x_t_a_r_g_e_t", false);
        tree.add("z/target", false);
        assert_eq!(
            search(&tree, "target", false, Limits::default()),
            found_all(&["z/target", "a/x_t_a_r_g_e_t"])
        );
    }

    #[test]
    fn only_the_best_few_are_kept() {
        let mut tree = TreeVfs::default();
        for i in 0..50 {
            tree.add(&format!("file{i:02}"), false);
        }
        let Outcome::Found(hits) = search(&tree, "file", false, Limits::default()) else {
            panic!("expected hits");
        };
        assert_eq!(hits.len(), KEPT);
        assert_eq!(
            hits[0],
            PathBuf::from("/root/file00"),
            "equal fits keep listing order"
        );
    }

    #[test]
    fn running_into_the_entry_limit_keeps_the_matches_already_found() {
        let mut tree = TreeVfs::default();
        for name in ["aa", "ab", "ac", "ad", "ae"] {
            tree.add(name, false);
        }
        let tight = Limits {
            max_entries: 3,
            ..Limits::default()
        };
        assert_eq!(
            search(&tree, "a", false, tight),
            found_all(&["aa", "ab", "ac"])
        );
    }

    #[test]
    fn rank_entries_orders_one_listing_like_the_walk_would() {
        let entry = |name: &str| DirEntryInfo {
            name: name.into(),
            path: PathBuf::from("/d").join(name),
            is_dir: false,
            size: 0,
            modified: None,
            mode: None,
        };
        let entries = [
            entry("a_b_c_d"),
            entry("zzz"),
            entry("abcd"),
            entry("abcdef"),
        ];
        let query = Query::new("abcd").unwrap();
        assert_eq!(
            rank_entries(&query, &entries),
            [
                PathBuf::from("/d/abcd"),
                PathBuf::from("/d/abcdef"),
                PathBuf::from("/d/a_b_c_d")
            ]
        );
        assert!(rank_entries(&Query::new("q").unwrap(), &entries).is_empty());
    }

    #[test]
    fn a_shallower_match_beats_a_deeper_one_in_an_earlier_directory() {
        let mut tree = TreeVfs::default();
        // `a` sorts before `z`, so a depth-first walk would reach the deep match first.
        tree.add("a/b/c/target", true);
        tree.add("z/target", true);
        assert_eq!(
            search(&tree, "target", false, Limits::default()),
            found("z/target")
        );
    }

    #[test]
    fn a_missing_name_reports_not_found_untruncated() {
        let mut tree = TreeVfs::default();
        tree.add("a/b.txt", false);
        assert_eq!(
            search(&tree, "zzz", false, Limits::default()),
            Outcome::NotFound { truncated: false }
        );
    }

    #[test]
    fn hidden_entries_are_skipped_unless_shown() {
        let mut tree = TreeVfs::default();
        tree.add(".secret/target", true);
        tree.add(".target", false);
        assert_eq!(
            search(&tree, "target", false, Limits::default()),
            Outcome::NotFound { truncated: false }
        );
        assert_eq!(
            search(&tree, "target", true, Limits::default()),
            found(".target")
        );
    }

    #[test]
    fn the_depth_limit_stops_the_walk_and_says_so() {
        let mut tree = TreeVfs::default();
        tree.add("a/b/c/target", true);
        let shallow = Limits {
            max_depth: 2,
            ..Limits::default()
        };
        assert_eq!(
            search(&tree, "target", false, shallow),
            Outcome::NotFound { truncated: true }
        );
        let deep = Limits {
            max_depth: 4,
            ..Limits::default()
        };
        assert_eq!(search(&tree, "target", false, deep), found("a/b/c/target"));
    }

    #[test]
    fn the_entry_limit_stops_the_walk_and_says_so() {
        let mut tree = TreeVfs::default();
        for name in ["a", "b", "c", "d"] {
            tree.add(name, false);
        }
        tree.add("d/target", true);
        let tight = Limits {
            max_entries: 3,
            ..Limits::default()
        };
        assert_eq!(
            search(&tree, "target", false, tight),
            Outcome::NotFound { truncated: true }
        );
    }

    #[test]
    fn a_directory_that_cannot_be_listed_is_skipped() {
        let mut tree = TreeVfs::default();
        tree.add("gone/x", false);
        tree.add("z/target", true);
        tree.dirs.remove(Path::new("/root/gone"));
        assert_eq!(
            search(&tree, "target", false, Limits::default()),
            found("z/target")
        );
    }

    #[test]
    fn a_set_cancel_flag_ends_the_walk_before_listing_anything() {
        let mut tree = TreeVfs::default();
        tree.add("target", false);
        let query = Query::new("target").unwrap();
        let cancel = AtomicBool::new(true);
        assert_eq!(
            find_below(
                &tree,
                Path::new("/root"),
                &query,
                false,
                Limits::default(),
                &cancel
            ),
            Outcome::Cancelled
        );
    }

    /// Depth of `path` below `/root`: 1 for an entry of the root itself.
    fn depth(path: &Path) -> usize {
        path.strip_prefix("/root").unwrap().components().count()
    }

    fn name_of(path: &Path) -> String {
        path.file_name().unwrap().to_string_lossy().into_owned()
    }

    proptest! {
        /// Over random trees: a hit really matches, no matching entry sits closer to the root
        /// than the hit, and "not found" means nothing in the whole tree matches.
        #[test]
        fn the_hit_matches_and_nothing_nearer_does(
            paths in proptest::collection::vec(
                proptest::collection::vec("[a-c]{1,2}", 1..5), 0..30
            ),
            needle in "[a-c]{1,2}",
        ) {
            let mut tree = TreeVfs::default();
            for parts in &paths {
                tree.add(&parts.join("/"), false);
            }
            let query = Query::new(&needle).unwrap();
            let all_matches: Vec<&DirEntryInfo> = tree
                .dirs
                .values()
                .flatten()
                .filter(|entry| query.matches(&entry.name))
                .collect();

            match search(&tree, &needle, true, Limits::default()) {
                Outcome::Found(hits) => {
                    prop_assert!(!hits.is_empty() && hits.len() <= KEPT);
                    let nearest = all_matches.iter().map(|e| depth(&e.path)).min().unwrap();
                    for hit in &hits {
                        prop_assert!(query.matches(&name_of(hit)));
                        prop_assert_eq!(depth(hit), nearest);
                    }
                    // Best first: scores never rise down the list.
                    let scores: Vec<i32> = hits
                        .iter()
                        .map(|hit| query.score(&name_of(hit)).unwrap())
                        .collect();
                    prop_assert!(scores.windows(2).all(|pair| pair[0] >= pair[1]));
                }
                Outcome::NotFound { truncated } => {
                    prop_assert!(!truncated);
                    prop_assert!(all_matches.is_empty());
                }
                Outcome::Cancelled => prop_assert!(false, "nothing set the cancel flag"),
            }
        }
    }
}
