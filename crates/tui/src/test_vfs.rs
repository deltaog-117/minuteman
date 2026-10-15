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

//! A stand-in for a folder on another machine, for the tests of the jobs that read one in the
//! background.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use shared::{DirEntryInfo, LocalVfs, Vfs, VfsError, cancel};

/// A `Vfs` whose listings take `delay` and are counted. Like a request to a remote machine, a
/// listing gives up as soon as the job that asked is cancelled, and counts that too.
pub struct Slow {
    inner: LocalVfs,
    delay: Duration,
    pub lists: AtomicUsize,
    pub gave_up: AtomicUsize,
}

impl Slow {
    pub fn new(delay: Duration) -> Self {
        Self {
            inner: LocalVfs,
            delay,
            lists: AtomicUsize::new(0),
            gave_up: AtomicUsize::new(0),
        }
    }

    /// Waits until `counter` reaches `wanted` or five seconds pass, returning whether it did.
    pub fn reaches(counter: &AtomicUsize, wanted: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while counter.load(Ordering::SeqCst) < wanted && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        counter.load(Ordering::SeqCst) >= wanted
    }
}

impl Vfs for Slow {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        let until = Instant::now() + self.delay;
        while Instant::now() < until {
            if cancel::cancelled() {
                self.gave_up.fetch_add(1, Ordering::SeqCst);
                return Err(VfsError::Io {
                    path: path.to_path_buf(),
                    source: io::Error::new(io::ErrorKind::Interrupted, "cancelled"),
                });
            }
            std::thread::sleep(Duration::from_millis(5).min(self.delay));
        }
        self.inner.list_dir(path)
    }
    fn metadata(&self, path: &Path) -> Result<shared::Metadata, VfsError> {
        self.inner.metadata(path)
    }
    fn symlink_metadata(&self, path: &Path) -> Result<shared::Metadata, VfsError> {
        self.inner.symlink_metadata(path)
    }
    fn read_link(&self, _: &Path) -> Result<PathBuf, VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn open_read(&self, _: &Path) -> Result<Box<dyn shared::ReadSeek>, VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn scan_dir(&self, _: &Path) -> Result<Vec<shared::ScanEntry>, VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn local_path(&self, _: &Path) -> Option<PathBuf> {
        None
    }
    fn create_write(&self, _: &Path) -> Result<Box<dyn shared::WriteSeek>, VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn create_symlink(&self, _: &Path, _: &Path) -> Result<(), VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn set_mode(&self, _: &Path, _: u32) -> Result<(), VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn replace(&self, _: &Path, _: &Path) -> Result<(), VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn is_dir(&self, path: &Path) -> bool {
        self.inner.is_dir(path)
    }
    fn exists(&self, path: &Path) -> bool {
        self.inner.exists(path)
    }
    fn create_dir(&self, _: &Path) -> Result<(), VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn create_file(&self, _: &Path) -> Result<(), VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn touch(&self, _: &Path) -> Result<(), VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn copy_file(&self, _: &Path, _: &Path) -> Result<(), VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn rename(&self, _: &Path, _: &Path) -> Result<(), VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn remove_file(&self, _: &Path) -> Result<(), VfsError> {
        unimplemented!("the jobs under test only list")
    }
    fn remove_dir_all(&self, _: &Path) -> Result<(), VfsError> {
        unimplemented!("the jobs under test only list")
    }
}
