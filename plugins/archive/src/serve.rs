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

//! Answering the host's questions about the inside of an archive: each [`VfsRequest`] is run
//! against the [`ArchiveVfs`] engine and turned into the reply the wire carries. The engine reads
//! the archive file itself back through the host, so a remote archive works the same as a local
//! one; this module only decides what a request means and keeps the files the host has open.
//!
//! A file is opened once and read in pieces through a handle, because a zip or gzip entry cannot
//! seek and is unpacked whole. The handle table is bounded by [`MAX_OPEN_HANDLES`], since each
//! open file can hold an unpacked entry in memory.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::UNIX_EPOCH;

use plugin_api::{
    Bytes, IoKind, LinkTarget, MAX_OPEN_HANDLES, MAX_VFS_READ, Opened, VfsFault, VfsRequest,
    WireEntry, WireKind, WireMeta, WireScan,
};
use serde::Serialize;
use serde_json::Value;
use shared::{FileKind, Metadata, ReadSeek, Vfs, VfsError};

use crate::cache::DiskCache;
use crate::engine::ArchiveVfs;

/// One open file. Its own lock lets reads of different files run at the same time.
type OpenFile = Arc<Mutex<Box<dyn ReadSeek>>>;

#[derive(Default)]
struct Handles {
    /// The number the next `open` gets. Never reused within a process, so a handle that was
    /// closed cannot come to mean another file.
    next: u64,
    open: HashMap<u64, OpenFile>,
}

pub struct Server {
    engine: ArchiveVfs,
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
        VfsError::Io { source, .. } => VfsFault::Io {
            kind: IoKind::from(source.kind()),
            message: source.to_string(),
        },
    }
}

