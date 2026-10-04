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

//! Sends a path to the real desktop trash — the freedesktop.org trash on Linux, the Recycle Bin
//! on Windows, the Trash on macOS — the same one Nautilus, Dolphin, Explorer and Finder use, via
//! the `trash` crate (imported here as `os_trash`; this crate is itself named `trash`, which
//! `cargo` won't let depend on the identically-named crates.io package under its own name).
//!
//! Besides sending and restoring what Minuteman itself trashed (for undo), it can list everything
//! in the trash and restore or permanently delete chosen items, for the trash browser.
//!
//! This is a local-filesystem concept with no equivalent in the `Vfs` trait `shared`/`file_ops`
//! build everything else on — there is no "SSH trash" — so, unlike the rest of `file_ops`, sending
//! something here is not backend-agnostic.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Wrapped as plain text rather than depending on `os_trash::Error`'s own trait shape, which
/// this crate has no control over.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct TrashError(String);

impl From<&str> for TrashError {
    fn from(message: &str) -> Self {
        Self(message.to_owned())
    }
}

/// Moves `path` to the desktop trash.
pub fn send(path: &Path) -> Result<(), TrashError> {
    os_trash::delete(path).map_err(|e| TrashError(e.to_string()))
}

/// One file or folder sitting in the desktop trash, kept so it can be put back where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trashed(os_trash::TrashItem);

/// What the trash held at one moment, so whatever appears after it can be told apart from what
/// was already there.
#[derive(Debug, Clone)]
pub struct Snapshot(HashSet<OsString>);

/// Records the trash's current contents, to be passed to `claim` after sending paths to it.
pub fn snapshot() -> Result<Snapshot, TrashError> {
    imp::list().map(|items| Snapshot(items.into_iter().map(|item| item.id).collect()))
}

/// Finds, for each of `paths`, the trash item that appeared since `before` was taken. A path with
/// no new item (the trash could not be read back, or its original location was recorded under a
/// different spelling) yields `None`, and the caller treats that one as not restorable.
pub fn claim(before: &Snapshot, paths: &[PathBuf]) -> Result<Vec<Option<Trashed>>, TrashError> {
    let added: Vec<os_trash::TrashItem> = imp::list()?
        .into_iter()
        .filter(|item| !before.0.contains(&item.id))
        .collect();
    Ok(paths
        .iter()
        .map(|path| {
            // The trash records the parent directory with symlinks resolved, so match on both
            // spellings; the newest wins if the same name was trashed twice in one batch.
            let resolved = path
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .zip(path.file_name())
                .map(|(parent, name)| parent.join(name));
            added
                .iter()
                .filter(|item| {
                    let original = item.original_path();
                    original == *path || resolved.as_ref() == Some(&original)
                })
                .max_by_key(|item| item.time_deleted)
                .cloned()
                .map(Trashed)
        })
        .collect())
}

/// Puts `item` back at its original location. Fails, leaving it in the trash, if something
/// already exists there.
pub fn restore(item: &Trashed) -> Result<(), TrashError> {
    imp::restore(item.0.clone())
}

/// How big a trashed item is: bytes for a file, the number of entries directly inside for a folder
/// (the trash does not total a folder's contents).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySize {
    Bytes(u64),
    Entries(usize),
    /// The trash could not say.
    Unknown,
}

/// One item in the trash, as plain data a browser can list, sort and test with. `id` is the
/// trash's own name for it, which `restore_entries` and `purge_entries` find it by again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    pub id: String,
    pub name: String,
    /// The folder it was in when it was trashed.
    pub original_dir: PathBuf,
    /// When it was trashed, in seconds since the Unix epoch.
    pub deleted: i64,
    pub size: EntrySize,
}

impl TrashEntry {
    /// Where it would go back to.
    pub fn original_path(&self) -> PathBuf {
        self.original_dir.join(&self.name)
    }
}

/// Everything in the desktop trash, newest first.
///
/// # Errors
///
/// The trash could not be read, or this platform's cannot be.
pub fn entries() -> Result<Vec<TrashEntry>, TrashError> {
    let mut items = imp::list()?;
    items.sort_by_key(|item| std::cmp::Reverse(item.time_deleted));
    Ok(items.iter().map(entry_of).collect())
}

