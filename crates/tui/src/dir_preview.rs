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

//! Lists the folder under the cursor for the preview column on the blocking pool, so a folder on
//! another machine or inside an archive never holds up a frame. The frame used to list it itself,
//! once per frame, which for those folders meant a round trip to a plugin or a remote host every
//! time anything redrew.
//!
//! A folder is listed when the cursor reaches it and again every [`LOCAL_INTERVAL`] (or
//! [`SLOW_INTERVAL`] for a slow one), which keeps the column as fresh as the per-frame read did
//! for a folder that changes underneath it. The first listing of a folder gets a short grace
//! period to land inside the same frame, so a local folder never shows up empty for a moment.
//! A read of a folder the cursor has left is given up (`shared::cancel`), so a slow one does not
//! hold back the folder the cursor is on now.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use shared::{Cancel, DirEntryInfo, Vfs};

/// How long a local folder's listing is trusted before it is read again.
const LOCAL_INTERVAL: Duration = Duration::from_millis(500);
/// The same for a folder on another machine or in an archive, where each read costs a round trip.
const SLOW_INTERVAL: Duration = Duration::from_secs(5);
/// How long a first listing may hold the frame back. A local folder answers well inside it; a
/// slow one outlasts it and is shown when it lands.
const GRACE: Duration = Duration::from_millis(20);

/// What one background read produced. An unreadable folder is an empty one, as the column has
/// always treated it.
struct Listed {
    path: PathBuf,
    entries: Vec<DirEntryInfo>,
}

/// A read that is running.
struct Reading {
    path: PathBuf,
    cancel: Cancel,
}

pub struct DirPreview {
    /// `None` for a read that was given up, which says nothing about the folder.
    tx: Sender<Option<Listed>>,
    rx: Receiver<Option<Listed>>,
    handle: tokio::runtime::Handle,
    vfs: Arc<dyn Vfs>,
    /// The latest listing that arrived and the folder it belongs to. Unfiltered, as `Vfs`
    /// returns it; the browser arranges it for the column.
    shown: Option<Listed>,
    /// The read that is running, if any; a slow one must not pile up requests.
    in_flight: Option<Reading>,
    last_started: Option<Instant>,
}

impl DirPreview {
    pub fn new(handle: tokio::runtime::Handle, vfs: Arc<dyn Vfs>) -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            handle,
            vfs,
            shown: None,
            in_flight: None,
            last_started: None,
        }
    }

    /// Call once per render tick with the folder under the cursor (`None` when the cursor is on
    /// a file or nothing). Takes in a finished read and starts the next when the folder changed
    /// or its listing has gone stale.
    pub fn poll(&mut self, folder: Option<&Path>, now: Instant) {
        self.take_finished();
        if let Some(reading) = &self.in_flight
            && folder != Some(reading.path.as_path())
        {
            reading.cancel.cancel();
        }
        let Some(folder) = folder else {
            return;
        };
        if self.in_flight.is_some() {
            return;
        }
        let first = self.shown.as_ref().is_none_or(|shown| shown.path != folder);
        let interval = if shared::is_slow(folder) {
            SLOW_INTERVAL
        } else {
            LOCAL_INTERVAL
        };
        let stale = self
            .last_started
            .is_none_or(|started| now.duration_since(started) >= interval);
        if !(first || stale) {
            return;
        }
        let cancel = Cancel::new();
        self.in_flight = Some(Reading {
            path: folder.to_path_buf(),
            cancel: cancel.clone(),
        });
        self.last_started = Some(now);
        self.spawn(folder.to_path_buf(), cancel);
        if first {
            self.wait_for_it();
        }
    }

    /// The listing of `folder`, unfiltered, once it has been read. `None` while the first read
    /// is still running, so the column shows nothing rather than another folder's children.
    pub fn entries(&self, folder: &Path) -> Option<&[DirEntryInfo]> {
        self.shown
            .as_ref()
            .filter(|shown| shown.path == folder)
            .map(|shown| shown.entries.as_slice())
    }

    /// Forgets the listing so the next tick reads again at once, for after the folder was changed
    /// by something this program did.
    pub fn invalidate(&mut self) {
        self.last_started = None;
    }

    fn take_finished(&mut self) {
        while let Ok(listed) = self.rx.try_recv() {
            self.land(listed);
        }
    }

    /// Gives the read just started a moment to finish, so a quick one is on screen this frame.
    fn wait_for_it(&mut self) {
        if let Ok(listed) = self.rx.recv_timeout(GRACE) {
            self.land(listed);
        }
    }

    fn land(&mut self, listed: Option<Listed>) {
        self.in_flight = None;
        match listed {
            Some(listed) => self.shown = Some(listed),
            // Read again at once if the cursor comes back.
            None => self.last_started = None,
        }
    }

    fn spawn(&self, path: PathBuf, cancel: Cancel) {
        let tx = self.tx.clone();
        let vfs = Arc::clone(&self.vfs);
        self.handle.spawn_blocking(move || {
            let _scope = cancel.enter();
            let entries = vfs.list_dir(&path).unwrap_or_default();
            // A read given up may have failed for that alone; it must not pass for an empty folder.
            let listed = (!cancel.is_cancelled()).then_some(Listed { path, entries });
            // Nobody listening means the program is exiting.
            let _ = tx.send(listed);
        });
    }
}

