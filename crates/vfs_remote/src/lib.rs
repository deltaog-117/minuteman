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

//! Browses and changes remote machines as folders: a [`Vfs`] that sends every `ssh://` path to
//! the plugin that serves them, and every other path to the one underneath untouched.
//!
//! Because it is a `Vfs`, everything that already goes through one works on a remote machine
//! with no code of its own: browsing, previews, search, the disk usage view, copying and deleting.
//! The plugin holds the SFTP session with each machine; this crate only decides which paths are
//! its business (`shared::ssh_path`), turns each call into a [`VfsRequest`] and each reply back
//! into the types `Vfs` returns, and moves a file in windows so a small read or write costs no
//! round trip of its own.
//!
//! A copy between this machine and a remote one, or between two remote ones, is streamed through
//! here, since no single session can see both ends. A move between them is refused with
//! `ErrorKind::CrossesDevices`, so the caller falls back to copy and delete. With no plugin
//! running, a remote path fails with a message saying so, and the rest of the file manager is
//! untouched.

mod reader;
mod writer;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use plugin_api::{
    Epoch, LinkTarget, Opened, Peer, PeerError, Resolved, VfsFault, VfsRequest, WireEntry,
    WireKind, WireMeta, WireScan,
};
use serde::de::DeserializeOwned;
use shared::ssh_path::{Location, Remote, TargetError};
use shared::{DirEntryInfo, FileKind, Metadata, ReadSeek, ScanEntry, Vfs, VfsError, WriteSeek};

use reader::PluginReader;
use writer::PluginWriter;

const NOT_THERE: &str = "this is not available on that machine";

/// The text of the error a request gets while no `mman-ssh` is running, so the interface can tell
/// that case apart and say where to get the plugin instead.
pub const PLUGIN_ABSENT: &str =
    "browsing a remote machine needs the mman-ssh plugin, which is not running";

/// Whether `error` is the one a request gets while no `mman-ssh` is running.
pub fn is_plugin_absent(error: &VfsError) -> bool {
    matches!(
        error,
        VfsError::Io { source, .. }
            if source.kind() == io::ErrorKind::Unsupported && source.to_string() == PLUGIN_ABSENT
    )
}

pub struct SshVfs {
    inner: Arc<dyn Vfs>,
    peer: Arc<dyn Peer>,
}

/// Where one path goes.
enum Route {
    Local,
    Remote,
}

/// Where two paths go: a move needs both in the same place.
enum Pair {
    Local,
    SameMachine,
    Apart,
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
        // SFTP v3 reports neither blocks, link counts nor inodes; a caller leaves those out.
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

fn bad_address(path: &Path, error: &TargetError) -> VfsError {
    VfsError::Io {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidInput, error.to_string()),
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
        PeerError::Fault(VfsFault::Unsupported { .. }) => VfsError::Unsupported(NOT_THERE),
        PeerError::Fault(VfsFault::Io { kind, message }) => io(kind.into(), message),
        // The reader's and writer's own windows make these two unlikely; they are the plugin's
        // limits.
        error @ PeerError::Fault(VfsFault::BadHandle | VfsFault::TooManyOpen) => {
            io(io::ErrorKind::Other, error.to_string())
        }
        PeerError::Unavailable => io(io::ErrorKind::Unsupported, PLUGIN_ABSENT.to_string()),
        error @ (PeerError::Stopped
        | PeerError::TimedOut
        | PeerError::Stale
        | PeerError::Garbled(_)) => io(io::ErrorKind::Other, error.to_string()),
    }
}

impl SshVfs {
    /// An `SshVfs` over `inner` (the local disk) that asks `peer` about remote machines.
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

    /// Sends a request whose reply says nothing but that it worked.
    fn command(&self, path: &Path, request: &VfsRequest) -> Result<(), VfsError> {
        self.ask::<serde_json::Value>(path, request).map(|_| ())
    }

    fn route(path: &Path) -> Result<Route, VfsError> {
        match Remote::locate(path) {
            Ok(Location::Local) => Ok(Route::Local),
            Ok(Location::Remote(_)) => Ok(Route::Remote),
            Ok(Location::Beyond) => Err(VfsError::NotFound(path.to_path_buf())),
            Err(error) => Err(bad_address(path, &error)),
        }
    }

