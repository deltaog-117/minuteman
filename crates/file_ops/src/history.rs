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

//! Undo and redo for the operations this crate performs.
//!
//! Every finished operation is recorded as a `Change` that knows how to apply itself again and how
//! to revert itself. A batch (one paste of several files, one trash of a marked set) is committed
//! as a single `Entry`, so one undo steps back the whole batch. Reverting never destroys data
//! that may have changed since: a created file is removed only while it is still empty, and an
//! undone copy goes to the trash instead of being deleted.
//!
//! Not recorded, because they cannot be put back: a permanent delete, an overwrite, and anything
//! a shell command does.

use std::path::{Path, PathBuf};

use shared::Vfs;
use trash::Trashed;

use crate::{ConflictPolicy, FileOpsError};

/// How many entries are kept before the oldest is forgotten.
pub const DEFAULT_CAPACITY: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    #[error("{0} is gone")]
    Missing(PathBuf),
    #[error("{0} is no longer empty")]
    NotEmpty(PathBuf),
    #[error("{0}")]
    Restore(#[from] trash::TrashError),
    #[error(transparent)]
    Op(#[from] FileOpsError),
}

/// One recorded step, with enough to run it again (`apply`) or take it back (`revert`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A new, empty file or folder.
    Created { path: PathBuf, is_dir: bool },
    /// `from` was renamed or moved to `to`.
    Moved { from: PathBuf, to: PathBuf },
    /// `src` was copied to `dst`, which did not exist before.
    Copied { src: PathBuf, dst: PathBuf },
    /// `path` was sent to the desktop trash, where it is now `item`.
    Trashed { path: PathBuf, item: Trashed },
}

impl Change {
    /// Puts the change into effect again and returns the change as it now stands: a redone trash
    /// lands in the trash as a new item, so the record is refreshed.
    fn apply(&self, vfs: &dyn Vfs) -> Result<Change, UndoError> {
        match self {
            Change::Created { path, is_dir } => {
                if *is_dir {
                    crate::create_directory(vfs, path)?;
                } else {
                    crate::create_new_file(vfs, path)?;
                }
            }
            Change::Moved { from, to } => {
                crate::mv(vfs, from, to, ConflictPolicy::Abort)?;
            }
            Change::Copied { src, dst } => {
                crate::copy(vfs, src, dst, ConflictPolicy::Abort)?;
            }
            Change::Trashed { path, .. } => {
                let before = trash::snapshot()?;
                crate::trash(path)?;
                let item = trash::claim(&before, std::slice::from_ref(path))?
                    .pop()
                    .flatten()
                    .ok_or_else(|| {
                        trash::TrashError::from("the trashed item could not be found again")
                    })?;
                return Ok(Change::Trashed {
                    path: path.clone(),
                    item,
                });
            }
        }
        Ok(self.clone())
    }

    fn revert(&self, vfs: &dyn Vfs) -> Result<(), UndoError> {
        match self {
            Change::Created { path, is_dir } => {
                if !vfs.exists(path) {
                    return Err(UndoError::Missing(path.clone()));
                }
                let untouched = if *is_dir {
                    vfs.list_dir(path).map_err(FileOpsError::from)?.is_empty()
                } else {
                    file_size(vfs, path)? == 0
                };
                if !untouched {
                    return Err(UndoError::NotEmpty(path.clone()));
                }
                crate::delete(vfs, path)?;
            }
            Change::Moved { from, to } => {
                crate::mv(vfs, to, from, ConflictPolicy::Abort)?;
            }
            Change::Copied { dst, .. } => {
                if !vfs.exists(dst) {
                    return Err(UndoError::Missing(dst.clone()));
                }
                crate::trash(dst)?;
            }
            Change::Trashed { item, .. } => trash::restore(item)?,
        }
        Ok(())
    }

    /// The verb for the status line.
    fn verb(&self) -> &'static str {
        match self {
            Change::Created { .. } => "create",
            Change::Moved { from, to } if from.parent() == to.parent() => "rename",
            Change::Moved { .. } => "move",
            Change::Copied { .. } => "copy",
            Change::Trashed { .. } => "trash",
        }
    }

    fn subject(&self) -> &Path {
        match self {
            Change::Created { path, .. } | Change::Trashed { path, .. } => path,
            Change::Moved { to, .. } => to,
            Change::Copied { dst, .. } => dst,
        }
    }
}

