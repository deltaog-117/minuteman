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
//! An archive or a folder an extract made is not empty, so it is not a `Created`: undoing it sends
//! it to the trash and redoing it takes it back out, so the contents are never lost.
//!
//! Not recorded, because they cannot be put back: a permanent delete, an overwrite, and anything
//! a shell command does.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use shared::Vfs;
use trash::Trashed;

use crate::{ConflictPolicy, FileOpsError, ProgressFn};

mod id;
mod merge;
mod store;

pub use id::EntryId;
pub use merge::Merger;
pub use store::{DecodeError, Restored};

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
    /// A compress or an extract wrote `path`, which did not exist before. `held` is where it sits
    /// in the trash while the change is undone, and `None` while it is in place.
    Produced {
        path: PathBuf,
        product: Product,
        held: Option<Trashed>,
    },
}

/// What a `Change::Produced` is, for the status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// The archive a compress wrote.
    Archive,
    /// The folder an extract unpacked into.
    Folder,
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
            Change::Produced {
                path,
                product,
                held,
            } => {
                let item = held
                    .as_ref()
                    .ok_or_else(|| UndoError::Missing(path.clone()))?;
                trash::restore(item)?;
                return Ok(Change::Produced {
                    path: path.clone(),
                    product: *product,
                    held: None,
                });
            }
        }
        Ok(self.clone())
    }

    /// Takes the change back and returns it as it now stands: an undone `Produced` sits in the
    /// trash under a handle the record must keep, so it can be redone.
    fn revert(&self, vfs: &dyn Vfs) -> Result<Change, UndoError> {
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
            Change::Produced { path, product, .. } => {
                if !vfs.exists(path) {
                    return Err(UndoError::Missing(path.clone()));
                }
                let before = trash::snapshot()?;
                crate::trash(path)?;
                let item = trash::claim(&before, std::slice::from_ref(path))?
                    .pop()
                    .flatten()
                    .ok_or_else(|| {
                        trash::TrashError::from("the trashed item could not be found again")
                    })?;
                return Ok(Change::Produced {
                    path: path.clone(),
                    product: *product,
                    held: Some(item),
                });
            }
        }
        Ok(self.clone())
    }

    /// The verb for the status line.
    fn verb(&self) -> &'static str {
        match self {
            Change::Created { .. } => "create",
            Change::Moved { from, to } if from.parent() == to.parent() => "rename",
            Change::Moved { .. } => "move",
            Change::Copied { .. } => "copy",
            Change::Trashed { .. } => "trash",
            Change::Produced {
                product: Product::Archive,
                ..
            } => "compress",
            Change::Produced {
                product: Product::Folder,
                ..
            } => "extract",
        }
    }

    fn subject(&self) -> &Path {
        match self {
            Change::Created { path, .. }
            | Change::Trashed { path, .. }
            | Change::Produced { path, .. } => path,
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
    /// Names the entry across sessions, so two histories saved to one file can be told apart.
    id: EntryId,
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
    /// The user stopped it between two steps. The steps before it stay as they are.
    Cancelled { label: String },
}

/// What `History::summary` lists: one label per entry, next-to-run first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub undo: Vec<String>,
    pub redo: Vec<String>,
}

