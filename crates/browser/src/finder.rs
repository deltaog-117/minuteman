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

//! The flat index of every entry beneath a root that the fuzzy finder (`f`) ranks, gathered nearest
//! level first. The ranking itself belongs to the finder plugin; this is the walk the interface
//! does for it, because it is the interface that knows how to read every kind of path the browser
//! can show.
//!
//! The index is built through `Vfs::list_dir` alone, one directory per call, handing each
//! directory's entries to a callback as it goes. That is what lets the caller show results while
//! the walk is still running, cancel it between directories, and run it over a remote backend.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use shared::Vfs;

use crate::is_hidden;
use crate::search::Limits;

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
    use shared::MemVfs;

    fn candidate(rel: &str) -> Candidate {
        Candidate {
            path: PathBuf::from("/root").join(rel),
            rel: rel.to_string(),
            is_dir: false,
        }
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
}
