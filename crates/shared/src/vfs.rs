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

use std::io::{Read, Seek, Write};
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

/// What kind of thing a path is. A symlink is its own kind: `Vfs::symlink_metadata` reports one,
/// and `Vfs::metadata` reports what it points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Dir,
    Symlink,
    /// A socket, pipe or device.
    Other,
}

/// Everything a properties view, a size scan or a preview needs to know about one path. Every
/// field but `kind` and `len` is optional, because a backend reports what it has (a remote one
/// may have no inode numbers, a FAT disk no owner); a caller leaves a missing field out rather
/// than guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub kind: FileKind,
    /// Length in bytes. For a folder it is the directory node's own size, not its contents.
    pub len: u64,
    /// Bytes allocated on the backing store, which differs from `len` for a sparse or compressed
    /// file and is rounded up to whole blocks.
    pub allocated: Option<u64>,
    /// The permission and type bits (`st_mode`).
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    /// How many names the file has; over one means it is hard-linked.
    pub nlink: Option<u64>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    /// When the file was created, on a filesystem that records it.
    pub created: Option<SystemTime>,
    /// Which filesystem the file is on. With `inode` it identifies a file for hard-link counting,
    /// and a scan uses it to stay on the filesystem it started on.
    pub device: Option<u64>,
    pub inode: Option<u64>,
}

/// One entry of [`Vfs::scan_dir`]: its path and, unless it could not be read, its own metadata
/// (a symlink is described as the link, not what it points at).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEntry {
    pub path: PathBuf,
    pub name: String,
    pub metadata: Option<Metadata>,
}

/// A readable, seekable stream of a file's bytes, as [`Vfs::open_read`] returns.
pub trait ReadSeek: Read + Seek + Send {}

impl<T: Read + Seek + Send> ReadSeek for T {}

/// A writable, seekable stream into a new file, as [`Vfs::create_write`] returns. Seekable because
/// a zip goes back to patch an entry's header once its size is known.
pub trait WriteSeek: Write + Seek + Send {}

impl<T: Write + Seek + Send> WriteSeek for T {}

/// A filesystem the file manager can browse: the local disk, and in time remote ones. Every
/// feature that touches a browsed path goes through this, so a backend added here works with all
/// of them. `Send + Sync` because background jobs (copies, scans, previews) share one.
///
/// `shared::conformance::check_backend` is the test a backend must pass.
pub trait Vfs: Send + Sync {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError>;
    fn is_dir(&self, path: &Path) -> bool;
    fn exists(&self, path: &Path) -> bool;

    /// Describes `path`, following symlinks (like `stat`).
    fn metadata(&self, path: &Path) -> Result<Metadata, VfsError>;

    /// Describes `path` itself, so a symlink is reported as a link (like `lstat`).
    fn symlink_metadata(&self, path: &Path) -> Result<Metadata, VfsError>;

    /// Where the symlink at `path` points, as written in the link.
    fn read_link(&self, path: &Path) -> Result<PathBuf, VfsError>;

    /// Opens the regular file at `path` for reading, following symlinks. A directory, or a file
    /// that cannot be read, is an error.
    fn open_read(&self, path: &Path) -> Result<Box<dyn ReadSeek>, VfsError>;

    /// Lists `path` for a scan: every entry with its own metadata, in no particular order and
    /// without the sorting and symlink-following `list_dir` does for display. One call, so a
    /// remote backend can answer in one round trip.
    fn scan_dir(&self, path: &Path) -> Result<Vec<ScanEntry>, VfsError>;

    /// The same file as a path on this machine's own disk, or `None` when the backend is not the
    /// local disk. A feature that can only work on a real local file (handing it to another
    /// program, reading an archive by seeking) asks this first and skips the file when it is
    /// `None`, so a remote backend degrades to "not available" instead of failing oddly.
    fn local_path(&self, path: &Path) -> Option<PathBuf>;

    /// Turns a location the user typed into the folder to open, `current` being the folder being
    /// browsed. A relative path is taken from `current`; a backend with another way of naming
    /// places (a remote machine's `ssh://host`, which opens at the login's home) says so here.
    fn resolve_typed(&self, typed: &Path, current: &Path) -> PathBuf {
        if typed.is_absolute() {
            typed.to_path_buf()
        } else {
            current.join(typed)
        }
    }

    /// Creates a new file at `path` and opens it for writing. Fails with
    /// `VfsError::AlreadyExists` if anything is there, a symlink (even a dangling one) included,
    /// which is never followed, and never truncates or replaces what is there. It does not create
    /// missing parents. This is `open(O_CREAT | O_EXCL)`: archive extraction relies on it so an
    /// entry can never be written through a link.
    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteSeek>, VfsError>;

