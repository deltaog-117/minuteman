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

//! Keeps the undo history in a file, so `u` and `:history` still work after a restart.
//!
//! The text itself is `file_ops::history`'s (`History::encode` and `decode`); this is only where
//! it lives and how it gets there. Writing happens on one thread of its own, so a save never holds
//! up the render loop, and a burst of changes costs one write: the thread writes the newest text
//! it was handed and skips the ones a newer text has already overtaken. Each write goes to a
//! temporary file that is then renamed over the real one, so a crash leaves the old history or the
//! new one, never half of either. Two `mman` running at once save into one file (see `Merger`), and a
//! session takes in what the other saved when asked (`adopt_into`), not before. To know when
//! asking is worthwhile, the file's folder is watched like a browsed one (`take_changed`), or the
//! file's modified time compared when the folder cannot be watched.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::SystemTime;

use file_ops::history::{DecodeError, EntryId, History, Merger};
use shared::{LocalVfs, Vfs, Wake, WatchGuard};

/// The file the history is kept in: `$XDG_STATE_HOME/minuteman/history`, or
/// `~/.local/state/minuteman/history` when that is unset. `None` when neither variable names an
/// absolute folder.
pub fn state_file() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let base = var("XDG_STATE_HOME")
        .or_else(|| var("HOME").map(|home| home.join(".local").join("state")))?;
    Some(base.join("minuteman").join("history"))
}

/// What reading the saved history gave.
pub struct Loaded {
    pub history: History,
    /// What to tell the user, when something was left out or the file could not be read; `None`
    /// when it was read whole, or when there was no file yet.
    pub notice: Option<String>,
    /// What the file holds that `history` leaves out, for `HistoryFile::start`.
    pub forgotten: Vec<EntryId>,
    /// `false` when the file was saved by a newer `mman`: writing this session's history over it
    /// would destroy what that version kept.
    pub writable: bool,
}

/// Reads the history saved at `path`. Anything that goes wrong starts an empty history and says
/// so in `notice`: not being able to undo what was done last week must never stop `mman` starting.
pub fn load(path: &Path, capacity: usize) -> Loaded {
    let empty = |notice: Option<String>| Loaded {
        history: History::with_capacity(capacity),
        notice,
        forgotten: Vec::new(),
        writable: true,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return empty(None),
        Err(e) => return empty(Some(format!("could not read the undo history: {e}"))),
    };
    // Listed once for the whole file; a trash that cannot be read leaves every entry that needs
    // an item in it out, and `dropped` below says so.
    let live = trash::entries().unwrap_or_default();
    match History::decode(&text, capacity, &live) {
        Ok(restored) => Loaded {
            forgotten: restored.forgotten,
            writable: true,
            history: restored.history,
            notice: (restored.dropped > 0).then(|| {
                format!(
                    "{} undo history entries could not be brought back (their trash items are gone, or the lines were damaged)",
                    restored.dropped
                )
            }),
        },
        Err(e @ DecodeError::Version(_)) => Loaded {
            writable: false,
            ..empty(Some(format!(
                "starting with an empty undo history, and leaving the file as it is: {e}"
            )))
        },
        Err(e) => empty(Some(format!("starting with an empty undo history: {e}"))),
    }
}

/// The writer thread and the way to it.
pub struct HistoryFile {
    path: PathBuf,
    /// Shared with the writer thread; it is locked only to merge or to ask what was let go, never
    /// across a read or a write of the file.
    merger: Arc<Mutex<Merger>>,
    /// `None` only while dropping, which closes it to tell the thread to finish.
    texts: Option<Sender<String>>,
    failures: Receiver<String>,
    writer: Option<JoinHandle<()>>,
    /// Set by the watch on the file's folder from its own thread; any write there sets it,
    /// including this session's own saves, which `take_changed` callers answer with an `adopt_into`
    /// that finds nothing new.
    woken: Arc<AtomicBool>,
    /// `None` when the folder could not be watched (it does not exist before the first save, or
    /// the user's watch limit is spent), which `take_changed` answers by comparing `seen`.
    watch: Option<WatchGuard>,
    /// The file's modified time at the last `take_changed`, for when there is no watch.
    seen: Option<SystemTime>,
}

