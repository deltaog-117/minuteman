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

//! File contents over SFTP as the blocking `Read`/`Write`/`Seek` streams the `Vfs` trait hands out.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::pin::Pin;
use std::sync::Arc;

use openssh_sftp_client::file::TokioCompatFile;
use shared::{ReadSeek, WriteSeek};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::executor::Executor;

/// The most a single `read` asks the server for; `Read` callers pass anything from a few bytes to
/// megabytes, and one request that large would only make the first byte wait for the rest.
const READ_CHUNK: usize = 256 * 1024;

fn gone() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "the SSH connection was shut down",
    )
}

/// Turns an end-relative seek into an absolute one, which is all the SFTP client can do.
fn from_end(len: u64, offset: i64) -> io::Result<SeekFrom> {
    len.checked_add_signed(offset)
        .map(SeekFrom::Start)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))
}

/// An open remote file. The SFTP client's file cannot move once it is being waited on.
pub(crate) type Pinned = Pin<Box<TokioCompatFile>>;

/// A remote file opened for reading.
pub(crate) struct RemoteReader {
    executor: Arc<Executor>,
    // Taken while a request is in flight, so a lost runtime leaves a reader that only errors.
    file: Option<Pinned>,
    len: u64,
}

impl RemoteReader {
    pub(crate) fn new(executor: Arc<Executor>, file: Pinned, len: u64) -> Self {
        Self {
            executor,
            file: Some(file),
            len,
        }
    }

    pub(crate) fn boxed(self) -> Box<dyn ReadSeek> {
        Box::new(self)
    }
}

impl Read for RemoteReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut file = self.file.take().ok_or_else(gone)?;
        let want = buf.len().min(READ_CHUNK);
        let (file, chunk, outcome) = self
            .executor
            .run(async move {
                let mut chunk = vec![0; want];
                let outcome = file.read(&mut chunk).await;
                (file, chunk, outcome)
            })
            .map_err(|_| gone())?;
        self.file = Some(file);
        let got = outcome?;
        buf[..got].copy_from_slice(&chunk[..got]);
        Ok(got)
    }
}

impl Seek for RemoteReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let to = match to {
            SeekFrom::End(offset) => from_end(self.len, offset)?,
            other => other,
        };
        let mut file = self.file.take().ok_or_else(gone)?;
        let (file, outcome) = self
            .executor
            .run(async move {
                let outcome = file.seek(to).await;
                (file, outcome)
            })
            .map_err(|_| gone())?;
        self.file = Some(file);
        outcome
    }
}

/// A remote file created for writing.
pub(crate) struct RemoteWriter {
    executor: Arc<Executor>,
    file: Option<Pinned>,
    // The file is created empty by this writer and written only through it, so these two say
    // where the cursor is and how long the file is without asking the server, which will not
    // describe a handle opened for writing.
    position: u64,
    len: u64,
}

impl RemoteWriter {
    pub(crate) fn new(executor: Arc<Executor>, file: Pinned) -> Self {
        Self {
            executor,
            file: Some(file),
            position: 0,
            len: 0,
        }
    }

    pub(crate) fn boxed(self) -> Box<dyn WriteSeek> {
        Box::new(self)
    }
}

impl Write for RemoteWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut file = self.file.take().ok_or_else(gone)?;
        let data = buf.to_vec();
        let (file, outcome) = self
            .executor
            .run(async move {
                let outcome = file.write(&data).await;
                (file, outcome)
            })
            .map_err(|_| gone())?;
        self.file = Some(file);
        let written = outcome?;
        self.position += written as u64;
        self.len = self.len.max(self.position);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut file = self.file.take().ok_or_else(gone)?;
        let (file, outcome) = self
            .executor
            .run(async move {
                let outcome = file.flush().await;
                (file, outcome)
            })
            .map_err(|_| gone())?;
        self.file = Some(file);
        outcome
    }
}

impl Seek for RemoteWriter {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let to = match to {
            SeekFrom::End(offset) => from_end(self.len, offset)?,
            other => other,
        };
        let mut file = self.file.take().ok_or_else(gone)?;
        let (file, outcome) = self
            .executor
            .run(async move {
                let outcome = file.seek(to).await;
                (file, outcome)
            })
            .map_err(|_| gone())?;
        self.file = Some(file);
        self.position = outcome?;
        Ok(self.position)
    }
}

impl Drop for RemoteWriter {
    /// A writer the caller lets go of without a `flush` still reaches the server whole, as a local
    /// file does; there is no one left to tell if this last attempt fails.
    fn drop(&mut self) {
        if let Some(mut file) = self.file.take() {
            let _ = self.executor.run(async move {
                let _ = file.flush().await;
                let _ = file.shutdown().await;
            });
        }
    }
}