    /// Makes a symlink at `link` pointing at `target`, which is stored as written and need not
    /// exist. Fails with `VfsError::AlreadyExists` if anything is at `link`, and does not create
    /// missing parents.
    fn create_symlink(&self, target: &Path, link: &Path) -> Result<(), VfsError>;

    /// Sets the permission bits (the low twelve bits of `st_mode`: rwx for owner, group and
    /// other, and setuid, setgid and sticky) of the file or folder at `path`, following a
    /// symlink. A backend with no permissions answers `VfsError::Unsupported`.
    fn set_mode(&self, path: &Path, mode: u32) -> Result<(), VfsError>;

    /// Moves `src` to `dst`, replacing a file or symlink already at `dst` (a folder in the way is
    /// an error). Atomic where the backend can make it so, which is what lets a finished archive
    /// appear under its real name all at once; otherwise the old file may briefly be missing.
    fn replace(&self, src: &Path, dst: &Path) -> Result<(), VfsError>;

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

/// `std`'s metadata as this crate's. The owner, link count, allocated size and device/inode are
/// Unix facts; elsewhere they are `None`.
fn convert(metadata: &std::fs::Metadata) -> Metadata {
    let file_type = metadata.file_type();
    let kind = if file_type.is_symlink() {
        FileKind::Symlink
    } else if file_type.is_dir() {
        FileKind::Dir
    } else if file_type.is_file() {
        FileKind::File
    } else {
        FileKind::Other
    };
    #[cfg(unix)]
    let (mode, uid, gid, nlink, allocated, device, inode) = {
        use std::os::unix::fs::MetadataExt;
        (
            Some(metadata.mode()),
            Some(metadata.uid()),
            Some(metadata.gid()),
            Some(metadata.nlink()),
            // `st_blocks` counts 512-byte units whatever the filesystem's block size is.
            Some(metadata.blocks().saturating_mul(512)),
            Some(metadata.dev()),
            Some(metadata.ino()),
        )
    };
    #[cfg(not(unix))]
    let (mode, uid, gid, nlink, allocated, device, inode) =
        (None, None, None, None, None, None, None);
    Metadata {
        kind,
        len: metadata.len(),
        allocated,
        mode,
        uid,
        gid,
        nlink,
        modified: metadata.modified().ok(),
        accessed: metadata.accessed().ok(),
        created: metadata.created().ok(),
        device,
        inode,
    }
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

    fn metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        std::fs::metadata(path)
            .map(|m| convert(&m))
            .map_err(|source| map_io_err(path, source))
    }

    fn symlink_metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        std::fs::symlink_metadata(path)
            .map(|m| convert(&m))
            .map_err(|source| map_io_err(path, source))
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, VfsError> {
        std::fs::read_link(path).map_err(|source| map_io_err(path, source))
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn ReadSeek>, VfsError> {
        // `open` on a directory succeeds on Unix and only fails on the first read, so say so now.
        if path.is_dir() {
            return Err(VfsError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "is a directory"),
            });
        }
        std::fs::File::open(path)
            .map(|file| Box::new(file) as Box<dyn ReadSeek>)
            .map_err(|source| map_io_err(path, source))
    }

    fn scan_dir(&self, path: &Path) -> Result<Vec<ScanEntry>, VfsError> {
        let io = |source| VfsError::Io {
            path: path.to_path_buf(),
            source,
        };
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path).map_err(io)? {
            let entry = entry.map_err(io)?;
            entries.push(ScanEntry {
                path: entry.path(),
                name: entry.file_name().to_string_lossy().into_owned(),
                // `DirEntry::metadata` does not follow a symlink, which is what a scan wants.
                metadata: entry.metadata().ok().map(|m| convert(&m)),
            });
        }
        Ok(entries)
    }

    fn local_path(&self, path: &Path) -> Option<PathBuf> {
        Some(path.to_path_buf())
    }

    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteSeek>, VfsError> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(|file| Box::new(file) as Box<dyn WriteSeek>)
            .map_err(|source| map_io_err(path, source))
    }

    #[cfg(unix)]
    fn create_symlink(&self, target: &Path, link: &Path) -> Result<(), VfsError> {
        std::os::unix::fs::symlink(target, link).map_err(|source| map_io_err(link, source))
    }

    #[cfg(not(unix))]
    fn create_symlink(&self, _target: &Path, _link: &Path) -> Result<(), VfsError> {
        Err(VfsError::Unsupported("symlinks"))
    }

    #[cfg(unix)]
    fn set_mode(&self, path: &Path, mode: u32) -> Result<(), VfsError> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777))
            .map_err(|source| map_io_err(path, source))
    }

    #[cfg(not(unix))]
    fn set_mode(&self, _path: &Path, _mode: u32) -> Result<(), VfsError> {
        Err(VfsError::Unsupported("permissions"))
    }

    fn replace(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        // `rename` replaces an existing file atomically on every platform `std` supports.
        std::fs::rename(src, dst).map_err(|source| map_io_err(src, source))
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
