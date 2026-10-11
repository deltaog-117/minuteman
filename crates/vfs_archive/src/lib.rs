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

//! Browses an archive as a folder: a [`Vfs`] that wraps another, sends every path inside an
//! archive (`/x/a.zip!/dir/f`) to the plugin that serves archives, and passes every other path to
//! the one underneath untouched.
//!
//! Because it is a `Vfs`, everything that already goes through one works inside an archive with no
//! code of its own: browsing, previews, search, the disk usage view, and copying a file or a
//! whole folder out. The archive can sit on the local disk or on a remote machine: the plugin
//! reads the archive file back through the host, which reads it through whatever serves the paths.
//!
//! The plugin does the reading of the archive's table of contents and the unpacking of entries;
//! this crate only decides which paths are its business (`shared::route`), turns each call into a
//! [`VfsRequest`] and each reply back into the types `Vfs` returns, and reads a file in windows
//! so a small read costs no round trip. Archives are read-only: every way of changing one reports
//! [`VfsError::Unsupported`] without asking the plugin. With no plugin running, a path inside an
//! archive fails with a message saying so, and the rest of the file manager is untouched.
//!
//! Zip, tar and tar.gz are served; an archive inside an archive is just a file of the outer one.

mod reader;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use plugin_api::{
    Epoch, LinkTarget, Peer, PeerError, VfsFault, VfsRequest, WireEntry, WireKind, WireMeta,
    WireScan,
};
use serde::de::DeserializeOwned;
use shared::{DirEntryInfo, FileKind, Metadata, ReadSeek, ScanEntry, Vfs, VfsError, WriteSeek};

use reader::PluginReader;

pub use shared::{Format, Mount, Route, is_archive, is_inside, route};

const READ_ONLY: &str = "an archive is read-only";
const NOT_HERE: &str = "this is not available inside an archive";

pub struct ArchiveVfs {
    inner: Arc<dyn Vfs>,
    peer: Arc<dyn Peer>,
}

fn read_only<T>() -> Result<T, VfsError> {
    Err(VfsError::Unsupported(READ_ONLY))
}

fn seconds(seconds: u64) -> std::time::SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

fn kind_of(kind: WireKind) -> FileKind {
    match kind {
        WireKind::File => FileKind::File,
        WireKind::Dir => FileKind::Dir,
        WireKind::Symlink => FileKind::Symlink,
        WireKind::Other => FileKind::Other,
    }
}

fn metadata_of(meta: &WireMeta) -> Metadata {
    Metadata {
        kind: kind_of(meta.kind),
        len: meta.len,
        allocated: None,
        mode: meta.mode,
        uid: None,
        gid: None,
        nlink: None,
        modified: meta.modified.map(seconds),
        accessed: None,
        created: None,
        device: None,
        inode: None,
    }
}

fn entry_of(entry: WireEntry) -> DirEntryInfo {
    DirEntryInfo {
        name: entry.name,
        path: entry.path,
        is_dir: entry.is_dir,
        size: entry.size,
        modified: entry.modified.map(seconds),
        mode: entry.mode,
    }
}

fn scan_of(entry: WireScan) -> ScanEntry {
    ScanEntry {
        path: entry.path,
        name: entry.name,
        metadata: entry.meta.as_ref().map(metadata_of),
    }
}

/// What a failed request means to the caller that asked about `path`.
pub(crate) fn error_of(error: PeerError, path: &Path) -> VfsError {
    let io = |kind: io::ErrorKind, message: String| VfsError::Io {
        path: path.to_path_buf(),
        source: io::Error::new(kind, message),
    };
    match error {
        PeerError::Fault(VfsFault::NotFound) => VfsError::NotFound(path.to_path_buf()),
        PeerError::Fault(VfsFault::NotADirectory) => VfsError::NotADirectory(path.to_path_buf()),
        PeerError::Fault(VfsFault::AlreadyExists) => VfsError::AlreadyExists(path.to_path_buf()),
        PeerError::Fault(VfsFault::Unsupported { .. }) => VfsError::Unsupported(NOT_HERE),
        PeerError::Fault(VfsFault::Io { kind, message }) => io(kind.into(), message),
        // The reader's own windows make these two unlikely; they are the plugin's limits.
        error @ PeerError::Fault(VfsFault::BadHandle | VfsFault::TooManyOpen) => {
            io(io::ErrorKind::Other, error.to_string())
        }
        PeerError::Unavailable => io(
            io::ErrorKind::Unsupported,
            "browsing inside an archive needs the mman-archive plugin, which is not running"
                .to_string(),
        ),
        error @ (PeerError::Stopped
        | PeerError::TimedOut
        | PeerError::Stale
        | PeerError::Garbled(_)) => io(io::ErrorKind::Other, error.to_string()),
    }
}

impl ArchiveVfs {
    /// An `ArchiveVfs` over `inner` that asks `peer` about what is inside archives.
    pub fn new(inner: Arc<dyn Vfs>, peer: Arc<dyn Peer>) -> Self {
        Self { inner, peer }
    }

    /// Sends `request` about `path` and reads the reply as a `T`, with the epoch it came from.
    fn ask_at<T: DeserializeOwned>(
        &self,
        path: &Path,
        request: &VfsRequest,
        after: Option<Epoch>,
    ) -> Result<(T, Epoch), VfsError> {
        let answer = self
            .peer
            .call(request, after)
            .map_err(|error| error_of(error, path))?;
        let value = serde_json::from_value(answer.value)
            .map_err(|e| error_of(PeerError::Garbled(e.to_string()), path))?;
        Ok((value, answer.epoch))
    }