impl Drop for DirPreview {
    fn drop(&mut self) {
        if let Some(reading) = &self.in_flight {
            reading.cancel.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use shared::LocalVfs;

    use super::*;
    use crate::test_vfs::Slow;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-dir-preview-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        std::fs::write(dir.join("inner").join("a.txt"), "a").unwrap();
        dir
    }

    fn names(entries: &[DirEntryInfo]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn wait_for(preview: &mut DirPreview, folder: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while preview.entries(folder).is_none() && Instant::now() < deadline {
            preview.poll(Some(folder), Instant::now());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_quick_folder_is_ready_in_the_same_tick() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("quick");
        let mut preview = DirPreview::new(runtime.handle().clone(), Arc::new(LocalVfs));
        preview.poll(Some(&root.join("inner")), Instant::now());
        let listed = preview
            .entries(&root.join("inner"))
            .expect("listed in grace");
        assert_eq!(names(listed), vec!["a.txt"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_slow_folder_does_not_hold_the_tick_back_and_shows_nothing_until_it_lands() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("slow");
        let vfs = Arc::new(Slow::new(Duration::from_millis(300)));
        let mut preview = DirPreview::new(runtime.handle().clone(), vfs);
        let started = Instant::now();
        preview.poll(Some(&root.join("inner")), started);
        assert!(
            started.elapsed() < Duration::from_millis(150),
            "tick waited"
        );
        assert!(preview.entries(&root.join("inner")).is_none());

        wait_for(&mut preview, &root.join("inner"));
        assert_eq!(
            names(preview.entries(&root.join("inner")).unwrap()),
            vec!["a.txt"]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn another_folders_listing_is_never_shown_for_this_one() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("other");
        let mut preview = DirPreview::new(runtime.handle().clone(), Arc::new(LocalVfs));
        preview.poll(Some(&root.join("inner")), Instant::now());
        assert!(preview.entries(&root.join("inner")).is_some());
        assert!(preview.entries(&root).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_listing_is_not_read_again_before_its_interval() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("interval");
        let vfs = Arc::new(Slow::new(Duration::ZERO));
        let mut preview = DirPreview::new(runtime.handle().clone(), vfs.clone());
        let folder = root.join("inner");
        let t0 = Instant::now();
        preview.poll(Some(&folder), t0);
        for step in 1..20 {
            preview.poll(Some(&folder), t0 + Duration::from_millis(step * 10));
        }
        assert_eq!(vfs.lists.load(Ordering::SeqCst), 1);

        preview.poll(Some(&folder), t0 + LOCAL_INTERVAL);
        wait_for_count(&vfs, 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn wait_for_count(vfs: &Slow, wanted: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while vfs.lists.load(Ordering::SeqCst) < wanted && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(vfs.lists.load(Ordering::SeqCst), wanted);
    }

    #[test]
    fn invalidate_reads_again_on_the_next_tick() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("invalidate");
        let vfs = Arc::new(Slow::new(Duration::ZERO));
        let mut preview = DirPreview::new(runtime.handle().clone(), vfs.clone());
        let folder = root.join("inner");
        let t0 = Instant::now();
        preview.poll(Some(&folder), t0);
        preview.invalidate();
        preview.poll(Some(&folder), t0 + Duration::from_millis(1));
        wait_for_count(&vfs, 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn leaving_a_folder_gives_up_its_read_and_the_next_one_starts() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("leave");
        let vfs = Arc::new(Slow::new(Duration::from_secs(60)));
        let mut preview = DirPreview::new(runtime.handle().clone(), vfs.clone());
        let folder = root.join("inner");
        preview.poll(Some(&folder), Instant::now());
        assert!(Slow::reaches(&vfs.lists, 1));

        preview.poll(None, Instant::now());
        assert!(Slow::reaches(&vfs.gave_up, 1), "the read was given up");
        // The given-up read does not pass for an empty folder, and coming back reads it again.
        let deadline = Instant::now() + Duration::from_secs(5);
        while vfs.lists.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
            preview.poll(Some(&folder), Instant::now());
            assert!(preview.entries(&folder).is_none());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(vfs.lists.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreadable_folder_is_an_empty_one() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut preview = DirPreview::new(runtime.handle().clone(), Arc::new(LocalVfs));
        let missing = std::env::temp_dir().join("minuteman-dir-preview-missing-xyz");
        preview.poll(Some(&missing), Instant::now());
        wait_for(&mut preview, &missing);
        assert_eq!(preview.entries(&missing).map(<[_]>::len), Some(0));
    }
}