fn entry_of(item: &os_trash::TrashItem) -> TrashEntry {
    TrashEntry {
        id: item.id.to_string_lossy().into_owned(),
        name: item.name.to_string_lossy().into_owned(),
        original_dir: item.original_parent.clone(),
        deleted: item.time_deleted,
        size: imp::size(item),
    }
}

/// Puts each of `entries` back where it was, one result per entry, in order. One that cannot go
/// back (something is already there, or it is no longer in the trash) stays in the trash and says
/// why; the others are not held up by it.
pub fn restore_entries(entries: &[TrashEntry]) -> Vec<Result<(), TrashError>> {
    act_on(entries, imp::restore)
}

/// Deletes each of `entries` for good, one result per entry, in order.
pub fn purge_entries(entries: &[TrashEntry]) -> Vec<Result<(), TrashError>> {
    act_on(entries, imp::purge)
}

/// Looks the entries up in the trash as it is now — once — and applies `act` to each.
fn act_on(
    entries: &[TrashEntry],
    act: fn(os_trash::TrashItem) -> Result<(), TrashError>,
) -> Vec<Result<(), TrashError>> {
    let current = match imp::list() {
        Ok(items) => items,
        Err(error) => return entries.iter().map(|_| Err(error.clone())).collect(),
    };
    entries
        .iter()
        .map(|entry| {
            match current
                .iter()
                .find(|item| item.id.to_string_lossy() == entry.id)
            {
                Some(item) => act(item.clone()),
                None => Err(TrashError("it is no longer in the trash".into())),
            }
        })
        .collect()
}

#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
mod imp {
    use super::TrashError;

    pub fn list() -> Result<Vec<os_trash::TrashItem>, TrashError> {
        os_trash::os_limited::list().map_err(|e| TrashError(e.to_string()))
    }

    pub fn restore(item: os_trash::TrashItem) -> Result<(), TrashError> {
        os_trash::os_limited::restore_all([item]).map_err(|e| TrashError(e.to_string()))
    }

    pub fn purge(item: os_trash::TrashItem) -> Result<(), TrashError> {
        os_trash::os_limited::purge_all([item]).map_err(|e| TrashError(e.to_string()))
    }

    pub fn size(item: &os_trash::TrashItem) -> super::EntrySize {
        match os_trash::os_limited::metadata(item).map(|meta| meta.size) {
            Ok(os_trash::TrashItemSize::Bytes(bytes)) => super::EntrySize::Bytes(bytes),
            Ok(os_trash::TrashItemSize::Entries(entries)) => super::EntrySize::Entries(entries),
            Err(_) => super::EntrySize::Unknown,
        }
    }
}

/// The platforms whose trash cannot be listed or restored from (macOS): sending still works, so
/// an operation is simply not undoable there.
#[cfg(not(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
mod imp {
    use super::TrashError;

    pub fn list() -> Result<Vec<os_trash::TrashItem>, TrashError> {
        Err(TrashError(
            "this platform's trash cannot be read back".into(),
        ))
    }

    pub fn restore(_item: os_trash::TrashItem) -> Result<(), TrashError> {
        Err(TrashError("this platform's trash cannot restore".into()))
    }

    pub fn purge(_item: os_trash::TrashItem) -> Result<(), TrashError> {
        Err(TrashError(
            "this platform's trash cannot be emptied from here".into(),
        ))
    }

