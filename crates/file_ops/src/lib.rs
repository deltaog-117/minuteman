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

//! Copy/move/delete/create/rename orchestration on top of the `Vfs` trait — no direct
//! `std::fs` calls here, so these operations work against any future `Vfs` backend.

pub mod error;

pub use error::FileOpsError;

use std::path::Path;

use shared::Vfs;

pub fn copy(vfs: &dyn Vfs, src: &Path, dst: &Path) -> Result<(), FileOpsError> {
    guard_distinct(src, dst)?;
    guard_not_recursive(src, dst)?;

    if vfs.is_dir(src) {
        copy_dir(vfs, src, dst)
    } else {
        vfs.copy_file(src, dst).map_err(Into::into)
    }
}

fn copy_dir(vfs: &dyn Vfs, src: &Path, dst: &Path) -> Result<(), FileOpsError> {
    vfs.create_dir(dst)?;
    for entry in vfs.list_dir(src)? {
        let target = dst.join(&entry.name);
        if entry.is_dir {
            copy_dir(vfs, &entry.path, &target)?;
        } else {
            vfs.copy_file(&entry.path, &target)?;
        }
    }
    Ok(())
}

/// Moves `src` to `dst` via a single filesystem rename. Cross-filesystem moves (where rename
/// fails because `src` and `dst` live on different devices) are not yet supported — that
/// needs a copy+delete fallback, deferred to the async bulk-ops cycle where progress reporting
/// for the copy phase actually matters.
pub fn mv(vfs: &dyn Vfs, src: &Path, dst: &Path) -> Result<(), FileOpsError> {
    guard_distinct(src, dst)?;
    guard_not_recursive(src, dst)?;

    vfs.rename(src, dst).map_err(Into::into)
}

pub fn delete(vfs: &dyn Vfs, path: &Path) -> Result<(), FileOpsError> {
    if vfs.is_dir(path) {
        vfs.remove_dir_all(path).map_err(Into::into)
    } else {
        vfs.remove_file(path).map_err(Into::into)
    }
}

pub fn create_directory(vfs: &dyn Vfs, path: &Path) -> Result<(), FileOpsError> {
    vfs.create_dir(path).map_err(Into::into)
}

pub fn create_new_file(vfs: &dyn Vfs, path: &Path) -> Result<(), FileOpsError> {
    vfs.create_file(path).map_err(Into::into)
}

/// Renames `path` to `new_name` within the same parent directory.
pub fn rename(vfs: &dyn Vfs, path: &Path, new_name: &str) -> Result<(), FileOpsError> {
    let parent = path
        .parent()
        .ok_or_else(|| FileOpsError::NoParent(path.to_path_buf()))?;
    let dst = parent.join(new_name);
    mv(vfs, path, &dst)
}

fn guard_distinct(src: &Path, dst: &Path) -> Result<(), FileOpsError> {
    if src == dst {
        return Err(FileOpsError::SameLocation(src.to_path_buf()));
    }
    Ok(())
}

fn guard_not_recursive(src: &Path, dst: &Path) -> Result<(), FileOpsError> {
    if dst.starts_with(src) {
        return Err(FileOpsError::RecursiveDestination {
            src: src.to_path_buf(),
            dst: dst.to_path_buf(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{LocalVfs, VfsError};
    use std::path::PathBuf;

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-file-ops-test-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copy_file_duplicates_contents() {
        let dir = scratch_dir("copy-file");
        let src = dir.join("a.txt");
        let dst = dir.join("b.txt");
        std::fs::write(&src, b"hello").unwrap();

        copy(&LocalVfs, &src, &dst).unwrap();

        assert_eq!(std::fs::read(&dst).unwrap(), b"hello");
        assert!(src.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copy_dir_recurses_into_subdirectories() {
        let dir = scratch_dir("copy-dir");
        let src = dir.join("src");
        let dst = dir.join("dst");
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::write(src.join("nested").join("f.txt"), b"hi").unwrap();

        copy(&LocalVfs, &src, &dst).unwrap();

        assert_eq!(
            std::fs::read(dst.join("nested").join("f.txt")).unwrap(),
            b"hi"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copy_rejects_destination_inside_source() {
        let dir = scratch_dir("copy-recursive");
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let dst = src.join("nested");

        let result = copy(&LocalVfs, &src, &dst);

        assert!(matches!(
            result,
            Err(FileOpsError::RecursiveDestination { .. })
        ));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mv_relocates_file() {
        let dir = scratch_dir("mv");
        let src = dir.join("a.txt");
        let dst = dir.join("b.txt");
        std::fs::write(&src, b"hello").unwrap();

        mv(&LocalVfs, &src, &dst).unwrap();

        assert!(!src.exists());
        assert_eq!(std::fs::read(&dst).unwrap(), b"hello");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mv_fails_instead_of_replacing_existing_destination() {
        let dir = scratch_dir("mv-conflict");
        let src = dir.join("a.txt");
        let dst = dir.join("b.txt");
        std::fs::write(&src, b"new").unwrap();
        std::fs::write(&dst, b"keep me").unwrap();

        let result = mv(&LocalVfs, &src, &dst);

        assert!(matches!(
            result,
            Err(FileOpsError::Vfs(VfsError::AlreadyExists(_)))
        ));
        assert_eq!(std::fs::read(&dst).unwrap(), b"keep me");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_removes_file_and_nonempty_directory() {
        let dir = scratch_dir("delete");
        let file = dir.join("a.txt");
        std::fs::write(&file, b"hi").unwrap();
        let sub = dir.join("sub");
        std::fs::create_dir_all(sub.join("nested")).unwrap();
        std::fs::write(sub.join("nested").join("f.txt"), b"hi").unwrap();

        delete(&LocalVfs, &file).unwrap();
        delete(&LocalVfs, &sub).unwrap();

        assert!(!file.exists());
        assert!(!sub.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn create_new_file_fails_on_existing_path() {
        let dir = scratch_dir("create-file");
        let target = dir.join("a.txt");
        std::fs::write(&target, b"keep me").unwrap();

        let result = create_new_file(&LocalVfs, &target);

        assert!(matches!(
            result,
            Err(FileOpsError::Vfs(VfsError::AlreadyExists(_)))
        ));
        assert_eq!(std::fs::read(&target).unwrap(), b"keep me");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn create_directory_makes_a_new_dir() {
        let dir = scratch_dir("create-dir");
        let target = dir.join("sub");

        create_directory(&LocalVfs, &target).unwrap();

        assert!(target.is_dir());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rename_changes_name_within_same_parent() {
        let dir = scratch_dir("rename");
        let src = dir.join("old.txt");
        std::fs::write(&src, b"hi").unwrap();

        rename(&LocalVfs, &src, "new.txt").unwrap();

        assert!(!src.exists());
        assert_eq!(std::fs::read(dir.join("new.txt")).unwrap(), b"hi");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn same_source_and_destination_is_rejected() {
        let dir = scratch_dir("same-location");
        let path = dir.join("a.txt");
        std::fs::write(&path, b"hi").unwrap();

        let result = mv(&LocalVfs, &path, &path);

        assert!(matches!(result, Err(FileOpsError::SameLocation(_))));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