/// The undo and redo stacks.
#[derive(Debug)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// Changes of a batch still running, committed as one entry by `commit`.
    pending: Vec<Change>,
    /// The id the batch in progress will have once committed, so the file names it the same before
    /// and after.
    pending_id: Option<EntryId>,
    capacity: usize,
    /// Counts every change to what the stacks (or the batch in progress) hold, so a caller that
    /// keeps the history on disk can tell whether there is anything new to write.
    revision: u64,
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
            pending_id: None,
            capacity: capacity.max(1),
            revision: 0,
        }
    }

    /// Changes whenever the history does; equal values mean nothing needs writing again.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Adds one finished step to the batch in progress.
    pub fn record(&mut self, change: Change) {
        self.revision += 1;
        self.pending_id.get_or_insert_with(EntryId::fresh);
        self.pending.push(change);
    }

    /// Ends the batch in progress: what was recorded becomes one undoable entry, and anything that
    /// could have been redone is forgotten, since the history has moved on. Does nothing when
    /// nothing was recorded.
    pub fn commit(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        self.revision += 1;
        self.redo.clear();
        self.undo.push(Entry {
            id: self.pending_id.take().unwrap_or_else(EntryId::fresh),
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

    /// What each stack holds, as the words `Report` uses, next-to-run first: the entry `undo`
    /// would take back is the first of `undo`, the one `redo` would do again the first of `redo`.
    /// Ends the batch in progress first, so it is listed as the entry it will be.
    pub fn summary(&mut self) -> Summary {
        self.commit();
        let labels = |stack: &[Entry]| stack.iter().rev().map(|e| label(&e.changes)).collect();
        Summary {
            undo: labels(&self.undo),
            redo: labels(&self.redo),
        }
    }

    /// Takes the newest entry off the undo stack to be run elsewhere (`Batch::run`, on a worker
    /// thread), or `None` when there is nothing to take back. Until `finish` hands the result
    /// back, the entry is on neither stack.
    pub fn begin_undo(&mut self) -> Option<Batch> {
        self.begin_undo_at(0)
    }

    /// Takes the entry `depth` places from the newest (0 is the one `begin_undo` takes) off the
    /// undo stack, leaving the others where they are. Entries recorded after it are not touched,
    /// so a step that depended on them is refused by its own checks and the entry goes back to
    /// the place it came from.
    pub fn begin_undo_at(&mut self, depth: usize) -> Option<Batch> {
        self.commit();
        self.revision += 1;
        Self::take(&mut self.undo, depth, Direction::Undo)
    }

    /// Takes the newest entry that was undone off the redo stack, like `begin_undo`.
    pub fn begin_redo(&mut self) -> Option<Batch> {
        self.begin_redo_at(0)
    }

    /// Takes the entry `depth` places from the next one to redo, like `begin_undo_at`.
    pub fn begin_redo_at(&mut self, depth: usize) -> Option<Batch> {
        self.revision += 1;
        Self::take(&mut self.redo, depth, Direction::Redo)
    }

    fn take(stack: &mut Vec<Entry>, depth: usize, direction: Direction) -> Option<Batch> {
        let index = stack.len().checked_sub(depth.checked_add(1)?)?;
        let Entry { id, changes } = stack.remove(index);
        Some(Batch {
            direction,
            index,
            id,
            changes,
        })
    }

    /// Puts what a `Batch` did back on the stacks: the steps carried out go on the opposite stack
    /// (an undone step can be redone), the steps left stay on the stack they came from, so no
    /// change is lost or duplicated.
    pub fn finish(&mut self, finished: Finished) -> Report {
        let Finished {
            direction,
            index,
            id,
            label,
            done,
            left,
            stop,
        } = finished;
        self.revision += 1;
        let (carried_out, remaining) = match direction {
            Direction::Undo => (&mut self.redo, &mut self.undo),
            Direction::Redo => (&mut self.undo, &mut self.redo),
        };
        if !done.is_empty() {
            carried_out.push(Entry {
                id: EntryId::fresh(),
                changes: done,
            });
        }
        if !left.is_empty() {
            // Back where it was taken from; the stack can only have grown since if it was the top.
            let at = index.min(remaining.len());
            // What is left is the same entry, shorter, so another session holding it still matches.
            remaining.insert(at, Entry { id, changes: left });
        }
        match stop {
            None => Report::Done { label },
            Some(Stop::Failed(error)) => Report::Failed { label, error },
            Some(Stop::Cancelled) => Report::Cancelled { label },
        }
    }

    /// Takes back the newest entry, waiting for it. The interface runs the same three calls
    /// (`begin_undo`, `Batch::run`, `finish`) with the middle one off the render thread.
    pub fn undo(&mut self, vfs: &dyn Vfs) -> Report {
        match self.begin_undo() {
            Some(batch) => {
                let finished = batch.run(vfs, &mut |_| ControlFlow::Continue(()));
                self.finish(finished)
            }
            None => Report::Empty,
        }
    }

    /// Does again the newest entry that was undone, waiting for it.
    pub fn redo(&mut self, vfs: &dyn Vfs) -> Report {
        match self.begin_redo() {
            Some(batch) => {
                let finished = batch.run(vfs, &mut |_| ControlFlow::Continue(()));
                self.finish(finished)
            }
            None => Report::Empty,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Undo,
    Redo,
}

/// An entry taken off a stack to be taken back or done again away from the `History`.
///
/// Model: every `Change` is in exactly one place, the undo stack, the redo stack or one `Batch`.
/// `begin_*` moves an entry from a stack into a `Batch`; `run` splits its changes into those
/// carried out and those left; `finish` puts each part on its stack. Nothing else touches them in
/// between, so a batch cut short by a failure or a cancel keeps the same rule as before: what was
/// carried out can be reversed, and the rest stays where it was.
#[derive(Debug)]
pub struct Batch {
    direction: Direction,
    /// Where in its stack the entry sat, so what is left of it can go back there.
    index: usize,
    id: EntryId,
    changes: Vec<Change>,
}

impl Batch {
    /// Carries out every step in turn: an undo reverts from the newest step to the oldest, a
    /// redo applies from the oldest to the newest. `step` is told which path is about to be
    /// touched and may stop the batch before it by returning `Break`.
    pub fn run(self, vfs: &dyn Vfs, step: &mut ProgressFn<'_>) -> Finished {
        let Batch {
            direction,
            index,
            id,
            changes,
        } = self;
        let label = label(&changes);
        let mut stop = None;
        let (done, left) = match direction {
            Direction::Undo => {
                let mut reverted = Vec::with_capacity(changes.len());
                for change in changes.iter().rev() {
                    if let Some(cancelled) = Self::before(step, change) {
                        stop = Some(cancelled);
                        break;
                    }
                    match change.revert(vfs) {
                        Ok(taken_back) => reverted.push(taken_back),
                        Err(error) => {
                            stop = Some(Stop::Failed(error));
                            break;
                        }
                    }
                }
                let mut left = changes;
                left.truncate(left.len() - reverted.len());
                // Oldest first, as a redo applies them.
                reverted.reverse();
                (reverted, left)
            }
            Direction::Redo => {
                let mut done = Vec::with_capacity(changes.len());
                for change in &changes {
                    if let Some(cancelled) = Self::before(step, change) {
                        stop = Some(cancelled);
                        break;
                    }
                    match change.apply(vfs) {
                        Ok(applied) => done.push(applied),
                        Err(error) => {
                            stop = Some(Stop::Failed(error));
                            break;
                        }
                    }
                }
                let left = changes.into_iter().skip(done.len()).collect();
                (done, left)
            }
        };
        Finished {
            direction,
            index,
            id,
            label,
            done,
            left,
            stop,
        }
    }

    fn before(step: &mut ProgressFn<'_>, change: &Change) -> Option<Stop> {
        step(change.subject()).is_break().then_some(Stop::Cancelled)
    }
}

/// Why a batch ended before its last step.
#[derive(Debug)]
enum Stop {
    Cancelled,
    Failed(UndoError),
}

/// What a `Batch` did, to be handed to `History::finish`.
#[derive(Debug)]
pub struct Finished {
    direction: Direction,
    index: usize,
    id: EntryId,
    label: String,
    /// The steps carried out, in the state they now stand in (a redone trash has a new handle).
    done: Vec<Change>,
    /// The steps not carried out, in their original order, the one that failed first.
    left: Vec<Change>,
    stop: Option<Stop>,
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
    fn one_entry_can_be_undone_without_the_ones_after_it() {
        let dir = scratch("single");
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::write(&a, b"").unwrap();
        std::fs::write(&b, b"").unwrap();
        let mut history = History::default();
        for path in [&a, &b] {
            entry(
                &mut history,
                Change::Created {
                    path: path.clone(),
                    is_dir: false,
                },
            );
        }

        let batch = history.begin_undo_at(1).unwrap();
        let finished = batch.run(&LocalVfs, &mut |_| ControlFlow::Continue(()));
        assert!(matches!(history.finish(finished), Report::Done { .. }));

        assert_eq!(listing(&dir), ["b"]);
        assert_eq!(history.summary().undo, ["create b"]);
        assert_eq!(history.summary().redo, ["create a"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_refused_single_undo_returns_to_its_own_place() {
        let dir = scratch("refused");
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::write(&a, b"precious").unwrap();
        std::fs::write(&b, b"").unwrap();
        let mut history = History::default();
        for path in [&a, &b] {
            entry(
                &mut history,
                Change::Created {
                    path: path.clone(),
                    is_dir: false,
                },
            );
        }

        let batch = history.begin_undo_at(1).unwrap();
        let finished = batch.run(&LocalVfs, &mut |_| ControlFlow::Continue(()));
        assert!(matches!(history.finish(finished), Report::Failed { .. }));

        assert_eq!(history.summary().undo, ["create b", "create a"]);
        assert!(!history.can_redo());
        assert_eq!(std::fs::read(&a).unwrap(), b"precious");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_depth_past_the_stack_takes_nothing() {
        let mut history = History::default();
        assert!(history.begin_undo_at(0).is_none());
        assert!(history.begin_redo_at(usize::MAX).is_none());
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
    fn undoing_a_made_archive_trashes_it_with_its_contents_and_redo_takes_it_back() {
        let dir = scratch("produced");
        let archive = dir.join("a.zip");
        std::fs::write(&archive, b"not empty").unwrap();
        let mut history = History::default();
        entry(
            &mut history,
            Change::Produced {
                path: archive.clone(),
                product: Product::Archive,
                held: None,
            },
        );

        assert!(matches!(history.undo(&LocalVfs), Report::Done { .. }));
        assert!(!archive.exists());
        assert!(matches!(history.redo(&LocalVfs), Report::Done { .. }));
        assert_eq!(std::fs::read(&archive).unwrap(), b"not empty");
        // The record followed the file through the trash both ways, so it undoes again.
        assert!(matches!(history.undo(&LocalVfs), Report::Done { .. }));
        assert!(!archive.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undoing_an_extract_trashes_the_whole_folder() {
        let dir = scratch("unpacked");
        let folder = dir.join("out");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("inside"), b"x").unwrap();
        let mut history = History::default();
        entry(
            &mut history,
            Change::Produced {
                path: folder.clone(),
                product: Product::Folder,
                held: None,
            },
        );

        assert!(matches!(history.undo(&LocalVfs), Report::Done { .. }));
        assert!(!folder.exists());
        assert!(matches!(history.redo(&LocalVfs), Report::Done { .. }));
        assert_eq!(std::fs::read(folder.join("inside")).unwrap(), b"x");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_made_archive_that_is_gone_is_reported_and_stays_on_the_stack() {
        let dir = scratch("produced-gone");
        let archive = dir.join("a.zip");
        let mut history = History::default();
        entry(
            &mut history,
            Change::Produced {
                path: archive,
                product: Product::Archive,
                held: None,
            },
        );

        assert!(matches!(
            history.undo(&LocalVfs),
            Report::Failed {
                error: UndoError::Missing(_),
                ..
            }
        ));
        assert!(history.can_undo());
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
    fn a_cancelled_undo_keeps_the_steps_it_did_and_leaves_the_rest_undoable() {
        let dir = scratch("cancel-undo");
        let mut history = History::default();
        for name in ["a", "b", "c"] {
            std::fs::write(dir.join(name), b"").unwrap();
            history.record(Change::Created {
                path: dir.join(name),
                is_dir: false,
            });
        }
        history.commit();
        let mut steps = 0;

        let batch = history.begin_undo().unwrap();
        assert!(!history.can_undo(), "the entry is out while it runs");
        let finished = batch.run(&LocalVfs, &mut |_| {
            steps += 1;
            if steps > 2 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        let report = history.finish(finished);

        assert!(matches!(report, Report::Cancelled { .. }));
        assert_eq!(listing(&dir), ["a"], "the newest two were taken back");
        assert!(history.can_undo() && history.can_redo());
        assert!(matches!(history.undo(&LocalVfs), Report::Done { .. }));
        assert!(listing(&dir).is_empty());
        assert!(matches!(history.redo(&LocalVfs), Report::Done { .. }));
        assert_eq!(listing(&dir), ["a"]);
        assert!(matches!(history.redo(&LocalVfs), Report::Done { .. }));
        assert_eq!(listing(&dir), ["a", "b", "c"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cancelled_redo_keeps_the_steps_it_did_and_leaves_the_rest_redoable() {
        let dir = scratch("cancel-redo");
        let mut history = History::default();
        for name in ["a", "b"] {
            std::fs::write(dir.join(name), b"").unwrap();
            history.record(Change::Created {
                path: dir.join(name),
                is_dir: false,
            });
        }
        history.commit();
        history.undo(&LocalVfs);
        let mut steps = 0;

        let batch = history.begin_redo().unwrap();
        let finished = batch.run(&LocalVfs, &mut |_| {
            steps += 1;
            if steps > 1 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        let report = history.finish(finished);

        assert!(matches!(report, Report::Cancelled { .. }));
        assert_eq!(listing(&dir), ["a"]);
        assert!(history.can_undo() && history.can_redo());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_step_callback_names_each_path_before_it_is_touched() {
        let dir = scratch("callback");
        let mut history = History::default();
        std::fs::write(dir.join("a"), b"").unwrap();
        entry(
            &mut history,
            Change::Created {
                path: dir.join("a"),
                is_dir: false,
            },
        );
        let mut seen = Vec::new();

        let finished = history.begin_undo().unwrap().run(&LocalVfs, &mut |path| {
            seen.push((path.to_path_buf(), path.exists()));
            ControlFlow::Continue(())
        });
        history.finish(finished);

        assert_eq!(seen, [(dir.join("a"), true)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_summary_lists_each_stack_next_to_run_first() {
        let dir = scratch("summary");
        let mut history = History::default();
        for name in ["a", "b", "c"] {
            let path = dir.join(name);
            std::fs::write(&path, b"").unwrap();
            entry(
                &mut history,
                Change::Created {
                    path,
                    is_dir: false,
                },
            );
        }
        assert!(matches!(history.undo(&LocalVfs), Report::Done { .. }));

        let summary = history.summary();

        assert_eq!(summary.undo, ["create b", "create a"]);
        assert_eq!(summary.redo, ["create c"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_summary_lists_a_batch_still_being_recorded_as_one_entry() {
        let mut history = History::default();
        history.record(Change::Created {
            path: PathBuf::from("/x/a"),
            is_dir: false,
        });
        history.record(Change::Created {
            path: PathBuf::from("/x/b"),
            is_dir: false,
        });

        assert_eq!(history.summary().undo, ["create 2 items"]);
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
