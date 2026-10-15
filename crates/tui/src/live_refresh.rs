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

//! Keeps the file lists in step with the disk while the app sits idle, so a file made by a
//! mini-shell, a `:` command or another program appears without the user having to navigate
//! away and back.
//!
//! The standard library has no way to be told about filesystem changes, so this polls: a few
//! times a second it re-lists the browsed directory (and its parent) on the blocking pool and
//! lets `BrowserState::apply_listing` compare the result with what is on screen. Comparing whole
//! listings — names, sizes, modified times, permissions — rather than a directory's own mtime is
//! what catches a file being written to in place, which never touches its directory's mtime.
//! Going through `Vfs::list_dir` also means any future backend gets this for free.
//!
//! A listing of a folder the browser has since left is given up (`shared::cancel`): on another
//! machine it may be waiting seconds for an answer nobody will look at.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use browser::BrowserState;
use shared::{Cancel, DirEntryInfo, Vfs};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Time between the start of one background listing and the next. Long enough that the extra
/// `readdir` + per-entry `stat` is negligible even in a large directory, short enough that a
/// change shows up before the user has looked away.
const INTERVAL: Duration = Duration::from_millis(500);
/// A folder on another machine is re-read far less often: each look costs several round trips.
const REMOTE_INTERVAL: Duration = Duration::from_secs(5);

/// One background listing: the directory it was taken from, and its parent. `None` when the
/// directory could not be read (e.g. it was just deleted) — the screen keeps what it has.
type Snapshot = Option<(PathBuf, Vec<DirEntryInfo>, Vec<DirEntryInfo>)>;

pub struct LiveRefresh {
    tx: UnboundedSender<Snapshot>,
    rx: UnboundedReceiver<Snapshot>,
    handle: tokio::runtime::Handle,
    vfs: Arc<dyn Vfs>,
    /// The listing already running, and the folder it reads; a slow filesystem must not pile up
    /// requests behind it.
    in_flight: Option<(PathBuf, Cancel)>,
    /// `None` until the first listing, which is started immediately.
    last_started: Option<Instant>,
    /// Something this program did changed the folder: list again as soon as no listing is
    /// running, without waiting for the interval.
    dirty: bool,
}

impl LiveRefresh {
    pub fn new(handle: tokio::runtime::Handle, vfs: Arc<dyn Vfs>) -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            tx,
            rx,
            handle,
            vfs,
            in_flight: None,
            last_started: None,
            dirty: false,
        }
    }

    /// Asks for a fresh listing at the next [`poll`](Self::poll). For a change made by this
    /// program in a folder that is slow to read, where re-listing on the spot would hold the
    /// interface up; the cursor keeps its place by path when the listing lands.
    pub fn kick(&mut self) {
        self.dirty = true;
    }

    /// Call once per render tick: applies any finished listing to `browser`, and starts the next
    /// one when `INTERVAL` has passed. Returns whether the selected file's own content changed on
    /// disk (its size or modified time moved while it stayed selected) — the cue to re-read its
    /// preview, which is otherwise keyed on the path alone.
    pub fn poll(&mut self, browser: &mut BrowserState, now: Instant) -> bool {
        let mut selected_changed = false;

        while let Ok(snapshot) = self.rx.try_recv() {
            self.in_flight = None;
            let Some((dir, current, parent)) = snapshot else {
                continue;
            };
            let before = browser.selected_entry().cloned();
            if browser.apply_listing(&dir, current, parent) {
                // A command may have removed a path that was marked in some other directory.
                browser.prune_marks(self.vfs.as_ref());
                selected_changed |= content_changed(before.as_ref(), browser.selected_entry());
            }
        }

        if let Some((dir, cancel)) = &self.in_flight
            && dir != browser.current_dir()
        {
            cancel.cancel();
        }

        let interval = if shared::ssh_path::is_remote(browser.current_dir()) {
            REMOTE_INTERVAL
        } else {
            INTERVAL
        };
        let due = self.dirty
            || self
                .last_started
                .is_none_or(|started| now.duration_since(started) >= interval);
        if due && self.in_flight.is_none() {
            let (dir, cancel) = (browser.current_dir().to_path_buf(), Cancel::new());
            self.in_flight = Some((dir.clone(), cancel.clone()));
            self.dirty = false;
            self.last_started = Some(now);
            self.spawn_listing(dir, cancel);
        }

        selected_changed
    }

    fn spawn_listing(&self, dir: PathBuf, cancel: Cancel) {
        let tx = self.tx.clone();
        let vfs = Arc::clone(&self.vfs);
        self.handle.spawn_blocking(move || {
            let _scope = cancel.enter();
            let snapshot = vfs.list_dir(&dir).ok().map(|current| {
                // Mirrors `BrowserState::refresh`: an unreadable parent is an empty column.
                let parent = dir
                    .parent()
                    .and_then(|parent| vfs.list_dir(parent).ok())
                    .unwrap_or_default();
                (dir, current, parent)
            });
            // A listing given up may be missing its parent for that alone.
            let _ = tx.send(snapshot.filter(|_| !cancel.is_cancelled()));
        });
    }
}

impl Drop for LiveRefresh {
    fn drop(&mut self) {
        if let Some((_, cancel)) = &self.in_flight {
            cancel.cancel();
        }
    }
}