impl HistoryFile {
    /// `merger` carries what this session knows of the file; see `Merger`.
    pub fn start(path: PathBuf, merger: Merger) -> Self {
        let merger = Arc::new(Mutex::new(merger));
        let (texts, queue) = mpsc::channel::<String>();
        let (failed, failures) = mpsc::channel();
        let writer = std::thread::spawn({
            let (path, merger) = (path.clone(), Arc::clone(&merger));
            move || {
                // Set once the file turns out to be in a format this version must not overwrite; it
                // is said once and every later text is let go.
                let mut frozen = false;
                while let Ok(mut text) = queue.recv() {
                    while let Ok(newer) = queue.try_recv() {
                        text = newer;
                    }
                    if frozen {
                        continue;
                    }
                    match save_merged(&path, &merger, &text) {
                        Ok(()) => {}
                        Err(SaveError::Newer(reason)) => {
                            frozen = true;
                            let _ = failed.send(reason);
                        }
                        // The receiver is gone only when the app is already shutting down.
                        Err(SaveError::Io(e)) => {
                            let _ = failed.send(e.to_string());
                        }
                    }
                }
            }
        });
        let woken = Arc::new(AtomicBool::new(false));
        let watch = path.parent().and_then(|folder| {
            let wake: Wake = {
                let woken = Arc::clone(&woken);
                Arc::new(move || woken.store(true, Ordering::Release))
            };
            LocalVfs.watch(folder, wake)
        });
        Self {
            path,
            merger,
            texts: Some(texts),
            failures,
            writer: Some(writer),
            woken,
            watch,
            seen: None,
        }
    }

    /// Whether the file may have changed since the last call, so that taking it in could find
    /// something. A false alarm costs one read of a small file; a missed change would leave the
    /// popup stale, so when in doubt this says yes.
    pub fn take_changed(&mut self) -> bool {
        if self.watch.is_some() {
            return self.woken.swap(false, Ordering::AcqRel);
        }
        let modified = std::fs::metadata(&self.path)
            .and_then(|meta| meta.modified())
            .ok();
        std::mem::replace(&mut self.seen, modified) != modified
    }

    /// Hands `text` to the writer. Returns at once.
    pub fn save(&self, text: String) {
        if let Some(texts) = &self.texts {
            // A send fails only if the writer thread died, which would already have panicked
            // loudly; there is nowhere left to save to.
            let _ = texts.send(text);
        }
    }

    /// Takes into `history` what other sessions saved that it does not hold, and returns how many
    /// entries that was. Run on the thread that owns `history`, when the user is about to look at
    /// it or use it: never while an undo or redo is running, whose entry is on neither stack.
    ///
    /// # Errors
    ///
    /// The file could not be read, or is not a history this version reads.
    pub fn adopt_into(&self, history: &mut History) -> Result<usize, String> {
        let text = match std::fs::read(&self.path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.to_string()),
        };
        let skip = merger_guard(&self.merger).forgotten_by(&history.ids());
        history
            .adopt(&text, &skip, || trash::entries().unwrap_or_default())
            .map_err(|e| e.to_string())
    }

    /// The reason the latest write failed, once, if one did since the last call.
    pub fn take_failure(&self) -> Option<String> {
        self.failures.try_iter().last()
    }
}

impl Drop for HistoryFile {
    /// Waits for the last text handed over to be written, so quitting right after an operation
    /// keeps it.
    fn drop(&mut self) {
        drop(self.texts.take());
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

enum SaveError {
    /// The file on disk was saved by a newer version and is left alone.
    Newer(String),
    Io(io::Error),
}

impl From<io::Error> for SaveError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Writes `ours` into the file together with what other sessions saved there. The lock is held
/// from the read to the rename: without it two sessions could each read the old file and the
/// later rename would drop the other's entries.
fn save_merged(path: &Path, merger: &Mutex<Merger>, ours: &str) -> Result<(), SaveError> {
    if let Some(folder) = path.parent() {
        create_private_folder(folder)?;
    }
    let lock = open_lock(path)?;
    lock.lock()?;
    let theirs = match std::fs::read(path) {
        Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let text = {
        let mut merger = merger_guard(merger);
        match merger.merge(ours, theirs.as_deref()) {
            Ok(text) => text,
            Err(e @ DecodeError::Version(_)) => {
                return Err(SaveError::Newer(format!(
                    "the file was saved by a newer mman, so this session's history is not saved: {e}"
                )));
            }
            // Not a history at all: a lone session has always overwritten that.
            Err(DecodeError::NotHistory) => merger
                .merge(ours, None)
                .map_err(|e| io::Error::other(e.to_string()))?,
        }
    };
    // `lock` is released when it goes out of scope, after the rename.
    Ok(write_atomically(path, &text)?)
}

/// The merger holds only sets of ids, so what a panicking holder left is still consistent.
fn merger_guard(merger: &Mutex<Merger>) -> MutexGuard<'_, Merger> {
    merger.lock().unwrap_or_else(PoisonError::into_inner)
}

fn open_lock(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path.with_extension("lock"))
}

/// The history names where the user's files are, so the file is for the user alone.
fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        create_private_folder(folder)?;
    }
    // The process id keeps two `mman` from writing into the same temporary file.
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = write_private(&temporary, text).and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn create_private_folder(folder: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)
}