    pub fn size(_item: &os_trash::TrashItem) -> super::EntrySize {
        super::EntrySize::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sending_a_file_removes_it_from_its_original_location() {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-trash-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, b"hi").unwrap();

        send(&file).unwrap();

        assert!(!file.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sending_a_missing_path_fails() {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-trash-test-missing-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("nope.txt");

        assert!(send(&missing).is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_trashed_file_is_found_again_and_restored_to_where_it_was() {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-trash-test-restore-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("back.txt");
        std::fs::write(&file, b"hi").unwrap();

        let before = snapshot().unwrap();
        send(&file).unwrap();
        let claimed = claim(&before, std::slice::from_ref(&file)).unwrap();
        let item = claimed[0].as_ref().expect("the new trash item is found");
        assert!(!file.exists());

        restore(item).unwrap();

        assert_eq!(std::fs::read(&file).unwrap(), b"hi");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A unique folder with one file in it, sent to the trash; returns the folder, the file's
    /// original path and its entry. Only this file is ever touched in the user's real trash.
    fn trashed(label: &str) -> (PathBuf, PathBuf, TrashEntry) {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-trash-test-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("{label}.txt"));
        std::fs::write(&file, b"12345").unwrap();
        send(&file).unwrap();
        let entry = entries()
            .unwrap()
            .into_iter()
            .find(|entry| entry.original_path() == file)
            .expect("the file shows up in the listing");
        (dir, file, entry)
    }

    #[test]
    fn the_listing_describes_a_trashed_file() {
        let (dir, file, entry) = trashed("listing");
        assert_eq!(entry.name, "listing.txt");
        assert_eq!(entry.original_dir, dir);
        assert_eq!(entry.original_path(), file);
        assert_eq!(entry.size, EntrySize::Bytes(5));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!((now - entry.deleted).abs() < 120, "deleted just now");

        assert!(restore_entries(&[entry]).remove(0).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_listing_is_newest_first() {
        let all = entries().unwrap();
        assert!(
            all.windows(2)
                .all(|pair| pair[0].deleted >= pair[1].deleted)
        );
    }

    #[test]
    fn a_listed_entry_can_be_restored_and_is_then_gone_from_the_trash() {
        let (dir, file, entry) = trashed("restore-entry");
        assert!(!file.exists());

        let results = restore_entries(std::slice::from_ref(&entry));
        assert!(results[0].is_ok(), "{:?}", results[0]);
        assert_eq!(std::fs::read(&file).unwrap(), b"12345");
        assert!(entries().unwrap().iter().all(|other| other.id != entry.id));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restoring_onto_something_that_is_there_fails_and_keeps_the_item() {
        let (dir, file, entry) = trashed("restore-clash");
        std::fs::write(&file, b"in the way").unwrap();

        let results = restore_entries(std::slice::from_ref(&entry));
        assert!(results[0].is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"in the way", "left alone");
        assert!(
            entries().unwrap().iter().any(|other| other.id == entry.id),
            "still in the trash"
        );

        // Clean up: make room, put it back, and remove the folder.
        std::fs::remove_file(&file).unwrap();
        assert!(restore_entries(&[entry]).remove(0).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_purged_entry_is_gone_for_good() {
        let (dir, file, entry) = trashed("purge-entry");

        let results = purge_entries(std::slice::from_ref(&entry));
        assert!(results[0].is_ok(), "{:?}", results[0]);
        assert!(!file.exists());
        assert!(entries().unwrap().iter().all(|other| other.id != entry.id));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_entry_that_is_no_longer_there_reports_it_without_stopping_the_rest() {
        let (dir, _file, real) = trashed("gone-mixed");
        let ghost = TrashEntry {
            id: "/nowhere/info/ghost.trashinfo".into(),
            name: "ghost".into(),
            original_dir: PathBuf::from("/nowhere"),
            deleted: 0,
            size: EntrySize::Unknown,
        };

        let results = restore_entries(&[ghost, real]);
        assert!(results[0].is_err());
        assert!(results[1].is_ok(), "the real one was still restored");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_trashed_folder_reports_how_many_entries_it_holds() {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-trash-test-folder-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let sub = dir.join("pile");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("one"), b"1").unwrap();
        std::fs::write(sub.join("two"), b"2").unwrap();
        send(&sub).unwrap();
        let entry = entries()
            .unwrap()
            .into_iter()
            .find(|entry| entry.original_path() == sub)
            .expect("listed");
        assert_eq!(entry.size, EntrySize::Entries(2));

        assert!(purge_entries(&[entry]).remove(0).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_path_that_was_never_trashed_claims_nothing() {
        let before = snapshot().unwrap();
        let claimed = claim(&before, &[PathBuf::from("/nowhere/at/all.txt")]).unwrap();
        assert_eq!(claimed, vec![None]);
    }
}
