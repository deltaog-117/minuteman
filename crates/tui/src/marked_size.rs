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

//! The total size of the marked entries, for the header's `◆ 3 marked │ 1.4 GiB` pill.
//!
//! Marks can sit in any directory and a marked folder means everything below it, so the sum needs
//! a walk that may take seconds. `MarkedSize` runs it on the blocking pool the way `search_job`
//! and `inspect` do, starting a fresh walk whenever the set of marks changes and cancelling the
//! one before it. Until the first answer lands the pill simply shows the count on its own, and
//! while a newer answer is on its way the previous total stays up rather than flickering off.
//!
//! The sum is taken when the marks change and is not refreshed after: a marked file that keeps
//! growing keeps its old size in the pill until the marks are touched again. Re-walking a marked
//! folder every half second would cost far more than the figure is worth.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::hud::format_size;
use crate::inspect::{Ending, TALLY_LIMIT, tally_dir};

/// What the marked entries add up to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Total {
    pub bytes: u64,
    /// `false` when the walk hit the entry limit, so `bytes` is a lower bound.
    pub exact: bool,
}

impl Total {
    /// The pill text: `1.4 GiB`, or `at least 1.4 GiB` for a lower bound.
    pub fn label(&self) -> String {
        if self.exact {
            format_size(self.bytes)
        } else {
            format!("at least {}", format_size(self.bytes))
        }
    }
}

/// Adds up what `paths` occupy: a file or link by its own length (a link counts as the link, not
/// its target), a folder by everything below it. A path that has vanished since it was marked
/// adds nothing. Returns `None` if `cancel` was set before the sum finished.
///
/// A marked folder already covers whatever is marked inside it, so paths under a marked folder
/// are skipped rather than counted twice. `limit` bounds how many entries the folder walks visit
/// in all, so marking `/` cannot keep the blocking pool busy for minutes.
pub fn total_of(paths: &[PathBuf], limit: u64, cancel: &AtomicBool) -> Option<Total> {
    let mut sorted: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    // Component-wise ordering puts everything below a folder right after it, so remembering the
    // latest folder is enough to recognise every path it covers.
    sorted.sort();
    sorted.dedup();

    let mut total = Total {
        bytes: 0,
        exact: true,
    };
    let mut budget = limit;
    let mut covering: Option<&Path> = None;
    for path in sorted {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        if covering.is_some_and(|folder| path.starts_with(folder)) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            continue;
        };
        if meta.is_dir() {
            covering = Some(path);
            let (tally, ending) = tally_dir(path, budget, cancel);
            match ending {
                Ending::Cancelled => return None,
                Ending::Truncated => total.exact = false,
                Ending::Complete => {}
            }
            total.bytes = total.bytes.saturating_add(tally.bytes);
            budget = budget.saturating_sub(tally.files + tally.dirs);
        } else {
            total.bytes = total.bytes.saturating_add(meta.len());
        }
    }
    Some(total)
}

struct Job {
    cancel: Arc<AtomicBool>,
    rx: UnboundedReceiver<Option<Total>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub struct MarkedSize {
    handle: tokio::runtime::Handle,
    /// The marks the running (or last finished) walk was started for.
    requested: Vec<PathBuf>,
    job: Option<Job>,
    total: Option<Total>,
}

impl MarkedSize {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self {
            handle,
            requested: Vec::new(),
            job: None,
            total: None,
        }
    }

