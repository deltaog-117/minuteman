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
//! new one, never half of either. Two `mman` running at once each write their own history over the
//! file; the last one to save wins.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use file_ops::history::History;

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
}

/// Reads the history saved at `path`. Anything that goes wrong starts an empty history and says
/// so in `notice`: not being able to undo what was done last week must never stop `mman` starting.
pub fn load(path: &Path, capacity: usize) -> Loaded {
    let empty = |notice: Option<String>| Loaded {
        history: History::with_capacity(capacity),
        notice,
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
            history: restored.history,
            notice: (restored.dropped > 0).then(|| {
                format!(
                    "{} undo history entries could not be brought back (their trash items are gone, or the lines were damaged)",
                    restored.dropped
                )
            }),
        },
        Err(e) => empty(Some(format!("starting with an empty undo history: {e}"))),
    }
}

/// The writer thread and the way to it.
pub struct HistoryFile {
    /// `None` only while dropping, which closes it to tell the thread to finish.
    texts: Option<Sender<String>>,
    failures: Receiver<String>,
    writer: Option<JoinHandle<()>>,
}

impl HistoryFile {
    pub fn start(path: PathBuf) -> Self {
        let (texts, queue) = mpsc::channel::<String>();
        let (failed, failures) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            while let Ok(mut text) = queue.recv() {
                while let Ok(newer) = queue.try_recv() {
                    text = newer;
                }
                if let Err(e) = write_atomically(&path, &text) {
                    // The receiver is gone only when the app is already shutting down.
                    let _ = failed.send(e.to_string());
                }
            }
        });
        Self {
            texts: Some(texts),
            failures,
            writer: Some(writer),
        }
    }

    /// Hands `text` to the writer. Returns at once.
    pub fn save(&self, text: String) {
        if let Some(texts) = &self.texts {
            // A send fails only if the writer thread died, which would already have panicked
            // loudly; there is nowhere left to save to.
            let _ = texts.send(text);
        }
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
        let writer = HistoryFile::start(file.clone());
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
        let leftovers = std::fs::read_dir(file.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary file is left behind");
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
        let writer = HistoryFile::start(saved.clone());
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
            let writer = HistoryFile::start(file.clone());
            writer.save(History::default().encode());
            writer
        });

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(file.parent().unwrap()), 0o700);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_write_that_cannot_happen_is_reported_once() {
        let dir = scratch("blocked");
        std::fs::create_dir_all(&dir).unwrap();
        // A plain file where the folder should be.
        let file = dir.join("in-the-way").join("history");
        std::fs::write(dir.join("in-the-way"), "").unwrap();
        let writer = HistoryFile::start(file);
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
}
