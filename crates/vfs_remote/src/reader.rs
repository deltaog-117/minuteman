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

//! A file on a remote machine, read in windows through the handle the plugin opened for it.
//!
//! A request costs a round trip through another process, so a reader fetches a window at a time
//! and answers small reads from it. The window starts small and doubles while reads continue where
//! the last ended (a file read straight through), and falls back to the small size after a jump
//! (a seek to a header or a footer).

use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;

use plugin_api::{Bytes, Epoch, MAX_VFS_READ, Opened, Peer, PeerError, VfsRequest};

use crate::error_of;

/// The first window a reader fetches, and the window it falls back to after a seek.
const FIRST_WINDOW: u64 = 16 << 10;

pub(crate) struct PluginReader {
    peer: Arc<dyn Peer>,
    path: PathBuf,
    handle: u64,
    /// The life of the plugin's process the handle belongs to: a reader outlives a restart of
    /// the plugin without ever being answered by the new process.
    epoch: Epoch,
    /// The length the file had when it was opened.
    len: u64,
    pos: u64,
    /// The bytes that start at `window_start`.
    window: Vec<u8>,
    window_start: u64,
    /// How much the next fetch asks for.
    next_len: u64,
}

impl PluginReader {
    pub(crate) fn new(peer: Arc<dyn Peer>, path: PathBuf, opened: Opened, epoch: Epoch) -> Self {
        Self {
            peer,
            path,
            handle: opened.handle,
            epoch,
            len: opened.len,
            pos: 0,
            window: Vec::new(),
            window_start: 0,
            next_len: FIRST_WINDOW,
        }
    }

    fn window_end(&self) -> u64 {
        self.window_start + self.window.len() as u64
    }

    /// Makes `pos` fall inside the window, fetching if it does not.
    fn fill(&mut self) -> io::Result<()> {
        if self.pos >= self.window_start && self.pos < self.window_end() {
            return Ok(());
        }
        let continues = !self.window.is_empty() && self.pos == self.window_end();
        self.next_len = if continues {
            (self.next_len * 2).min(MAX_VFS_READ)
        } else {
            FIRST_WINDOW
        };
        let wanted = (self.len - self.pos).min(self.next_len);
        let request = VfsRequest::Read {
            handle: self.handle,
            offset: self.pos,
            len: wanted,
        };
        let answer = self
            .peer
            .call(&request, Some(self.epoch))
            .map_err(|error| io_error(error, self))?;
        let bytes: Bytes = serde_json::from_value(answer.value).map_err(io::Error::other)?;
        if bytes.data.is_empty() {
            // The plugin has less than it said it had; looping on an empty answer would never end.
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the remote file ended early",
            ));
        }
        self.window = bytes.data;
        self.window_start = self.pos;
        Ok(())
    }
}

fn io_error(error: PeerError, reader: &PluginReader) -> io::Error {
    match error_of(error, &reader.path) {
        shared::VfsError::Io { source, .. } => source,
        other => io::Error::other(other.to_string()),
    }
}

impl Read for PluginReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.pos >= self.len {
            return Ok(0);
        }
        self.fill()?;
        let from = usize::try_from(self.pos - self.window_start)
            .map_err(|_| io::Error::other("offset out of range"))?;
        let available = self.window.get(from..).unwrap_or_default();
        let count = available.len().min(out.len());
        out[..count].copy_from_slice(&available[..count]);
        self.pos += count as u64;
        Ok(count)
    }
}

impl Seek for PluginReader {
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

impl Drop for PluginReader {
    fn drop(&mut self) {
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
