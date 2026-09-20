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

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::VfsError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// Size in bytes. Zero when the entry's metadata couldn't be read (e.g. a broken symlink),
    /// and meaningless for a directory.
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Unix permission bits (`st_mode`), when the backend has them.
    pub mode: Option<u32>,
}

pub trait Vfs {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError>;
    fn is_dir(&self, path: &Path) -> bool;
    fn exists(&self, path: &Path) -> bool;

    /// Creates a single directory. Fails with `VfsError::AlreadyExists` if `path` already
    /// exists, and does not create missing parents (mirrors `std::fs::create_dir`).
    fn create_dir(&self, path: &Path) -> Result<(), VfsError>;

    /// Creates a new, empty file. Fails with `VfsError::AlreadyExists` rather than truncating
    /// an existing file at `path`.
    fn create_file(&self, path: &Path) -> Result<(), VfsError>;

    /// Creates an empty file at `path`, or, if a file is already there, sets its modified time
    /// to now without touching its content (the `touch` command's contract).
    fn touch(&self, path: &Path) -> Result<(), VfsError>;

    /// Copies a single file. Fails with `VfsError::AlreadyExists` if `dst` already exists —
    /// never silently overwrites.
    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), VfsError>;

    /// Renames/moves `src` to `dst` in one filesystem operation. Fails with
    /// `VfsError::AlreadyExists` if `dst` already exists, rather than replacing it.
    fn rename(&self, src: &Path, dst: &Path) -> Result<(), VfsError>;

    fn remove_file(&self, path: &Path) -> Result<(), VfsError>;

    /// Removes a directory and everything under it.
    fn remove_dir_all(&self, path: &Path) -> Result<(), VfsError>;
}

#[cfg(unix)]
fn unix_mode(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode())
}

