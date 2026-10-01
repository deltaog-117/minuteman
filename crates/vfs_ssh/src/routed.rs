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

//! The [`Vfs`] the application is built with: local paths go to the local disk, `ssh://` paths to
//! an SFTP session with that machine, and a copy between the two is streamed through here.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use shared::{DirEntryInfo, Metadata, ReadSeek, ScanEntry, Vfs, VfsError, WriteSeek};

use crate::executor::Executor;
use crate::link::{ConnectOptions, Link};
use crate::target::{Location, Remote, Target, TargetError};

pub struct RoutedVfs {
    local: Arc<dyn Vfs>,
    executor: Arc<Executor>,
    options: ConnectOptions,
    // One session per machine, shared by every path on it and by every thread.
    links: Mutex<HashMap<Target, Arc<Link>>>,
}

/// Where one path goes.
enum Route {
    Local,
    Remote(Arc<Link>, Remote),
}

/// Where two paths go: a move needs both in the same place.
enum Pair {
    Local,
    SameMachine(Arc<Link>, Remote, Remote),
    Apart,
}

fn bad_address(path: &Path, error: &TargetError) -> VfsError {
    VfsError::Io {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidInput, error.to_string()),
    }
}

fn from_io(path: &Path, source: io::Error) -> VfsError {
    VfsError::Io {
        path: path.to_path_buf(),
        source,
    }
}

impl RoutedVfs {
    /// Routes between `local` and remote machines reached with the default connection options.
    ///
    /// # Errors
    ///
    /// If the worker threads for the SSH sessions cannot be started.
    pub fn new(local: Arc<dyn Vfs>) -> io::Result<Self> {
        Self::with_options(local, ConnectOptions::default())
    }

    pub fn with_options(local: Arc<dyn Vfs>, options: ConnectOptions) -> io::Result<Self> {
        Ok(Self {
            local,
            executor: Arc::new(Executor::new()?),
            options,
            links: Mutex::new(HashMap::new()),
        })
    }

    fn link(&self, target: &Target) -> Arc<Link> {
        let mut links = self.links.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(links.entry(target.clone()).or_insert_with(|| {
            Arc::new(Link::new(
                target.clone(),
                self.options.clone(),
                Arc::clone(&self.executor),
            ))
        }))
    }

    fn route(&self, path: &Path) -> Result<Route, VfsError> {
        match Remote::locate(path) {
            Ok(Location::Local) => Ok(Route::Local),
            Ok(Location::Remote(remote)) => Ok(Route::Remote(self.link(remote.target()), remote)),
            Ok(Location::Beyond) => Err(VfsError::NotFound(path.to_path_buf())),
            Err(error) => Err(bad_address(path, &error)),
        }
    }

    fn pair(&self, a: &Path, b: &Path) -> Result<Pair, VfsError> {
        match (self.route(a)?, self.route(b)?) {
            (Route::Local, Route::Local) => Ok(Pair::Local),
            (Route::Remote(link, a), Route::Remote(_, b)) if a.target() == b.target() => {
                Ok(Pair::SameMachine(link, a, b))
            }
            _ => Ok(Pair::Apart),
        }
    }

    /// What a move between two machines reports, so the caller falls back to copy and delete.
    fn apart(from: &Path) -> VfsError {
        from_io(
            from,
            io::Error::new(io::ErrorKind::CrossesDevices, "not on the same machine"),
        )
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
            return Err(from_io(dst, source));
        }
        if let Some(mode) = self.metadata(src).ok().and_then(|meta| meta.mode) {
            // Best effort: the other side may have no permissions to set.
            let _ = self.set_mode(dst, mode & 0o7777);
        }
        Ok(())
    }
}

impl Vfs for RoutedVfs {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError> {
        match self.route(path)? {
            Route::Local => self.local.list_dir(path),
            Route::Remote(link, remote) => link.list_dir(&remote),
        }
    }

    fn is_dir(&self, path: &Path) -> bool {
        match self.route(path) {
            Ok(Route::Local) => self.local.is_dir(path),
            Ok(Route::Remote(link, remote)) => link
                .metadata(&remote)
                .is_ok_and(|m| m.kind == shared::FileKind::Dir),
            Err(_) => false,
        }
    }

