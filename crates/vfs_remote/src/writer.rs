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

//! A new file on a remote machine, written in windows through the handle the plugin opened for it.
//!
//! A request costs a round trip through another process, and `io::copy` hands over eight
//! kilobytes at a time, so a writer gathers what it is given and sends a window at once: when it
//! is full, when the next byte does not continue where the last ended (a seek), and on `flush`
//! or drop. A write that fails after it was accepted is reported by the next call that reaches
//! the plugin, and certainly by `flush`.
//!
//! A window is not waited for when it is sent: up to [`IN_FLIGHT`] go out before the oldest
//! reply is read, so the plugin is writing one window while the next crosses the pipe, instead of
//! the pair taking turns. Each request names its own offset, so the order the plugin happens to
//! run them in does not change the file; `flush` and drop wait for every one, and a failure of
//! any is reported by the call that reads its reply, at the latest `flush`.

use std::collections::VecDeque;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;

use plugin_api::{Epoch, MAX_VFS_READ, Opened, Peer, PeerError, Reply, VfsRequest};

use crate::error_of;

/// The most one request carries, which is also how much a writer holds back.
const WINDOW: usize = MAX_VFS_READ as usize;

/// How many windows may be on their way at once. Enough to cover the round trip of one window
/// (about 7 ms against about 11 ms to write it), and a bound on the memory the plugin is handed.
const IN_FLIGHT: usize = 4;

pub(crate) struct PluginWriter {
    peer: Arc<dyn Peer>,
    path: PathBuf,
    handle: u64,
    /// The life of the plugin's process the handle belongs to; see `PluginReader`.
    epoch: Epoch,
    pos: u64,
    /// The end of the furthest byte written, for an end-relative seek.
    len: u64,
    /// What has been written and not yet sent, starting at `pending_start` in the file.
    pending: Vec<u8>,
    pending_start: u64,
    /// Windows sent whose reply has not been read, oldest first.
    in_flight: VecDeque<Box<dyn Reply>>,
}

impl PluginWriter {
    pub(crate) fn new(peer: Arc<dyn Peer>, path: PathBuf, opened: Opened, epoch: Epoch) -> Self {
        Self {
            peer,
            path,
            handle: opened.handle,
            epoch,
            pos: 0,
            len: 0,
            pending: Vec::new(),
            pending_start: 0,
            in_flight: VecDeque::new(),
        }
    }

    fn call(&self, request: &VfsRequest) -> io::Result<()> {
        self.peer
            .call(request, Some(self.epoch))
            .map(|_| ())
            .map_err(|error| io_error(error, &self.path))
    }

    fn pending_end(&self) -> u64 {
        self.pending_start + self.pending.len() as u64
    }

    /// Sends what is held back without waiting for the plugin, except to keep the number of
    /// windows on their way within [`IN_FLIGHT`].
    fn send(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let request = VfsRequest::Write {
            handle: self.handle,
            offset: self.pending_start,
            data: std::mem::take(&mut self.pending),
        };
        let reply = self
            .peer
            .submit(&request, Some(self.epoch))
            .map_err(|error| io_error(error, &self.path))?;
        self.in_flight.push_back(reply);
        while self.in_flight.len() > IN_FLIGHT {
            self.settle_oldest()?;
        }
        Ok(())
    }

    fn settle_oldest(&mut self) -> io::Result<()> {
        match self.in_flight.pop_front() {
            Some(reply) => reply
                .wait()
                .map(|_| ())
                .map_err(|error| io_error(error, &self.path)),
            None => Ok(()),
        }
    }

    /// Reads every reply still owed, and reports the first failure among them.
    fn settle_all(&mut self) -> io::Result<()> {
        let mut first = Ok(());
        while !self.in_flight.is_empty() {
            let outcome = self.settle_oldest();
            if first.is_ok() {
                first = outcome;
            }
        }
        first
    }
}

fn io_error(error: PeerError, path: &std::path::Path) -> io::Error {
    match error_of(error, path) {
        shared::VfsError::Io { source, .. } => source,
        other => io::Error::other(other.to_string()),
    }
}

impl Write for PluginWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        if !self.pending.is_empty() && self.pos != self.pending_end() {
            self.send()?;
        }
        if self.pending.is_empty() {
            self.pending_start = self.pos;
        }
        let taken = data.len().min(WINDOW - self.pending.len());
        self.pending.extend_from_slice(&data[..taken]);
        self.pos += taken as u64;
        self.len = self.len.max(self.pos);
        if self.pending.len() >= WINDOW {
            self.send()?;
        }
        Ok(taken)
    }

    fn flush(&mut self) -> io::Result<()> {
        let sent = self.send();
        // Settled even when the send failed, so no reply is left behind a closed handle.
        let settled = self.settle_all();
        sent.and(settled)?;
        self.call(&VfsRequest::Flush {
            handle: self.handle,
        })
    }
}

impl Seek for PluginWriter {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
        };
        self.pos = target.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek to a position before the start of the file",
            )
        })?;
        Ok(self.pos)
    }
}

impl Drop for PluginWriter {
    /// A writer the caller lets go of without a `flush` still delivers what it holds, as a local
    /// file does; there is no one left to tell if this last attempt fails.
    fn drop(&mut self) {
        let _ = self.send();
        // The close must not overtake a write the plugin has not run yet.
        let _ = self.settle_all();
        // Without waiting: the interface must not stall on a plugin that has stopped answering,
        // and one that is gone has let go of the file already.
        self.peer.release(
            &VfsRequest::Close {
                handle: self.handle,
            },
            self.epoch,
        );
    }
}