fn file_size(vfs: &dyn Vfs, path: &Path) -> Result<u64, UndoError> {
    let parent = path.parent().unwrap_or(Path::new("/"));
    let entries = vfs.list_dir(parent).map_err(FileOpsError::from)?;
    entries
        .iter()
        .find(|entry| entry.path == path)
        .map(|entry| entry.size)
        .ok_or_else(|| UndoError::Missing(path.to_path_buf()))
}

/// What one batch did, in words: `rename a.txt`, `trash 3 items`.
fn label(changes: &[Change]) -> String {
    let Some(first) = changes.first() else {
        return String::new();
    };
    match changes {
        [_] => {
            let name = first.subject().file_name().unwrap_or_default();
            format!("{} {}", first.verb(), name.to_string_lossy())
        }
        _ => format!("{} {} items", first.verb(), changes.len()),
    }
}

/// Watches the trash around a batch of `file_ops::trash` calls, so each item that went in can be
/// recorded with the handle needed to restore it.
pub struct TrashTracker(Option<trash::Snapshot>);

impl TrashTracker {
    /// Call before the first item is sent. If the trash cannot be read, everything sent is
    /// reported as untracked rather than the batch failing.
    pub fn start() -> Self {
        Self(trash::snapshot().ok())
    }

    /// Call after sending `sent`: the change to record for each item that can be restored, and how
    /// many items could not be found again in the trash.
    pub fn finish(self, sent: &[PathBuf]) -> (Vec<Change>, usize) {
        let claimed = self
            .0
            .and_then(|before| trash::claim(&before, sent).ok())
            .unwrap_or_else(|| vec![None; sent.len()]);
        let untracked = claimed.iter().filter(|item| item.is_none()).count();
        let changes = sent
            .iter()
            .zip(claimed)
            .filter_map(|(path, item)| {
                item.map(|item| Change::Trashed {
                    path: path.clone(),
                    item,
                })
            })
            .collect();
        (changes, untracked)
    }
}

/// A batch of changes that is undone and redone as one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    changes: Vec<Change>,
}

/// What an undo or redo did.
#[derive(Debug)]
pub enum Report {
    /// There was nothing to step back through.
    Empty,
    /// The whole entry was taken back (or redone); `label` says what it was.
    Done { label: String },
    /// A step failed. The steps before it were taken back and stay undone; the failed step and
    /// the rest remain on the stack so the user can fix the cause and try again.
    Failed { label: String, error: UndoError },
}