    fn pair(a: &Path, b: &Path) -> Result<Pair, VfsError> {
        let locate = |path: &Path| match Remote::locate(path) {
            Ok(Location::Beyond) => Err(VfsError::NotFound(path.to_path_buf())),
            Ok(location) => Ok(location),
            Err(error) => Err(bad_address(path, &error)),
        };
        match (locate(a)?, locate(b)?) {
            (Location::Local, Location::Local) => Ok(Pair::Local),
            (Location::Remote(a), Location::Remote(b)) if a.target() == b.target() => {
                Ok(Pair::SameMachine)
            }
            _ => Ok(Pair::Apart),
        }
    }

    /// What a move between two machines reports, so the caller falls back to copy and delete.
    fn apart(from: &Path) -> VfsError {
        VfsError::Io {
            path: from.to_path_buf(),
            source: io::Error::new(io::ErrorKind::CrossesDevices, "not on the same machine"),
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

    /// Copies a file between the local disk and a machine, or between two machines, by reading
    /// it here and writing it there.
    fn copy_across(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        if self.symlink_metadata(dst).is_ok() {
            return Err(VfsError::AlreadyExists(dst.to_path_buf()));
        }
        let mut from = self.open_read(src)?;
        let mut to = self.create_write(dst)?;
        let copied = io::copy(&mut from, &mut to).and_then(|_| to.flush());
        drop(to);
        if let Err(source) = copied {
            // A half-written copy would otherwise look like the real file.
            let _ = self.remove_file(dst);
            return Err(VfsError::Io {
                path: dst.to_path_buf(),
                source,
            });
        }
        if let Some(mode) = self.metadata(src).ok().and_then(|meta| meta.mode) {
            // Best effort: the other side may have no permissions to set.
            let _ = self.set_mode(dst, mode & 0o7777);
        }
        Ok(())
    }
}

impl Vfs for SshVfs {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.list_dir(path),
            Route::Remote => {
                let entries: Vec<WireEntry> = self.ask(
                    path,
                    &VfsRequest::List {
                        path: path.to_path_buf(),
                    },
                )?;
                Ok(entries.into_iter().map(entry_of).collect())
            }
        }
    }

    fn is_dir(&self, path: &Path) -> bool {
        match Self::route(path) {
            Ok(Route::Local) => self.inner.is_dir(path),
            Ok(Route::Remote) => self
                .describe(path, true)
                .is_ok_and(|meta| meta.kind == FileKind::Dir),
            Err(_) => false,
        }
    }

    fn exists(&self, path: &Path) -> bool {
        match Self::route(path) {
            Ok(Route::Local) => self.inner.exists(path),
            Ok(Route::Remote) => self.describe(path, true).is_ok(),
            Err(_) => false,
        }
    }