#[cfg(not(unix))]
fn unix_mode(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalVfs;

impl Vfs for LocalVfs {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError> {
        if !path.is_dir() {
            return Err(VfsError::NotADirectory(path.to_path_buf()));
        }

        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|source| VfsError::Io {
            path: path.to_path_buf(),
            source,
        })? {
            let entry = entry.map_err(|source| VfsError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            let entry_path = entry.path();
            // One `stat` (following symlinks, like `is_dir` did) answers everything at once, so
            // showing size/age/permissions costs no extra syscalls over the old listing.
            let metadata = entry_path.metadata().ok();
            let name = entry.file_name().to_string_lossy().into_owned();
            entries.push(DirEntryInfo {
                name,
                path: entry_path,
                is_dir: metadata.as_ref().is_some_and(|m| m.is_dir()),
                size: metadata.as_ref().map_or(0, |m| m.len()),
                modified: metadata.as_ref().and_then(|m| m.modified().ok()),
                mode: metadata.as_ref().and_then(unix_mode),
            });
        }

        // Directories first, then alphabetical within each group.
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        Ok(entries)
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn create_dir(&self, path: &Path) -> Result<(), VfsError> {
        std::fs::create_dir(path).map_err(|source| map_io_err(path, source))
    }

    fn create_file(&self, path: &Path) -> Result<(), VfsError> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(|_| ())
            .map_err(|source| map_io_err(path, source))
    }

    fn touch(&self, path: &Path) -> Result<(), VfsError> {
        // `create(true)` without `truncate` leaves an existing file's content alone, and opening
        // for write is what `set_modified` needs.
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .and_then(|file| file.set_modified(std::time::SystemTime::now()))
            .map_err(|source| map_io_err(path, source))
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        // `std::fs::copy` silently overwrites an existing `dst`; check first so a conflicting
        // destination is reported instead of clobbered.
        if dst.exists() {
            return Err(VfsError::AlreadyExists(dst.to_path_buf()));
        }
        std::fs::copy(src, dst)
            .map(|_| ())
            .map_err(|source| map_io_err(src, source))
    }

    fn rename(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        // `std::fs::rename` replaces an existing `dst` on most platforms; check first for the
        // same reason as `copy_file`.
        if dst.exists() {
            return Err(VfsError::AlreadyExists(dst.to_path_buf()));
        }
        std::fs::rename(src, dst).map_err(|source| map_io_err(src, source))
    }

    fn remove_file(&self, path: &Path) -> Result<(), VfsError> {
        std::fs::remove_file(path).map_err(|source| map_io_err(path, source))
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), VfsError> {
        std::fs::remove_dir_all(path).map_err(|source| map_io_err(path, source))
    }
}

fn map_io_err(path: &Path, source: std::io::Error) -> VfsError {
    match source.kind() {
        std::io::ErrorKind::NotFound => VfsError::NotFound(path.to_path_buf()),
        std::io::ErrorKind::AlreadyExists => VfsError::AlreadyExists(path.to_path_buf()),
        _ => VfsError::Io {
            path: path.to_path_buf(),
            source,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_directories_before_files_alphabetically() {
        let tmp = std::env::temp_dir().join(format!("minuteman-test-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("b_dir")).unwrap();
        std::fs::create_dir_all(tmp.join("a_dir")).unwrap();
        std::fs::write(tmp.join("a_file.txt"), b"hi").unwrap();

        let vfs = LocalVfs;
        let entries = vfs.list_dir(&tmp).unwrap();

        assert_eq!(entries[0].name, "a_dir");
        assert_eq!(entries[1].name, "b_dir");
        assert_eq!(entries[2].name, "a_file.txt");

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn listing_reports_size_modified_time_and_permissions() {
        let tmp = std::env::temp_dir().join(format!("minuteman-test-meta-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("five.txt"), b"12345").unwrap();

        let entries = LocalVfs.list_dir(&tmp).unwrap();
        let file = &entries[0];

        assert_eq!(file.size, 5);
        assert!(file.modified.is_some());
        #[cfg(unix)]
        assert!(file.mode.is_some_and(|m| m & 0o400 != 0));

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_broken_symlink_lists_as_a_zero_sized_non_directory() {
        let tmp = std::env::temp_dir().join(format!("minuteman-test-link-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.join("missing"), tmp.join("dangling")).unwrap();
            let entries = LocalVfs.list_dir(&tmp).unwrap();
            assert!(!entries[0].is_dir);
            assert_eq!(entries[0].size, 0);
            assert_eq!(entries[0].modified, None);
        }
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn rejects_non_directory_paths() {
        let vfs = LocalVfs;
        let tmp = std::env::temp_dir().join(format!("minuteman-test-file-{}", std::process::id()));
        std::fs::write(&tmp, b"hi").unwrap();

        let result = vfs.list_dir(&tmp);
        assert!(matches!(result, Err(VfsError::NotADirectory(_))));

        std::fs::remove_file(&tmp).unwrap();
    }

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-vfs-test-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_file_fails_instead_of_truncating_existing_file() {
        let dir = scratch_dir("create-file");
        let target = dir.join("f.txt");
        std::fs::write(&target, b"keep me").unwrap();

        let vfs = LocalVfs;
        let result = vfs.create_file(&target);

        assert!(matches!(result, Err(VfsError::AlreadyExists(_))));
        assert_eq!(std::fs::read(&target).unwrap(), b"keep me");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn touch_creates_a_missing_file_and_keeps_an_existing_files_content() {
        let dir = scratch_dir("touch");
        let vfs = LocalVfs;

        let fresh = dir.join("fresh.txt");
        vfs.touch(&fresh).unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), b"");

        let existing = dir.join("existing.txt");
        std::fs::write(&existing, b"keep me").unwrap();
        let long_ago = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        std::fs::File::options()
            .write(true)
            .open(&existing)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();

        vfs.touch(&existing).unwrap();

        assert_eq!(std::fs::read(&existing).unwrap(), b"keep me");
        assert!(std::fs::metadata(&existing).unwrap().modified().unwrap() > long_ago);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copy_file_fails_instead_of_overwriting_existing_destination() {
        let dir = scratch_dir("copy-file");
        let src = dir.join("src.txt");
        let dst = dir.join("dst.txt");
        std::fs::write(&src, b"new").unwrap();
        std::fs::write(&dst, b"keep me").unwrap();

        let vfs = LocalVfs;
        let result = vfs.copy_file(&src, &dst);

        assert!(matches!(result, Err(VfsError::AlreadyExists(_))));
        assert_eq!(std::fs::read(&dst).unwrap(), b"keep me");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rename_fails_instead_of_replacing_existing_destination() {
        let dir = scratch_dir("rename");
        let src = dir.join("src.txt");
        let dst = dir.join("dst.txt");
        std::fs::write(&src, b"new").unwrap();
        std::fs::write(&dst, b"keep me").unwrap();

        let vfs = LocalVfs;
        let result = vfs.rename(&src, &dst);

        assert!(matches!(result, Err(VfsError::AlreadyExists(_))));
        assert_eq!(std::fs::read(&dst).unwrap(), b"keep me");
        assert!(src.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remove_dir_all_deletes_nested_contents() {
        let dir = scratch_dir("remove-dir-all");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested").join("f.txt"), b"hi").unwrap();

        let vfs = LocalVfs;
        vfs.remove_dir_all(&dir).unwrap();

        assert!(!dir.exists());
    }

    #[test]
    fn missing_paths_map_to_not_found() {
        let dir = scratch_dir("not-found");
        let missing = dir.join("nope.txt");

        let vfs = LocalVfs;
        assert!(matches!(
            vfs.remove_file(&missing),
            Err(VfsError::NotFound(_))
        ));
        assert!(!vfs.exists(&missing));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
