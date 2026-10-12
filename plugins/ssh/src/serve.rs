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

//! Answering the host's questions about paths on remote machines: each [`VfsRequest`] is run
//! against the session with the machine its path names and turned into the reply the wire carries.
//!
//! A file is opened once and read or written in pieces through a handle. A handle is either a
//! file being read or a new file being written, never both, and the table holding them is bounded
//! by [`MAX_OPEN_HANDLES`] so a host that leaks handles is refused instead of growing the plugin.

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::UNIX_EPOCH;

use plugin_api::{
    Bytes, IoKind, LinkTarget, MAX_OPEN_HANDLES, MAX_VFS_READ, Opened, Resolved, VfsFault,
    VfsRequest, WireEntry, WireKind, WireMeta, WireScan,
};
use serde::Serialize;
use serde_json::Value;
use shared::{FileKind, Metadata, ReadSeek, VfsError, WriteSeek};

use crate::link::ConnectOptions;
use crate::machines::Machines;

/// A file the host has open. Its own lock lets requests for different files run at the same time.
enum Open {
    Reading(Box<dyn ReadSeek>),
    Writing(Box<dyn WriteSeek>),
}

type OpenFile = Arc<Mutex<Open>>;

#[derive(Default)]
struct Handles {
    /// The number the next open gets. Never reused within a process, so a handle that was closed
    /// cannot come to mean another file.
    next: u64,
    open: HashMap<u64, OpenFile>,
}

pub struct Server {
    machines: Machines,
    handles: Mutex<Handles>,
}

/// What a request fails with, as the wire spells it.
pub fn fault_of(error: &VfsError) -> VfsFault {
    match error {
        VfsError::NotFound(_) => VfsFault::NotFound,
        VfsError::NotADirectory(_) => VfsFault::NotADirectory,
        VfsError::AlreadyExists(_) => VfsFault::AlreadyExists,
        VfsError::Unsupported(reason) => VfsFault::Unsupported {
            reason: (*reason).to_string(),
        },
        VfsError::Io { source, .. } => io_fault(source),
    }
}

fn io_fault(error: &io::Error) -> VfsFault {
    VfsFault::Io {
        kind: IoKind::from(error.kind()),
        message: error.to_string(),
    }
}

fn seconds(time: Option<std::time::SystemTime>) -> Option<u64> {
    time.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
}

fn meta_of(meta: &Metadata) -> WireMeta {
    WireMeta {
        kind: match meta.kind {
            FileKind::File => WireKind::File,
            FileKind::Dir => WireKind::Dir,
            FileKind::Symlink => WireKind::Symlink,
            FileKind::Other => WireKind::Other,
        },
        len: meta.len,
        mode: meta.mode,
        modified: seconds(meta.modified),
    }
}

fn reply(value: &impl Serialize) -> Result<Value, VfsFault> {
    serde_json::to_value(value).map_err(|e| VfsFault::Io {
        kind: IoKind::Other,
        message: format!("could not write the reply: {e}"),
    })
}

fn nothing() -> Result<Value, VfsFault> {
    Ok(Value::Object(serde_json::Map::new()))
}

/// Runs a request on a `Link` and turns its failure into the wire's.
fn done<T>(result: Result<T, VfsError>) -> Result<T, VfsFault> {
    result.map_err(|e| fault_of(&e))
}

impl Server {
    /// A server that connects to machines the way `options` say.
    ///
    /// # Errors
    ///
    /// If the worker threads for the SSH sessions cannot be started.
    pub fn new(options: ConnectOptions) -> io::Result<Self> {
        Ok(Self {
            machines: Machines::new(options)?,
            handles: Mutex::new(Handles::default()),
        })
    }