    /// Call once per render tick with the current marks, sorted as
    /// `BrowserState::marked_paths` returns them. Starts a new walk when they changed and picks
    /// up a finished one.
    pub fn poll(&mut self, marks: &[PathBuf]) {
        if marks != self.requested.as_slice() {
            self.requested = marks.to_vec();
            // Dropping the old job cancels it; its answer would be for marks that are gone.
            self.job = None;
            if marks.is_empty() {
                self.total = None;
            } else {
                self.job = Some(self.start(marks.to_vec()));
            }
        }
        let Some(job) = self.job.as_mut() else { return };
        match job.rx.try_recv() {
            Ok(answer) => {
                // A cancelled or lost walk leaves the previous total as it was.
                if answer.is_some() {
                    self.total = answer;
                }
                self.job = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.job = None,
        }
    }

    /// The total for the current marks, or the previous one while a new walk is running; `None`
    /// when nothing is marked or no walk has finished yet.
    pub fn total(&self) -> Option<Total> {
        self.total
    }

    fn start(&self, marks: Vec<PathBuf>) -> Job {
        let (tx, rx) = unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        self.handle.spawn_blocking(move || {
            let _ = tx.send(total_of(&marks, TALLY_LIMIT, &cancel_bg));
        });
        Job { cancel, rx }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::time::{Duration, Instant};

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-marked-size-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, len: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; len]).unwrap();
    }

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// However many files are marked, in any directories, the total is the sum of their
        /// lengths — no more (nothing counted twice) and no less.
        #[test]
        fn the_total_of_marked_files_is_the_sum_of_their_sizes(
            sizes in proptest::collection::vec(0usize..3000, 0..8),
            nest in proptest::collection::vec(any::<bool>(), 8),
        ) {
            let root = scratch("sum");
            let paths: Vec<PathBuf> = sizes
                .iter()
                .enumerate()
                .map(|(i, len)| {
                    let path = if nest[i] {
                        root.join(format!("d{i}")).join(format!("f{i}"))
                    } else {
                        root.join(format!("f{i}"))
                    };
                    write(&path, *len);
                    path
                })
                .collect();

            let total = total_of(&paths, TALLY_LIMIT, &never()).unwrap();
            prop_assert_eq!(total.bytes, sizes.iter().sum::<usize>() as u64);
            prop_assert!(total.exact);
            std::fs::remove_dir_all(&root).unwrap();
        }

        /// The order the marks are given in, and a mark given twice, change nothing.
        #[test]
        fn order_and_repeats_do_not_change_the_total(
            sizes in proptest::collection::vec(1usize..500, 1..6),
        ) {
            let root = scratch("order");
            let mut paths: Vec<PathBuf> = sizes
                .iter()
                .enumerate()
                .map(|(i, len)| {
                    let path = root.join(format!("f{i}"));
                    write(&path, *len);
                    path
                })
                .collect();
            let forward = total_of(&paths, TALLY_LIMIT, &never());
            paths.reverse();
            paths.push(paths[0].clone());
            prop_assert_eq!(total_of(&paths, TALLY_LIMIT, &never()), forward);
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn a_marked_folder_counts_what_is_below_it_once_even_with_a_marked_child() {
        let root = scratch("covered");
        write(&root.join("dir/a"), 100);
        write(&root.join("dir/sub/b"), 200);
        write(&root.join("dir/sub/deeper/c"), 300);
        // `dir.txt` sorts right after `dir`; it is a sibling, not a child, and must still count.
        write(&root.join("dir.txt"), 7);

        let marks = [
            root.join("dir"),
            root.join("dir/sub"),
            root.join("dir/sub/b"),
            root.join("dir.txt"),
        ];
        let total = total_of(&marks, TALLY_LIMIT, &never()).unwrap();
        assert_eq!(total.bytes, 100 + 200 + 300 + 7);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_path_that_has_vanished_adds_nothing() {
        let root = scratch("vanished");
        write(&root.join("kept"), 40);
        let marks = [root.join("kept"), root.join("gone")];
        assert_eq!(
            total_of(&marks, TALLY_LIMIT, &never()).map(|t| t.bytes),
            Some(40)
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_symlink_counts_as_the_link_not_its_target() {
        let root = scratch("link");
        write(&root.join("big"), 5000);
        std::os::unix::fs::symlink(root.join("big"), root.join("link")).unwrap();
        let total = total_of(&[root.join("link")], TALLY_LIMIT, &never()).unwrap();
        assert!(total.bytes < 5000, "counted the target: {}", total.bytes);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hitting_the_entry_limit_makes_the_total_a_lower_bound() {
        let root = scratch("limit");
        for i in 0..10 {
            write(&root.join("dir").join(format!("f{i}")), 10);
        }
        let total = total_of(&[root.join("dir")], 3, &never()).unwrap();
        assert!(!total.exact);
        assert!(total.bytes < 100);
        assert!(total.label().starts_with("at least "));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_cancelled_sum_gives_no_answer() {
        let root = scratch("cancel");
        write(&root.join("a"), 1);
        let cancel = AtomicBool::new(true);
        assert_eq!(total_of(&[root.join("a")], TALLY_LIMIT, &cancel), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn nothing_marked_adds_up_to_zero() {
        assert_eq!(
            total_of(&[], TALLY_LIMIT, &never()),
            Some(Total {
                bytes: 0,
                exact: true
            })
        );
    }

    fn wait_for_total(sizes: &mut MarkedSize, marks: &[PathBuf]) -> Option<Total> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            sizes.poll(marks);
            if sizes.job.is_none() {
                return sizes.total();
            }
            assert!(Instant::now() < deadline, "the sum never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_tracker_follows_the_marks_and_forgets_them_when_they_clear() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("tracker");
        write(&root.join("a"), 10);
        write(&root.join("b"), 25);
        let mut sizes = MarkedSize::new(runtime.handle().clone());

        assert_eq!(sizes.total(), None, "nothing marked, nothing to show");
        let one = [root.join("a")];
        assert_eq!(wait_for_total(&mut sizes, &one).map(|t| t.bytes), Some(10));
        let two = [root.join("a"), root.join("b")];
        assert_eq!(wait_for_total(&mut sizes, &two).map(|t| t.bytes), Some(35));
        assert_eq!(wait_for_total(&mut sizes, &[]), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_previous_total_stays_up_while_a_new_sum_runs() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("stale");
        write(&root.join("a"), 10);
        write(&root.join("b"), 25);
        let mut sizes = MarkedSize::new(runtime.handle().clone());
        let one = [root.join("a")];
        wait_for_total(&mut sizes, &one);

        sizes.poll(&[root.join("a"), root.join("b")]);
        assert!(sizes.total().is_some(), "the pill must not flicker off");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn labels_are_plain_sizes_or_lower_bounds() {
        assert_eq!(
            Total {
                bytes: 0,
                exact: true
            }
            .label(),
            format_size(0)
        );
        assert_eq!(
            Total {
                bytes: 2048,
                exact: false
            }
            .label(),
            format!("at least {}", format_size(2048))
        );
    }
}