/// Whether the same file is still selected but was written to: a different path is the
/// preview's own business, since it already re-reads whenever the selected path changes.
fn content_changed(before: Option<&DirEntryInfo>, after: Option<&DirEntryInfo>) -> bool {
    match (before, after) {
        (Some(before), Some(after)) => {
            before.path == after.path
                && (before.size != after.size || before.modified != after.modified)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use shared::LocalVfs;

    use super::*;

    fn entry(path: &str, size: u64, modified_secs: u64) -> DirEntryInfo {
        DirEntryInfo {
            name: path.into(),
            path: PathBuf::from(path),
            is_dir: false,
            size,
            modified: Some(std::time::UNIX_EPOCH + Duration::from_secs(modified_secs)),
            mode: None,
        }
    }

    #[test]
    fn a_selected_file_that_grew_or_was_rewritten_counts_as_changed() {
        let before = entry("a.txt", 1, 10);
        assert!(content_changed(Some(&before), Some(&entry("a.txt", 2, 10))));
        assert!(content_changed(Some(&before), Some(&entry("a.txt", 1, 11))));
    }

    #[test]
    fn an_untouched_file_or_a_different_selection_does_not() {
        let before = entry("a.txt", 1, 10);
        assert!(!content_changed(
            Some(&before),
            Some(&entry("a.txt", 1, 10))
        ));
        assert!(!content_changed(
            Some(&before),
            Some(&entry("b.txt", 9, 99))
        ));
        assert!(!content_changed(None, Some(&before)));
        assert!(!content_changed(Some(&before), None));
    }

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-live-refresh-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("work")).unwrap();
        dir
    }

    /// Polls until `done` holds or two seconds pass, returning whether it held.
    fn poll_until(
        live: &mut LiveRefresh,
        browser: &mut BrowserState,
        mut done: impl FnMut(&BrowserState, bool) -> bool,
    ) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let selected_changed = live.poll(browser, Instant::now());
            if done(browser, selected_changed) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_listing_of_a_folder_the_browser_left_is_given_up() {
        use crate::test_vfs::Slow;

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch_dir("left");
        let work = root.join("work");
        let mut browser = BrowserState::new(&LocalVfs, work.clone()).unwrap();
        let vfs = Arc::new(Slow::new(Duration::from_secs(60)));
        let mut live = LiveRefresh::new(runtime.handle().clone(), vfs.clone());
        live.poll(&mut browser, Instant::now());
        assert!(Slow::reaches(&vfs.lists, 1));

        let moved = browser.plan_goto(&root);
        let loaded = moved.load(&LocalVfs).unwrap();
        assert!(browser.apply_move(loaded));
        live.poll(&mut browser, Instant::now());
        assert!(
            Slow::reaches(&vfs.gave_up, 1),
            "the old listing was given up"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_created_behind_the_browsers_back_shows_up() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch_dir("create");
        let work = root.join("work");
        let mut browser = BrowserState::new(&LocalVfs, work.clone()).unwrap();
        let mut live = LiveRefresh::new(runtime.handle().clone(), Arc::new(LocalVfs));

        std::fs::write(work.join("made-by-a-shell.txt"), b"hi").unwrap();

        assert!(poll_until(&mut live, &mut browser, |b, _| {
            b.current_entries()
                .iter()
                .any(|e| e.name == "made-by-a-shell.txt")
        }));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn writing_into_the_selected_file_updates_its_size_and_flags_the_preview() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch_dir("write");
        let work = root.join("work");
        std::fs::write(work.join("note.txt"), b"a").unwrap();
        let mut browser = BrowserState::new(&LocalVfs, work.clone()).unwrap();
        let mut live = LiveRefresh::new(runtime.handle().clone(), Arc::new(LocalVfs));

        std::fs::write(work.join("note.txt"), b"a much longer body").unwrap();

        let mut flagged = false;
        assert!(poll_until(
            &mut live,
            &mut browser,
            |b, selected_changed| {
                flagged |= selected_changed;
                b.selected_entry().is_some_and(|e| e.size == 18)
            }
        ));
        assert!(
            flagged,
            "the selected file's preview should have been told to re-read"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_kick_lists_again_without_waiting_for_the_interval() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch_dir("kick");
        let work = root.join("work");
        let mut browser = BrowserState::new(&LocalVfs, work.clone()).unwrap();
        let mut live = LiveRefresh::new(runtime.handle().clone(), Arc::new(LocalVfs));
        // Let the first listing land, so the next one is not due for a whole interval.
        let first = Instant::now();
        live.poll(&mut browser, first);
        while live.in_flight.is_some() && first.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
            live.poll(&mut browser, first);
        }
        assert!(live.in_flight.is_none(), "the first listing never landed");

        std::fs::write(work.join("made-by-us.txt"), b"hi").unwrap();
        live.kick();
        let kicked = Instant::now();
        assert!(poll_until(&mut live, &mut browser, |b, _| {
            b.current_entries()
                .iter()
                .any(|e| e.name == "made-by-us.txt")
        }));
        assert!(
            kicked.elapsed() < INTERVAL,
            "the listing waited for the interval"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_kick_during_a_listing_is_not_lost() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch_dir("kick-busy");
        let work = root.join("work");
        let mut browser = BrowserState::new(&LocalVfs, work.clone()).unwrap();
        let mut live = LiveRefresh::new(runtime.handle().clone(), Arc::new(LocalVfs));
        live.poll(&mut browser, Instant::now());
        assert!(live.in_flight.is_some());

        live.kick();
        assert!(live.dirty, "kept for after the running listing");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_deleted_behind_the_browsers_back_disappears() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch_dir("delete");
        let work = root.join("work");
        std::fs::write(work.join("doomed.txt"), b"hi").unwrap();
        let mut browser = BrowserState::new(&LocalVfs, work.clone()).unwrap();
        let mut live = LiveRefresh::new(runtime.handle().clone(), Arc::new(LocalVfs));
        assert_eq!(browser.current_entries().len(), 1);

        std::fs::remove_file(work.join("doomed.txt")).unwrap();

        assert!(poll_until(&mut live, &mut browser, |b, _| b
            .current_entries()
            .is_empty()));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