    fn handles(&self) -> MutexGuard<'_, Handles> {
        // A poisoned table is still a table of files that are open or not.
        self.handles.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The reply to `request`.
    ///
    /// # Errors
    ///
    /// Why it cannot be answered, in the words the wire carries.
    pub fn answer(&self, request: &VfsRequest) -> Result<Value, VfsFault> {
        match request {
            VfsRequest::Metadata { path, follow } => {
                let (link, remote) = self.machines.route(path)?;
                let meta = if *follow {
                    link.metadata(&remote)
                } else {
                    link.symlink_metadata(&remote)
                };
                reply(&meta_of(&done(meta)?))
            }
            VfsRequest::List { path } => {
                let (link, remote) = self.machines.route(path)?;
                let entries = done(link.list_dir(&remote))?;
                reply(&entries.iter().map(WireEntry::from).collect::<Vec<_>>())
            }
            VfsRequest::Scan { path } => {
                let (link, remote) = self.machines.route(path)?;
                let entries = done(link.scan_dir(&remote))?;
                reply(
                    &entries
                        .into_iter()
                        .map(|entry| WireScan {
                            name: entry.name,
                            path: entry.path,
                            meta: entry.metadata.as_ref().map(meta_of),
                        })
                        .collect::<Vec<_>>(),
                )
            }
            VfsRequest::ReadLink { path } => {
                let (link, remote) = self.machines.route(path)?;
                reply(&LinkTarget {
                    target: done(link.read_link(&remote))?,
                })
            }
            VfsRequest::Home { path } => {
                let (link, remote) = self.machines.route(path)?;
                let home = done(link.home())?;
                reply(&Resolved {
                    path: remote.at(home).to_path_buf(),
                })
            }
            VfsRequest::Open { path } => self.open(path),
            VfsRequest::Create { path } => self.create(path),
            VfsRequest::Read {
                handle,
                offset,
                len,
            } => self.read(*handle, *offset, *len),
            VfsRequest::Write {
                handle,
                offset,
                data,
            } => self.write(*handle, *offset, data),
            VfsRequest::Flush { handle } => self.flush(*handle),
            VfsRequest::Close { handle } => {
                // Taken out of the table first so the file's drop (which finishes a write) does
                // not run with the table locked.
                let closed = self.handles().open.remove(handle);
                drop(closed);
                nothing()
            }
            VfsRequest::CreateDir { path } => {
                let (link, remote) = self.machines.route(path)?;
                done(link.create_dir(&remote))?;
                nothing()
            }
            VfsRequest::CreateFile { path } => {
                let (link, remote) = self.machines.route(path)?;
                done(link.create_file(&remote))?;
                nothing()
            }
            VfsRequest::Touch { path } => {
                let (link, remote) = self.machines.route(path)?;
                done(link.touch(&remote))?;
                nothing()
            }
            VfsRequest::SetMode { path, mode } => {
                let (link, remote) = self.machines.route(path)?;
                done(link.set_mode(&remote, *mode))?;
                nothing()
            }
            VfsRequest::Symlink { target, link } => {
                let (session, remote) = self.machines.route(link)?;
                done(session.create_symlink(target, &remote))?;
                nothing()
            }
            VfsRequest::Rename { from, to } => {
                let (link, from, to) = self.machines.route_pair(from, to)?;
                done(link.rename(&from, &to))?;
                nothing()
            }
            VfsRequest::Replace { from, to } => {
                let (link, from, to) = self.machines.route_pair(from, to)?;
                done(link.replace(&from, &to))?;
                nothing()
            }
            VfsRequest::CopyFile { from, to } => {
                let (link, from, to) = self.machines.route_pair(from, to)?;
                done(link.copy_file(&from, &to))?;
                nothing()
            }
            VfsRequest::RemoveFile { path } => {
                let (link, remote) = self.machines.route(path)?;
                done(link.remove_file(&remote))?;
                nothing()
            }
            VfsRequest::RemoveDirAll { path } => {
                let (link, remote) = self.machines.route(path)?;
                done(link.remove_dir_all(&remote))?;
                nothing()
            }
        }
    }

    /// Keeps `file` under a new number, unless the table is full.
    fn keep(&self, file: Open) -> Result<u64, VfsFault> {
        let mut handles = self.handles();
        if handles.open.len() >= MAX_OPEN_HANDLES {
            return Err(VfsFault::TooManyOpen);
        }
        handles.next += 1;
        let handle = handles.next;
        handles.open.insert(handle, Arc::new(Mutex::new(file)));
        Ok(handle)
    }

    fn full(&self) -> bool {
        self.handles().open.len() >= MAX_OPEN_HANDLES
    }

    fn open(&self, path: &Path) -> Result<Value, VfsFault> {
        // Refuse before connecting to anything: a full table cannot take the file anyway.
        if self.full() {
            return Err(VfsFault::TooManyOpen);
        }
        let (link, remote) = self.machines.route(path)?;
        let mut file = done(link.open_read(&remote))?;
        let len = file.seek(SeekFrom::End(0)).map_err(|e| io_fault(&e))?;
        file.seek(SeekFrom::Start(0)).map_err(|e| io_fault(&e))?;
        let handle = self.keep(Open::Reading(file))?;
        reply(&Opened { handle, len })
    }

    fn create(&self, path: &Path) -> Result<Value, VfsFault> {
        if self.full() {
            return Err(VfsFault::TooManyOpen);
        }
        let (link, remote) = self.machines.route(path)?;
        let file = done(link.create_write(&remote))?;
        let handle = self.keep(Open::Writing(file))?;
        reply(&Opened { handle, len: 0 })
    }

    fn file(&self, handle: u64) -> Result<OpenFile, VfsFault> {
        self.handles()
            .open
            .get(&handle)
            .cloned()
            .ok_or(VfsFault::BadHandle)
    }

    fn read(&self, handle: u64, offset: u64, len: u64) -> Result<Value, VfsFault> {
        let file = self.file(handle)?;
        let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
        let Open::Reading(file) = &mut *file else {
            return Err(VfsFault::BadHandle);
        };
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| io_fault(&e))?;
        let mut data = Vec::new();
        file.by_ref()
            .take(len.min(MAX_VFS_READ))
            .read_to_end(&mut data)
            .map_err(|e| io_fault(&e))?;
        reply(&Bytes { data })
    }

    fn write(&self, handle: u64, offset: u64, data: &[u8]) -> Result<Value, VfsFault> {
        if data.len() as u64 > MAX_VFS_READ {
            return Err(VfsFault::Io {
                kind: IoKind::InvalidInput,
                message: format!("a write is at most {MAX_VFS_READ} bytes"),
            });
        }
        let file = self.file(handle)?;
        let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
        let Open::Writing(file) = &mut *file else {
            return Err(VfsFault::BadHandle);
        };
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| io_fault(&e))?;
        file.write_all(data).map_err(|e| io_fault(&e))?;
        nothing()
    }

    fn flush(&self, handle: u64) -> Result<Value, VfsFault> {
        let file = self.file(handle)?;
        let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
        let Open::Writing(file) = &mut *file else {
            return Err(VfsFault::BadHandle);
        };
        file.flush().map_err(|e| io_fault(&e))?;
        nothing()
    }

    /// How many files are open, for the tests.
    pub fn open_count(&self) -> usize {
        self.handles().open.len()
    }
}