    fn metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.metadata(path),
            Route::Remote => self.describe(path, true),
        }
    }

    fn symlink_metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.symlink_metadata(path),
            Route::Remote => self.describe(path, false),
        }
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.read_link(path),
            Route::Remote => {
                let link: LinkTarget = self.ask(
                    path,
                    &VfsRequest::ReadLink {
                        path: path.to_path_buf(),
                    },
                )?;
                Ok(link.target)
            }
        }
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn ReadSeek>, VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.open_read(path),
            Route::Remote => {
                let (opened, epoch): (Opened, _) = self.ask_at(
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
        }
    }

    fn scan_dir(&self, path: &Path) -> Result<Vec<ScanEntry>, VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.scan_dir(path),
            Route::Remote => {
                let entries: Vec<WireScan> = self.ask(
                    path,
                    &VfsRequest::Scan {
                        path: path.to_path_buf(),
                    },
                )?;
                Ok(entries.into_iter().map(scan_of).collect())
            }
        }
    }

    fn local_path(&self, path: &Path) -> Option<PathBuf> {
        match Self::route(path) {
            Ok(Route::Local) => self.inner.local_path(path),
            _ => None,
        }
    }

    fn resolve_typed(&self, typed: &Path, current: &Path) -> PathBuf {
        let Ok(Location::Remote(remote)) = Remote::locate(typed) else {
            // Not a machine's address, so the ordinary rule: from `current` unless absolute.
            return if typed.is_absolute() {
                typed.to_path_buf()
            } else {
                current.join(typed)
            };
        };
        // `ssh://host` names the machine; the folder to open is the login's home. A path under
        // it, or an explicit `ssh://host/`, is taken as written.
        let names_only_a_machine = typed
            .to_str()
            .and_then(|text| text.split_once("://"))
            .is_some_and(|(_, rest)| !rest.contains('/'));
        if names_only_a_machine
            && let Ok(home) = self.ask::<Resolved>(
                typed,
                &VfsRequest::Home {
                    path: remote.to_path_buf(),
                },
            )
        {
            return home.path;
        }
        typed.to_path_buf()
    }

    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteSeek>, VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.create_write(path),
            Route::Remote => {
                let (opened, epoch): (Opened, _) = self.ask_at(
                    path,
                    &VfsRequest::Create {
                        path: path.to_path_buf(),
                    },
                    None,
                )?;
                Ok(Box::new(PluginWriter::new(
                    Arc::clone(&self.peer),
                    path.to_path_buf(),
                    opened,
                    epoch,
                )))
            }
        }
    }

    fn create_symlink(&self, target: &Path, link: &Path) -> Result<(), VfsError> {
        match Self::route(link)? {
            Route::Local => self.inner.create_symlink(target, link),
            Route::Remote => self.command(
                link,
                &VfsRequest::Symlink {
                    target: target.to_path_buf(),
                    link: link.to_path_buf(),
                },
            ),
        }
    }

    fn set_mode(&self, path: &Path, mode: u32) -> Result<(), VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.set_mode(path, mode),
            Route::Remote => self.command(
                path,
                &VfsRequest::SetMode {
                    path: path.to_path_buf(),
                    mode,
                },
            ),
        }
    }

    fn replace(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        match Self::pair(src, dst)? {
            Pair::Local => self.inner.replace(src, dst),
            Pair::SameMachine => self.command(
                src,
                &VfsRequest::Replace {
                    from: src.to_path_buf(),
                    to: dst.to_path_buf(),
                },
            ),
            Pair::Apart => Err(Self::apart(src)),
        }
    }

    fn create_dir(&self, path: &Path) -> Result<(), VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.create_dir(path),
            Route::Remote => self.command(
                path,
                &VfsRequest::CreateDir {
                    path: path.to_path_buf(),
                },
            ),
        }
    }

    fn create_file(&self, path: &Path) -> Result<(), VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.create_file(path),
            Route::Remote => self.command(
                path,
                &VfsRequest::CreateFile {
                    path: path.to_path_buf(),
                },
            ),
        }
    }

    fn touch(&self, path: &Path) -> Result<(), VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.touch(path),
            Route::Remote => self.command(
                path,
                &VfsRequest::Touch {
                    path: path.to_path_buf(),
                },
            ),
        }
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        match Self::pair(src, dst)? {
            Pair::Local => self.inner.copy_file(src, dst),
            Pair::SameMachine => self.command(
                src,
                &VfsRequest::CopyFile {
                    from: src.to_path_buf(),
                    to: dst.to_path_buf(),
                },
            ),
            Pair::Apart => self.copy_across(src, dst),
        }
    }

    fn rename(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        match Self::pair(src, dst)? {
            Pair::Local => self.inner.rename(src, dst),
            Pair::SameMachine => self.command(
                src,
                &VfsRequest::Rename {
                    from: src.to_path_buf(),
                    to: dst.to_path_buf(),
                },
            ),
            Pair::Apart => Err(Self::apart(src)),
        }
    }

    fn remove_file(&self, path: &Path) -> Result<(), VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.remove_file(path),
            Route::Remote => self.command(
                path,
                &VfsRequest::RemoveFile {
                    path: path.to_path_buf(),
                },
            ),
        }
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), VfsError> {
        match Self::route(path)? {
            Route::Local => self.inner.remove_dir_all(path),
            Route::Remote => self.command(
                path,
                &VfsRequest::RemoveDirAll {
                    path: path.to_path_buf(),
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests;
