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
//! This is a local-filesystem concept with no equivalent in the `Vfs` trait `shared`/`file_ops`
//! build everything else on — there is no "SSH trash" — so, unlike the rest of `file_ops`, sending
//! something here is not backend-agnostic.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Wrapped as plain text rather than depending on `os_trash::Error`'s own trait shape, which
/// this crate has no control over.
#[derive(Debug, thiserror::Error)]
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

    #[test]
    fn a_path_that_was_never_trashed_claims_nothing() {
        let before = snapshot().unwrap();
        let claimed = claim(&before, &[PathBuf::from("/nowhere/at/all.txt")]).unwrap();
        assert_eq!(claimed, vec![None]);
    }
}