    fn ask<T: DeserializeOwned>(&self, path: &Path, request: &VfsRequest) -> Result<T, VfsError> {
        self.ask_at(path, request, None).map(|(value, _)| value)
    }

    /// Where `path` goes: `Ok(true)` for the plugin, `Ok(false)` for the backend underneath.
    fn inside(path: &Path) -> Result<bool, VfsError> {
        match route(path) {
            Route::Outside => Ok(false),
            Route::Escapes => Err(VfsError::NotFound(path.to_path_buf())),
            Route::Inside(_) => Ok(true),
        }
    }

    fn describe(&self, path: &Path, follow: bool) -> Result<Metadata, VfsError> {
        let meta: WireMeta = self.ask(
            path,
            &VfsRequest::Metadata {
                path: path.to_path_buf(),
                follow,
            },
        )?;
        Ok(metadata_of(&meta))
    }
}

impl Vfs for ArchiveVfs {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError> {
        if !Self::inside(path)? {
            return self.inner.list_dir(path);
        }
        let entries: Vec<WireEntry> = self.ask(
            path,
            &VfsRequest::List {
                path: path.to_path_buf(),
            },
        )?;
        Ok(entries.into_iter().map(entry_of).collect())
    }

    fn is_dir(&self, path: &Path) -> bool {
        match Self::inside(path) {
            Ok(true) => self
                .describe(path, true)
                .is_ok_and(|meta| meta.kind == FileKind::Dir),
            Ok(false) => self.inner.is_dir(path),
            Err(_) => false,
        }
    }

    fn exists(&self, path: &Path) -> bool {
        match Self::inside(path) {
            Ok(true) => self.describe(path, true).is_ok(),
            Ok(false) => self.inner.exists(path),
            Err(_) => false,
        }
    }

    fn metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        if Self::inside(path)? {
            self.describe(path, true)
        } else {
            self.inner.metadata(path)
        }
    }

    fn symlink_metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        if Self::inside(path)? {
            self.describe(path, false)
        } else {
            self.inner.symlink_metadata(path)
        }
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, VfsError> {
        if !Self::inside(path)? {
            return self.inner.read_link(path);
        }
        let link: LinkTarget = self.ask(
            path,
            &VfsRequest::ReadLink {
                path: path.to_path_buf(),
            },
        )?;
        Ok(link.target)
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn ReadSeek>, VfsError> {
        if !Self::inside(path)? {
            return self.inner.open_read(path);
        }
        let (opened, epoch): (plugin_api::Opened, _) = self.ask_at(
            path,
            &VfsRequest::Open {
                path: path.to_path_buf(),
            },
            None,
        )?;
        Ok(Box::new(PluginReader::new(
            Arc::clone(&self.peer),
            path.to_path_buf(),
            opened,
            epoch,
        )))
    }

    fn scan_dir(&self, path: &Path) -> Result<Vec<ScanEntry>, VfsError> {
        if !Self::inside(path)? {
            return self.inner.scan_dir(path);
        }
        let entries: Vec<WireScan> = self.ask(
            path,
            &VfsRequest::Scan {
                path: path.to_path_buf(),
            },
        )?;
        Ok(entries.into_iter().map(scan_of).collect())
    }

    fn local_path(&self, path: &Path) -> Option<PathBuf> {
        if is_inside(path) {
            // What is in an archive is not a file on any disk.
            return None;
        }
        self.inner.local_path(path)
    }

    fn resolve_typed(&self, typed: &Path, current: &Path) -> PathBuf {
        self.inner.resolve_typed(typed, current)
    }

    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteSeek>, VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.create_write(path)
    }

    fn create_symlink(&self, target: &Path, link: &Path) -> Result<(), VfsError> {
        if is_inside(link) {
            return read_only();
        }
        self.inner.create_symlink(target, link)
    }

    fn set_mode(&self, path: &Path, mode: u32) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.set_mode(path, mode)
    }

    fn replace(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        if is_inside(src) || is_inside(dst) {
            return read_only();
        }
        self.inner.replace(src, dst)
    }

    fn create_dir(&self, path: &Path) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.create_dir(path)
    }

    fn create_file(&self, path: &Path) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.create_file(path)
    }

    fn touch(&self, path: &Path) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.touch(path)
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        if is_inside(dst) {
            return read_only();
        }
        if !is_inside(src) {
            return self.inner.copy_file(src, dst);
        }
        // Out of an archive into a folder: read the entry and write it as a new file.
        if self.inner.symlink_metadata(dst).is_ok() {
            return Err(VfsError::AlreadyExists(dst.to_path_buf()));
        }
        let meta = self.metadata(src)?;
        let mut from = self.open_read(src)?;
        let mut to = self.inner.create_write(dst)?;
        let copied = io::copy(&mut from, &mut to).and_then(|_| to.flush());
        drop(to);
        if let Err(source) = copied {
            // A half-written copy would otherwise look like the real file.
            let _ = self.inner.remove_file(dst);
            return Err(VfsError::Io {
                path: dst.to_path_buf(),
                source,
            });
        }
        if let Some(mode) = meta.mode {
            // Best effort: the destination may have no permissions to set.
            let _ = self.inner.set_mode(dst, mode & 0o7777);
        }
        Ok(())
    }

    fn rename(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        if is_inside(src) || is_inside(dst) {
            return read_only();
        }
        self.inner.rename(src, dst)
    }

    fn remove_file(&self, path: &Path) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.remove_file(path)
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.remove_dir_all(path)
    }
}

#[cfg(test)]
mod tests;