#[cfg(not(unix))]
fn create_private_folder(folder: &Path) -> io::Result<()> {
    std::fs::create_dir_all(folder)
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(text.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    std::fs::File::create(path)?.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(path: PathBuf) -> HistoryFile {
        HistoryFile::start(path, Merger::new(100, []))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-history-file-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_missing_file_is_an_empty_history_without_a_notice() {
        let loaded = load(&scratch("missing").join("history"), 100);
        assert!(!loaded.history.can_undo());
        assert_eq!(loaded.notice, None);
    }

    #[test]
    fn a_file_that_is_not_a_history_starts_empty_and_says_so() {
        let dir = scratch("foreign");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("history");
        std::fs::write(&file, "not a history\n").unwrap();

        let loaded = load(&file, 100);

        assert!(!loaded.history.can_undo());
        assert!(loaded.notice.unwrap().contains("empty undo history"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_was_saved_is_there_after_the_writer_is_dropped() {
        let dir = scratch("saved");
        let file = dir.join("deeper").join("history");
        let writer = start(file.clone());
        let mut history = History::default();
        history.record(file_ops::history::Change::Moved {
            from: "/h/a".into(),
            to: "/h/b".into(),
        });
        history.commit();
        for _ in 0..50 {
            writer.save(history.encode());
        }
        drop(writer);

        let loaded = load(&file, 100);

        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.history.encode(), history.encode());
        let mut left: Vec<_> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["history", "history.lock"],
            "no temporary file is left behind"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_trashed_file_is_restored_by_a_history_saved_and_loaded_again() {
        use file_ops::history::{Report, TrashTracker};
        let dir = scratch("trashed");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gone.txt");
        std::fs::write(&file, "kept").unwrap();
        let tracker = TrashTracker::start();
        file_ops::trash(&file).unwrap();
        let (changes, untracked) = tracker.finish(std::slice::from_ref(&file));
        assert_eq!(untracked, 0);
        let mut history = History::default();
        changes
            .into_iter()
            .for_each(|change| history.record(change));
        history.commit();
        let saved = dir.join("state").join("history");
        let writer = start(saved.clone());
        writer.save(history.encode());
        drop(writer);

        let mut loaded = load(&saved, 100);
        let report = loaded.history.undo(&shared::LocalVfs);

        assert_eq!(loaded.notice, None);
        assert!(matches!(report, Report::Done { .. }), "{report:?}");
        assert_eq!(std::fs::read(&file).unwrap(), b"kept");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_its_folder_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("private");
        let file = dir.join("state").join("history");
        drop({
            let writer = start(file.clone());
            writer.save(History::default().encode());
            writer
        });

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(file.parent().unwrap()), 0o700);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn committed(path: &str) -> History {
        let mut history = History::default();
        history.record(file_ops::history::Change::Created {
            path: path.into(),
            is_dir: false,
        });
        history.commit();
        history
    }

    #[test]
    fn a_file_in_a_newer_format_is_not_loaded_nor_written() {
        let dir = scratch("newer");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("history");
        std::fs::write(&file, "mman-history 9\nwhat a later version keeps\n").unwrap();

        let loaded = load(&file, 100);
        assert!(!loaded.writable);
        assert!(loaded.notice.unwrap().contains("format 9"));

        // A session that started before the newer one wrote must not overwrite it either.
        let writer = start(file.clone());
        writer.save(committed("/h/a").encode());
        let give_up = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while writer.take_failure().is_none() {
            assert!(std::time::Instant::now() < give_up, "nothing was reported");
            std::thread::yield_now();
        }
        drop(writer);

        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "mman-history 9\nwhat a later version keeps\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sessions_saving_at_once_keep_each_others_entries() {
        let dir = scratch("sessions");
        let file = dir.join("history");
        let sessions: Vec<_> = (0..4)
            .map(|n| {
                let file = file.clone();
                std::thread::spawn(move || {
                    let writer = start(file);
                    let mut history = History::default();
                    for i in 0..5 {
                        history.record(file_ops::history::Change::Created {
                            path: format!("/h/s{n}-{i}").into(),
                            is_dir: false,
                        });
                        history.commit();
                        writer.save(history.encode());
                    }
                })
            })
            .collect();
        sessions
            .into_iter()
            .for_each(|s| s.join().expect("a session panicked"));

        let mut loaded = load(&file, 100);

        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.history.summary().undo.len(), 20);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_write_that_cannot_happen_is_reported_once() {
        let dir = scratch("blocked");
        std::fs::create_dir_all(&dir).unwrap();
        // A plain file where the folder should be.
        let file = dir.join("in-the-way").join("history");
        std::fs::write(dir.join("in-the-way"), "").unwrap();
        let writer = start(file);
        writer.save("x".into());

        let give_up = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let failure = loop {
            if let Some(failure) = writer.take_failure() {
                break failure;
            }
            assert!(
                std::time::Instant::now() < give_up,
                "no failure was reported"
            );
            std::thread::yield_now();
        };

        assert!(!failure.is_empty());
        assert_eq!(writer.take_failure(), None);
        drop(writer);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_session_takes_in_what_another_saved_while_it_ran() {
        let dir = scratch("adopt");
        let file = dir.join("history");
        let (one, two) = (start(file.clone()), start(file.clone()));
        let mut mine = committed("/h/mine");
        one.save(mine.encode());
        two.save(committed("/h/theirs").encode());
        // The writers run on their own threads: wait until both entries are in the file.
        for _ in 0..200 {
            if std::fs::read_to_string(&file).is_ok_and(|t| t.contains("/h/theirs")) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(one.adopt_into(&mut mine), Ok(1));
        assert_eq!(one.adopt_into(&mut mine), Ok(0), "taken once");
        assert!(mine.encode().contains("/h/theirs"));
        drop((one, two));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_entry_let_go_of_before_the_next_save_is_not_taken_back() {
        let dir = scratch("adopt-removed");
        let file = dir.join("history");
        let session = start(file.clone());
        session.save(committed("/h/gone").encode());
        // Waits for the writer to have merged it, which is when the merger learns it was held.
        for _ in 0..200 {
            if file.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut now_empty = History::default();

        let adopted = session.adopt_into(&mut now_empty);

        assert_eq!(adopted, Ok(0));
        assert!(!now_empty.can_undo());
        drop(session);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Calls `take_changed` until it says yes, for up to two seconds.
    fn changed_soon(file: &mut HistoryFile) -> bool {
        for _ in 0..200 {
            if file.take_changed() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn another_session_saving_is_announced_by_the_watch() {
        let dir = scratch("announce");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("history");
        let mut mine = start(file.clone());
        assert!(mine.watch.is_some(), "the folder exists, so it is watched");
        assert!(!mine.take_changed(), "nothing happened yet");

        let theirs = start(file);
        theirs.save(committed("/h/theirs").encode());

        assert!(changed_soon(&mut mine));
        drop((mine, theirs));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_a_watch_a_new_modified_time_is_the_announcement() {
        let dir = scratch("poll");
        let file = dir.join("history");
        // The folder does not exist yet, so it cannot be watched.
        let mut mine = start(file.clone());
        assert!(mine.watch.is_none());
        assert!(!mine.take_changed(), "no file, and none seen before");

        let theirs = start(file);
        theirs.save(committed("/h/theirs").encode());

        assert!(changed_soon(&mut mine));
        assert!(!mine.take_changed(), "said once per change");
        drop((mine, theirs));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_file_adopts_nothing() {
        let dir = scratch("adopt-missing");
        let writer = start(dir.join("history"));

        assert_eq!(writer.adopt_into(&mut History::default()), Ok(0));
        drop(writer);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
