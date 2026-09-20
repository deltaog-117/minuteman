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

//! Finds the nearest entry whose name contains a query, looking in a directory first and then in
//! every directory beneath it, one level at a time — so `aerend` typed in `~` lands on
//! `~/Desktop/aerend` before `~/Desktop/old/backup/aerend`.
//!
//! Everything goes through `Vfs::list_dir`, one directory per call, and `cancel` is checked
//! between calls. That is what lets the caller run this off the render thread and drop a stale
//! search the moment the query changes, and what will let a remote backend serve it later: no
//! path is ever opened or `stat`ed here, only listed.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use shared::Vfs;

use crate::is_hidden;

/// What to look for: a non-empty, case-insensitive substring of an entry's name. Non-empty by
/// construction, because an empty query matches everything and would "find" the first entry of
/// the directory it started in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query(String);

impl Query {
    /// `None` for an empty `text`.
    pub fn new(text: &str) -> Option<Self> {
        (!text.is_empty()).then(|| Self(text.to_lowercase()))
    }

    pub fn matches(&self, name: &str) -> bool {
        name.to_lowercase().contains(&self.0)
    }
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
    Found(PathBuf),
    /// Nothing matched. `truncated` says a limit stopped the walk early, so a match may exist
    /// beyond it.
    NotFound {
        truncated: bool,
    },
    /// `cancel` was set before the walk finished.
    Cancelled,
}

/// Breadth-first search of `root` for the first entry matching `query`. Within a directory,
/// entries are examined in listing order (directories first, then by name), which is the order
/// the browser shows them, so a match in `root` itself is the one the cursor would reach first.
/// A directory that can't be listed is skipped, as it would be by `find`. Hidden entries are
/// neither matched nor entered unless `show_hidden`, so a search can't land on something the
/// user has chosen not to see.
pub fn find_below(
    vfs: &dyn Vfs,
    root: &Path,
    query: &Query,
    show_hidden: bool,
    limits: Limits,
    cancel: &AtomicBool,
) -> Outcome {
    let mut queue = VecDeque::from([(root.to_path_buf(), 1)]);
    let mut listed = 0usize;
    let mut truncated = false;

    while let Some((dir, depth)) = queue.pop_front() {
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
                return Outcome::NotFound { truncated: true };
            }
            if query.matches(&entry.name) {
                return Outcome::Found(entry.path);
            }
            if entry.is_dir {
                match depth < limits.max_depth {
                    true => queue.push_back((entry.path, depth + 1)),
                    false => truncated = true,
                }
            }
        }
    }
    Outcome::NotFound { truncated }
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
        Outcome::Found(PathBuf::from("/root").join(path))
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
                Outcome::Found(hit) => {
                    prop_assert!(query.matches(&name_of(&hit)));
                    let nearest = all_matches.iter().map(|e| depth(&e.path)).min().unwrap();
                    prop_assert_eq!(depth(&hit), nearest);
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