    fn exists(&self, path: &Path) -> bool {
        match self.route(path) {
            Ok(Route::Local) => self.local.exists(path),
            Ok(Route::Remote(link, remote)) => link.metadata(&remote).is_ok(),
            Err(_) => false,
        }
    }

    fn metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        match self.route(path)? {
            Route::Local => self.local.metadata(path),
            Route::Remote(link, remote) => link.metadata(&remote),
        }
    }

    fn symlink_metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        match self.route(path)? {
            Route::Local => self.local.symlink_metadata(path),
            Route::Remote(link, remote) => link.symlink_metadata(&remote),
        }
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, VfsError> {
        match self.route(path)? {
            Route::Local => self.local.read_link(path),
            Route::Remote(link, remote) => link.read_link(&remote),
        }
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn ReadSeek>, VfsError> {
        match self.route(path)? {
            Route::Local => self.local.open_read(path),
            Route::Remote(link, remote) => link.open_read(&remote),
        }
    }

    fn scan_dir(&self, path: &Path) -> Result<Vec<ScanEntry>, VfsError> {
        match self.route(path)? {
            Route::Local => self.local.scan_dir(path),
            Route::Remote(link, remote) => link.scan_dir(&remote),
        }
    }

    fn local_path(&self, path: &Path) -> Option<PathBuf> {
        match self.route(path) {
            Ok(Route::Local) => self.local.local_path(path),
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
        if names_only_a_machine && let Ok(home) = self.link(remote.target()).home() {
            return remote.at(home).to_path_buf();
        }
        typed.to_path_buf()
    }

    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteSeek>, VfsError> {
        match self.route(path)? {
            Route::Local => self.local.create_write(path),
            Route::Remote(link, remote) => link.create_write(&remote),
        }
    }

    fn create_symlink(&self, target: &Path, link: &Path) -> Result<(), VfsError> {
        match self.route(link)? {
            Route::Local => self.local.create_symlink(target, link),
            Route::Remote(session, remote) => session.create_symlink(target, &remote),
        }
    }

    fn set_mode(&self, path: &Path, mode: u32) -> Result<(), VfsError> {
        match self.route(path)? {
            Route::Local => self.local.set_mode(path, mode),
            Route::Remote(link, remote) => link.set_mode(&remote, mode),
        }
    }

    fn replace(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        match self.pair(src, dst)? {
            Pair::Local => self.local.replace(src, dst),
            Pair::SameMachine(link, from, to) => link.replace(&from, &to),
            Pair::Apart => Err(Self::apart(src)),
        }
    }

    fn create_dir(&self, path: &Path) -> Result<(), VfsError> {
        match self.route(path)? {
            Route::Local => self.local.create_dir(path),
            Route::Remote(link, remote) => link.create_dir(&remote),
        }
    }

    fn create_file(&self, path: &Path) -> Result<(), VfsError> {
        match self.route(path)? {
            Route::Local => self.local.create_file(path),
            Route::Remote(link, remote) => link.create_file(&remote),
        }
    }

    fn touch(&self, path: &Path) -> Result<(), VfsError> {
        match self.route(path)? {
            Route::Local => self.local.touch(path),
            Route::Remote(link, remote) => link.touch(&remote),
        }
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        match self.pair(src, dst)? {
            Pair::Local => self.local.copy_file(src, dst),
            Pair::SameMachine(link, from, to) => link.copy_file(&from, &to),
            Pair::Apart => self.copy_across(src, dst),
        }
    }

    fn rename(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        match self.pair(src, dst)? {
            Pair::Local => self.local.rename(src, dst),
            Pair::SameMachine(link, from, to) => link.rename(&from, &to),
            Pair::Apart => Err(Self::apart(src)),
        }
    }

    fn remove_file(&self, path: &Path) -> Result<(), VfsError> {
        match self.route(path)? {
            Route::Local => self.local.remove_file(path),
            Route::Remote(link, remote) => link.remove_file(&remote),
        }
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), VfsError> {
        match self.route(path)? {
            Route::Local => self.local.remove_dir_all(path),
            Route::Remote(link, remote) => link.remove_dir_all(&remote),
        }
    }
}
