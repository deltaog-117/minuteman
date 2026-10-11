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

//! Counting what is below a folder: how many files and folders, how many bytes. A walk can take
//! seconds on a big tree, so callers run it on the blocking pool and hand it a cancel flag.
//! The marked-size pill and the host's `tally` request (which the Inspect plugin asks) share it, so
//! the number a folder's panel shows and the total the header shows for marks are one count.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{FileKind, Vfs};

/// A folder with more entries than this is reported as "at least" rather than walked to the end,
/// so tallying `/` cannot keep the blocking pool busy for minutes.
pub const TALLY_LIMIT: u64 = 500_000;

/// What a walk of a folder found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub files: u64,
    pub dirs: u64,
    /// The files' own sizes added up (a symlink counts as the link, not its target).
    pub bytes: u64,
    /// Folders inside that could not be read.
    pub unreadable: u64,
    /// The length of the biggest single file, which decides whether the whole will fit on a
    /// filesystem with a cap on file size.
    pub largest: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Complete,
    /// Stopped at the entry limit; the numbers are a lower bound.
    Truncated,
    Cancelled,
}

/// Walks `root` counting what is below it, without following symlinks (so a link loop cannot
/// hang it) and stopping early once `limit` entries were seen or `cancel` is set.
pub fn tally_dir(vfs: &dyn Vfs, root: &Path, limit: u64, cancel: &AtomicBool) -> (Tally, Ending) {
    let mut tally = Tally::default();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = vfs.scan_dir(&dir) else {
            tally.unreadable += 1;
            continue;
        };
        for entry in entries {
            if cancel.load(Ordering::Relaxed) {
                return (tally, Ending::Cancelled);
            }
            if tally.files + tally.dirs >= limit {
                return (tally, Ending::Truncated);
            }
            match entry.metadata {
                Some(meta) if meta.kind == FileKind::Dir => {
                    tally.dirs += 1;
                    pending.push(entry.path);
                }
                Some(meta) => {
                    tally.files += 1;
                    tally.bytes += meta.len;
                    tally.largest = tally.largest.max(meta.len);
                }
                None => tally.unreadable += 1,
            }
        }
    }
    (tally, Ending::Complete)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("minuteman-tally-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_folder_is_counted_with_its_files_folders_and_bytes() {
        let root = scratch("tally");
        std::fs::create_dir_all(root.join("a").join("b")).unwrap();
        std::fs::write(root.join("one"), [0u8; 10]).unwrap();
        std::fs::write(root.join("a").join("two"), [0u8; 20]).unwrap();
        std::fs::write(root.join("a").join("b").join("three"), [0u8; 5]).unwrap();

        let (tally, ending) = tally_dir(&crate::LocalVfs, &root, 100, &AtomicBool::new(false));
        assert_eq!(ending, Ending::Complete);
        assert_eq!(
            tally,
            Tally {
                files: 3,
                dirs: 2,
                bytes: 35,
                unreadable: 0,
                largest: 20
            }
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_walk_stops_at_its_limit_and_when_cancelled() {
        let root = scratch("limits");
        for i in 0..6 {
            std::fs::write(root.join(format!("f{i}")), b"").unwrap();
        }
        let (tally, ending) = tally_dir(&crate::LocalVfs, &root, 3, &AtomicBool::new(false));
        assert_eq!((tally.files, ending), (3, Ending::Truncated));
        let (tally, ending) = tally_dir(&crate::LocalVfs, &root, 100, &AtomicBool::new(true));
        assert_eq!((tally.files, ending), (0, Ending::Cancelled));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_symlink_loop_is_counted_once_not_followed() {
        let root = scratch("loop");
        std::os::unix::fs::symlink(&root, root.join("again")).unwrap();
        let (tally, ending) = tally_dir(&crate::LocalVfs, &root, 100, &AtomicBool::new(false));
        assert_eq!((tally.files, tally.dirs, ending), (1, 0, Ending::Complete));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_on_another_backend_is_tallied_without_following_links() {
        let mem = crate::MemVfs::new();
        mem.add_file("/r/one", vec![0u8; 10]);
        mem.add_file("/r/a/two", vec![0u8; 20]);
        mem.add_file("/r/a/b/three", vec![0u8; 5]);
        mem.add_symlink("/r/a/loop", "/r");

        let (tally, ending) = tally_dir(&mem, Path::new("/r"), 100, &AtomicBool::new(false));

        assert_eq!(ending, Ending::Complete);
        assert_eq!(
            (tally.files, tally.dirs),
            (4, 2),
            "the link is one entry, not followed"
        );
        assert_eq!(
            tally.bytes,
            35 + "/r".len() as u64,
            "a link counts as the link"
        );
    }
}