/// The undo and redo stacks.
#[derive(Debug)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// Changes of a batch still running, committed as one entry by `commit`.
    pending: Vec<Change>,
    capacity: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl History {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            pending: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    /// Adds one finished step to the batch in progress.
    pub fn record(&mut self, change: Change) {
        self.pending.push(change);
    }

    /// Ends the batch in progress: what was recorded becomes one undoable entry, and anything that
    /// could have been redone is forgotten, since the history has moved on. Does nothing when
    /// nothing was recorded.
    pub fn commit(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        self.redo.clear();
        self.undo.push(Entry {
            changes: std::mem::take(&mut self.pending),
        });
        if self.undo.len() > self.capacity {
            self.undo.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || !self.pending.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Takes back the newest entry.
    pub fn undo(&mut self, vfs: &dyn Vfs) -> Report {
        self.commit();
        let Some(entry) = self.undo.pop() else {
            return Report::Empty;
        };
        let label = label(&entry.changes);
        for index in (0..entry.changes.len()).rev() {
            if let Err(error) = entry.changes[index].revert(vfs) {
                let mut changes = entry.changes;
                let taken_back = changes.split_off(index + 1);
                if !taken_back.is_empty() {
                    self.redo.push(Entry {
                        changes: taken_back,
                    });
                }
                self.undo.push(Entry { changes });
                return Report::Failed { label, error };
            }
        }
        self.redo.push(entry);
        Report::Done { label }
    }

    /// Does again the newest entry that was undone.
    pub fn redo(&mut self, vfs: &dyn Vfs) -> Report {
        let Some(entry) = self.redo.pop() else {
            return Report::Empty;
        };
        let label = label(&entry.changes);
        let mut applied = Vec::with_capacity(entry.changes.len());
        for (index, change) in entry.changes.iter().enumerate() {
            match change.apply(vfs) {
                Ok(done) => applied.push(done),
                Err(error) => {
                    if !applied.is_empty() {
                        self.undo.push(Entry { changes: applied });
                    }
                    self.redo.push(Entry {
                        changes: entry.changes[index..].to_vec(),
                    });
                    return Report::Failed { label, error };
                }
            }
        }
        self.undo.push(Entry { changes: applied });
        Report::Done { label }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use proptest::prelude::*;
    use shared::LocalVfs;

    use super::*;

    fn scratch(label: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "minuteman-history-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(history: &mut History, change: Change) {
        history.record(change);
        history.commit();
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn undoing_a_creation_removes_it_and_redo_makes_it_again() {
        let dir = scratch("create");
        let path = dir.join("new.txt");
        std::fs::write(&path, b"").unwrap();
        let mut history = History::default();
        entry(
            &mut history,
            Change::Created {
                path: path.clone(),
                is_dir: false,
            },
        );

        assert!(matches!(history.undo(&LocalVfs), Report::Done { .. }));
        assert!(!path.exists());
        assert!(matches!(history.redo(&LocalVfs), Report::Done { .. }));
        assert!(path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_created_file_that_was_written_to_is_not_removed() {
        let dir = scratch("written");
        let path = dir.join("notes.txt");
        std::fs::write(&path, b"").unwrap();
        let mut history = History::default();
        entry(
            &mut history,
            Change::Created {
                path: path.clone(),
                is_dir: false,
            },
        );
        std::fs::write(&path, b"precious").unwrap();

        let report = history.undo(&LocalVfs);

        assert!(matches!(
            report,
            Report::Failed {
                error: UndoError::NotEmpty(_),
                ..
            }
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"precious");
        assert!(history.can_undo(), "a refused undo stays on the stack");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_created_folder_with_something_inside_is_not_removed() {
        let dir = scratch("folder");
        let path = dir.join("d");
        std::fs::create_dir(&path).unwrap();
        let mut history = History::default();
        entry(
            &mut history,
            Change::Created {
                path: path.clone(),
                is_dir: true,
            },
        );
        std::fs::write(path.join("inside"), b"x").unwrap();

        assert!(matches!(history.undo(&LocalVfs), Report::Failed { .. }));
        assert!(path.join("inside").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undoing_a_move_puts_the_file_back_and_redo_moves_it_again() {
        let dir = scratch("move");
        let (from, to) = (dir.join("a"), dir.join("b"));
        std::fs::write(&to, b"data").unwrap();
        let mut history = History::default();
        entry(
            &mut history,
            Change::Moved {
                from: from.clone(),
                to: to.clone(),
            },
        );

        history.undo(&LocalVfs);
        assert_eq!(std::fs::read(&from).unwrap(), b"data");
        assert!(!to.exists());
        history.redo(&LocalVfs);
        assert_eq!(std::fs::read(&to).unwrap(), b"data");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undoing_a_copy_trashes_the_copy_and_keeps_the_original() {
        let dir = scratch("copy");
        let (src, dst) = (dir.join("orig"), dir.join("dup"));
        std::fs::write(&src, b"data").unwrap();
        std::fs::write(&dst, b"data").unwrap();
        let mut history = History::default();
        entry(
            &mut history,
            Change::Copied {
                src: src.clone(),
                dst: dst.clone(),
            },
        );

        history.undo(&LocalVfs);
        assert!(src.exists() && !dst.exists());
        history.redo(&LocalVfs);
        assert_eq!(std::fs::read(&dst).unwrap(), b"data");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undoing_a_trash_restores_the_file_and_redo_trashes_it_again() {
        let dir = scratch("trash");
        let path = dir.join("gone.txt");
        std::fs::write(&path, b"data").unwrap();
        let before = trash::snapshot().unwrap();
        crate::trash(&path).unwrap();
        let item = trash::claim(&before, std::slice::from_ref(&path))
            .unwrap()
            .pop()
            .flatten()
            .expect("the trash item is found");
        let mut history = History::default();
        entry(
            &mut history,
            Change::Trashed {
                path: path.clone(),
                item,
            },
        );

        assert!(matches!(history.undo(&LocalVfs), Report::Done { .. }));
        assert_eq!(std::fs::read(&path).unwrap(), b"data");
        assert!(matches!(history.redo(&LocalVfs), Report::Done { .. }));
        assert!(!path.exists());
        // And it can be undone once more: redo refreshed the record of where it went.
        assert!(matches!(history.undo(&LocalVfs), Report::Done { .. }));
        assert!(path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_batch_is_one_undo_step() {
        let dir = scratch("batch");
        for name in ["x", "y"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let mut history = History::default();
        history.record(Change::Created {
            path: dir.join("x"),
            is_dir: false,
        });
        history.record(Change::Created {
            path: dir.join("y"),
            is_dir: false,
        });

        let Report::Done { label } = history.undo(&LocalVfs) else {
            panic!("the batch is undone");
        };

        assert_eq!(label, "create 2 items");
        assert!(listing(&dir).is_empty());
        assert!(!history.can_undo());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failure_partway_keeps_what_was_not_undone_and_makes_the_rest_redoable() {
        let dir = scratch("partial");
        let (a, b, c, d) = (dir.join("a"), dir.join("b"), dir.join("c"), dir.join("d"));
        // `b` (the first move's result) is missing, so taking that move back fails; the second
        // move's undo runs first (newest first) and succeeds.
        std::fs::write(&d, b"d").unwrap();
        let mut history = History::default();
        history.record(Change::Moved {
            from: a.clone(),
            to: b,
        });
        history.record(Change::Moved {
            from: c.clone(),
            to: d,
        });

        let report = history.undo(&LocalVfs);

        assert!(matches!(report, Report::Failed { .. }));
        assert!(c.exists(), "the step before the failure was taken back");
        assert!(history.can_undo(), "the failed step stays undoable");
        assert!(
            history.can_redo(),
            "the step that was taken back can be redone"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_entry_forgets_what_could_have_been_redone() {
        let dir = scratch("forget");
        std::fs::write(dir.join("one"), b"").unwrap();
        std::fs::write(dir.join("two"), b"").unwrap();
        let mut history = History::default();
        entry(
            &mut history,
            Change::Created {
                path: dir.join("one"),
                is_dir: false,
            },
        );
        history.undo(&LocalVfs);
        assert!(history.can_redo());

        entry(
            &mut history,
            Change::Created {
                path: dir.join("two"),
                is_dir: false,
            },
        );

        assert!(!history.can_redo());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_oldest_entries_are_dropped_past_the_capacity() {
        let mut history = History::with_capacity(2);
        for n in 0..5 {
            entry(
                &mut history,
                Change::Created {
                    path: PathBuf::from(format!("/nowhere/{n}")),
                    is_dir: false,
                },
            );
        }
        assert_eq!(history.undo.len(), 2);
        assert_eq!(
            history.undo[0].changes[0].subject(),
            Path::new("/nowhere/3")
        );
    }

    #[test]
    fn undo_and_redo_with_nothing_recorded_report_empty() {
        let mut history = History::default();
        assert!(matches!(history.undo(&LocalVfs), Report::Empty));
        assert!(matches!(history.redo(&LocalVfs), Report::Empty));
    }

    #[derive(Debug, Clone)]
    enum Step {
        Create(u8),
        Move(u8, u8),
    }

    fn steps() -> impl Strategy<Value = Vec<Step>> {
        prop::collection::vec(
            prop_oneof![
                (0u8..5).prop_map(Step::Create),
                (0u8..5, 0u8..5).prop_map(|(a, b)| Step::Move(a, b)),
            ],
            0..16,
        )
    }

    proptest! {
        /// Whatever creates and moves were made, undoing them all leaves the folder as it began
        /// and redoing them all leaves it as it ended.
        #[test]
        fn undoing_everything_then_redoing_everything_round_trips(steps in steps()) {
            let dir = scratch("prop");
            let mut history = History::default();
            for step in steps {
                match step {
                    Step::Create(n) => {
                        let path = dir.join(n.to_string());
                        if path.exists() { continue; }
                        std::fs::write(&path, b"").unwrap();
                        entry(&mut history, Change::Created { path, is_dir: false });
                    }
                    Step::Move(a, b) => {
                        let (from, to) = (dir.join(a.to_string()), dir.join(b.to_string()));
                        if a == b || !from.exists() || to.exists() { continue; }
                        std::fs::rename(&from, &to).unwrap();
                        entry(&mut history, Change::Moved { from, to });
                    }
                }
            }
            let end = listing(&dir);

            while history.can_undo() {
                let done = matches!(history.undo(&LocalVfs), Report::Done { .. });
                prop_assert!(done);
            }
            prop_assert!(listing(&dir).is_empty());
            while history.can_redo() {
                let done = matches!(history.redo(&LocalVfs), Report::Done { .. });
                prop_assert!(done);
            }
            prop_assert_eq!(listing(&dir), end);
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