fn io_fault(error: &std::io::Error) -> VfsFault {
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

impl Server {
    /// A server for the archives that `inner` (the host's files) holds.
    pub fn new(inner: Arc<dyn Vfs>) -> Self {
        Self {
            engine: ArchiveVfs::new(inner).with_disk_cache(DiskCache::in_user_cache()),
            handles: Mutex::new(Handles::default()),
        }
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
                let meta = if *follow {
                    self.engine.metadata(path)
                } else {
                    self.engine.symlink_metadata(path)
                }
                .map_err(|e| fault_of(&e))?;
                reply(&meta_of(&meta))
            }
            VfsRequest::List { path } => {
                let entries = self.engine.list_dir(path).map_err(|e| fault_of(&e))?;
                reply(&entries.iter().map(WireEntry::from).collect::<Vec<_>>())
            }
            VfsRequest::Scan { path } => {
                let entries = self.engine.scan_dir(path).map_err(|e| fault_of(&e))?;
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
                let target = self.engine.read_link(path).map_err(|e| fault_of(&e))?;
                reply(&LinkTarget { target })
            }
            VfsRequest::Open { path } => self.open(path),
            VfsRequest::Read {
                handle,
                offset,
                len,
            } => self.read(*handle, *offset, *len),
            VfsRequest::Close { handle } => {
                self.handles().open.remove(handle);
                Ok(Value::Object(serde_json::Map::new()))
            }
            // An archive is read-only and the host never sends these here; they exist for the
            // plugin that serves remote machines.
            VfsRequest::Home { .. }
            | VfsRequest::Create { .. }
            | VfsRequest::Write { .. }
            | VfsRequest::Flush { .. }
            | VfsRequest::CreateDir { .. }
            | VfsRequest::CreateFile { .. }
            | VfsRequest::Touch { .. }
            | VfsRequest::SetMode { .. }
            | VfsRequest::Symlink { .. }
            | VfsRequest::Rename { .. }
            | VfsRequest::Replace { .. }
            | VfsRequest::CopyFile { .. }
            | VfsRequest::RemoveFile { .. }
            | VfsRequest::RemoveDirAll { .. } => Err(VfsFault::Unsupported {
                reason: "an archive is read-only".to_string(),
            }),
        }
    }

    fn open(&self, path: &std::path::Path) -> Result<Value, VfsFault> {
        // Refuse before unpacking anything: an entry can be hundreds of megabytes.
        if self.handles().open.len() >= MAX_OPEN_HANDLES {
            return Err(VfsFault::TooManyOpen);
        }
        let mut file = self.engine.open_read(path).map_err(|e| fault_of(&e))?;
        let len = file.seek(SeekFrom::End(0)).map_err(|e| io_fault(&e))?;
        file.seek(SeekFrom::Start(0)).map_err(|e| io_fault(&e))?;

        let mut handles = self.handles();
        // Checked again: other requests may have opened files while this one unpacked.
        if handles.open.len() >= MAX_OPEN_HANDLES {
            return Err(VfsFault::TooManyOpen);
        }
        handles.next += 1;
        let handle = handles.next;
        handles.open.insert(handle, Arc::new(Mutex::new(file)));
        reply(&Opened { handle, len })
    }

    fn read(&self, handle: u64, offset: u64, len: u64) -> Result<Value, VfsFault> {
        let file = self
            .handles()
            .open
            .get(&handle)
            .cloned()
            .ok_or(VfsFault::BadHandle)?;
        let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| io_fault(&e))?;
        let mut data = Vec::new();
        file.by_ref()
            .take(len.min(MAX_VFS_READ))
            .read_to_end(&mut data)
            .map_err(|e| io_fault(&e))?;
        reply(&Bytes { data })
    }

    /// How many files are open, for the tests.
    #[cfg(test)]
    fn open_count(&self) -> usize {
        self.handles().open.len()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use proptest::prelude::*;
    use shared::MemVfs;

    use super::*;

    fn zip_bytes(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn server_with(files: &[(&str, Vec<u8>)]) -> Server {
        let mem = Arc::new(MemVfs::new());
        mem.add_dir("/x");
        mem.add_file("/x/a.zip", zip_bytes(files));
        Server::new(mem)
    }

    fn root() -> PathBuf {
        PathBuf::from("/x/a.zip!")
    }

    fn ask<T: serde::de::DeserializeOwned>(
        server: &Server,
        request: VfsRequest,
    ) -> Result<T, VfsFault> {
        server
            .answer(&request)
            .map(|value| serde_json::from_value(value).expect("the reply has the shape asked"))
    }

    fn open(server: &Server, path: &Path) -> Result<Opened, VfsFault> {
        ask(
            server,
            VfsRequest::Open {
                path: path.to_path_buf(),
            },
        )
    }

    fn read(server: &Server, handle: u64, offset: u64, len: u64) -> Result<Vec<u8>, VfsFault> {
        ask::<Bytes>(
            server,
            VfsRequest::Read {
                handle,
                offset,
                len,
            },
        )
        .map(|bytes| bytes.data)
    }

    #[test]
    fn a_folder_is_listed_and_described_from_the_wire() {
        let server = server_with(&[("d/f.txt", b"hello".to_vec())]);
        let listed: Vec<WireEntry> = ask(&server, VfsRequest::List { path: root() }).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].is_dir);
        assert_eq!(listed[0].path, root().join("d"));

        let meta: WireMeta = ask(
            &server,
            VfsRequest::Metadata {
                path: root().join("d/f.txt"),
                follow: true,
            },
        )
        .unwrap();
        assert_eq!((meta.kind, meta.len), (WireKind::File, 5));

        let scanned: Vec<WireScan> = ask(
            &server,
            VfsRequest::Scan {
                path: root().join("d"),
            },
        )
        .unwrap();
        assert_eq!(scanned[0].name, "f.txt");
        assert_eq!(scanned[0].meta.as_ref().map(|m| m.len), Some(5));
    }

    #[test]
    fn failures_keep_their_meaning() {
        let server = server_with(&[("f.txt", b"x".to_vec())]);
        let list = |path: PathBuf| server.answer(&VfsRequest::List { path });
        assert_eq!(list(root().join("nope")), Err(VfsFault::NotFound));
        assert_eq!(list(root().join("f.txt")), Err(VfsFault::NotADirectory));
        assert!(matches!(
            list(PathBuf::from("/x/not-there.zip!")),
            Err(VfsFault::NotFound)
        ));
        let damaged = Arc::new(MemVfs::new());
        damaged.add_file("/x/a.zip", b"not an archive at all".repeat(30));
        let server = Server::new(damaged);
        assert!(matches!(
            server.answer(&VfsRequest::List { path: root() }),
            Err(VfsFault::Io { .. })
        ));
    }

    #[test]
    fn a_file_is_read_in_pieces_by_handle_and_closed() {
        let body: Vec<u8> = (0..=255u8).cycle().take(10_000).collect();
        let server = server_with(&[("big.bin", body.clone())]);
        let opened = open(&server, &root().join("big.bin")).unwrap();
        assert_eq!(opened.len, 10_000);
        assert_eq!(read(&server, opened.handle, 0, 100).unwrap(), body[..100]);
        assert_eq!(
            read(&server, opened.handle, 9_990, 100).unwrap(),
            body[9_990..],
            "the end of the file is short, not an error"
        );
        assert_eq!(read(&server, opened.handle, 20_000, 10).unwrap(), b"");

        assert!(
            server
                .answer(&VfsRequest::Close {
                    handle: opened.handle
                })
                .is_ok()
        );
        assert_eq!(
            read(&server, opened.handle, 0, 1),
            Err(VfsFault::BadHandle),
            "a closed handle reads nothing"
        );
        assert!(
            server
                .answer(&VfsRequest::Close {
                    handle: opened.handle
                })
                .is_ok(),
            "closing twice is not an error"
        );
    }

    #[test]
    fn one_read_never_returns_more_than_the_cap() {
        let body = vec![7u8; (MAX_VFS_READ as usize) * 2];
        let server = server_with(&[("big.bin", body)]);
        let opened = open(&server, &root().join("big.bin")).unwrap();
        let got = read(&server, opened.handle, 0, u64::MAX).unwrap();
        assert_eq!(got.len() as u64, MAX_VFS_READ);
    }

    #[test]
    fn handles_are_numbered_once_and_never_reused() {
        let server = server_with(&[("f", b"1".to_vec())]);
        let first = open(&server, &root().join("f")).unwrap().handle;
        server.answer(&VfsRequest::Close { handle: first }).unwrap();
        let second = open(&server, &root().join("f")).unwrap().handle;
        assert_ne!(first, second);
        assert_eq!(read(&server, first, 0, 1), Err(VfsFault::BadHandle));
    }

    #[test]
    fn too_many_open_files_are_refused_until_one_is_closed() {
        let server = server_with(&[("f", b"1".to_vec())]);
        let path = root().join("f");
        let handles: Vec<u64> = (0..MAX_OPEN_HANDLES)
            .map(|_| open(&server, &path).unwrap().handle)
            .collect();
        assert_eq!(open(&server, &path), Err(VfsFault::TooManyOpen));
        assert_eq!(server.open_count(), MAX_OPEN_HANDLES);
        server
            .answer(&VfsRequest::Close { handle: handles[0] })
            .unwrap();
        assert!(open(&server, &path).is_ok());
    }

    #[test]
    fn a_folder_cannot_be_opened() {
        let server = server_with(&[("d/f", b"1".to_vec())]);
        assert!(matches!(
            open(&server, &root().join("d")),
            Err(VfsFault::Io { .. })
        ));
        assert_eq!(server.open_count(), 0);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// However a file is read (any offsets and lengths), the pieces are the bytes of the
        /// same range of the file, and none is longer than the cap.
        #[test]
        fn reads_match_the_file_for_any_range(
            body in proptest::collection::vec(any::<u8>(), 0..3000),
            ranges in proptest::collection::vec((0u64..3500, 0u64..4000), 1..10),
        ) {
            let server = server_with(&[("f.bin", body.clone())]);
            let opened = open(&server, &root().join("f.bin")).unwrap();
            prop_assert_eq!(opened.len, body.len() as u64);
            for (offset, len) in ranges {
                let got = read(&server, opened.handle, offset, len).unwrap();
                let start = (offset as usize).min(body.len());
                let end = (start + len as usize).min(body.len());
                prop_assert_eq!(&got[..], &body[start..end]);
            }
        }
    }
}
