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

//! A plugin answering for a kind of path: the host asks, the plugin replies. Everywhere else the
//! protocol has the plugin ask and the host reply; this is the one place it runs the other way,
//! because the interface needs the answer in the middle of a synchronous `Vfs` call.
//!
//! A plugin that `provides` [`PROVIDES_VFS_ARCHIVE`] is sent [`METHOD_VFS`] requests for every path
//! inside an archive (`/x/a.zip!/dir/f`) and answers each with one reply carrying the same `id`.
//! The folder it serves is read-only, so the requests only look: stat, list, read a link, and open
//! a file to read it in pieces. What the plugin needs from the archive file itself it asks of the
//! host with `read_file`, as the preview plugin does.
//!
//! A file is read through a handle. The plugin keeps what it unpacked for the handle (a zip or
//! gzip entry cannot seek, so it is unpacked once) until `close`; handles number from 1 in each
//! process of the plugin, so the host must never carry one across a restart (see `Peer`).

use std::fmt;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::preview::hex_string;

/// The `provides` word of a plugin that serves the paths inside archives.
pub const PROVIDES_VFS_ARCHIVE: &str = "vfs-archive";

/// The `provides` word of a plugin that serves the paths on remote machines (`ssh://host/dir`).
/// It is sent the same [`METHOD_VFS`] requests, and also the ones that change things (see
/// [`VfsRequest::Create`] on), which a plugin that serves archives is never sent.
pub const PROVIDES_VFS_SSH: &str = "vfs-ssh";

/// The method of a host -> plugin request line.
pub const METHOD_VFS: &str = "vfs";

/// How many files a plugin keeps open for the host at once. Each can hold an unpacked entry in
/// memory, so the count is what bounds that memory; a host that leaks handles gets a refusal
/// rather than a plugin that grows.
pub const MAX_OPEN_HANDLES: usize = 32;

/// The most bytes one `read` returns, so a single reply stays a line of reasonable length.
pub const MAX_VFS_READ: u64 = 256 << 10;

/// One thing the host asks of the plugin. `path` is always a full path inside an archive, the way
/// the browser shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum VfsRequest {
    /// Describes one entry; `follow` says whether a final symlink is followed. Replies [`WireMeta`].
    Metadata { path: PathBuf, follow: bool },
    /// Lists a folder the way the browser shows it. Replies a list of `WireEntry`.
    List { path: PathBuf },
    /// Lists a folder describing each entry itself. Replies a list of [`WireScan`].
    Scan { path: PathBuf },
    /// Reads where a symlink points. Replies [`LinkTarget`].
    ReadLink { path: PathBuf },
    /// Opens a file for reading. Replies [`Opened`].
    Open { path: PathBuf },
    /// Reads up to `len` bytes of an open file from `offset`. Replies [`Bytes`], fewer at the end
    /// of the file and none past it.
    Read { handle: u64, offset: u64, len: u64 },
    /// Lets go of a handle. Replies `{}`, also for one that is already gone.
    Close { handle: u64 },
    /// Turns what was typed after `ssh://` into the folder to open: for an address that names only
    /// a machine, that login's home. Replies [`Resolved`].
    Home { path: PathBuf },
    /// Creates a new file to write and keeps it open. Replies [`Opened`] (the length is 0), and
    /// is refused if `path` exists. It shares [`MAX_OPEN_HANDLES`] with `open`.
    Create { path: PathBuf },
    /// Writes `data` into an open new file at `offset`, which may lie beyond what is written (a
    /// zip goes back to patch a header). Replies `{}`; at most [`MAX_VFS_READ`] bytes.
    Write {
        handle: u64,
        offset: u64,
        #[serde(with = "hex_string")]
        data: Vec<u8>,
    },
    /// Makes what was written reach the file, and reports a write that failed after it was
    /// accepted. Replies `{}`.
    Flush { handle: u64 },
    /// Makes a folder. Replies `{}`.
    CreateDir { path: PathBuf },
    /// Makes an empty file, refusing one that exists. Replies `{}`.
    CreateFile { path: PathBuf },
    /// Sets the modified time to now, making an empty file when there is none. Replies `{}`.
    Touch { path: PathBuf },
    /// Sets the permission bits. Replies `{}`.
    SetMode { path: PathBuf, mode: u32 },
    /// Makes `link` a symlink pointing at `target`. Replies `{}`.
    Symlink { target: PathBuf, link: PathBuf },
    /// Moves within one machine, refusing a `to` that exists. Replies `{}`.
    Rename { from: PathBuf, to: PathBuf },
    /// Moves within one machine over a `to` that exists. Replies `{}`.
    Replace { from: PathBuf, to: PathBuf },
    /// Copies a file within one machine, so its bytes never travel. Replies `{}`.
    CopyFile { from: PathBuf, to: PathBuf },
    /// Removes a file or a symlink. Replies `{}`.
    RemoveFile { path: PathBuf },
    /// Removes a folder and everything under it. Replies `{}`.
    RemoveDirAll { path: PathBuf },
}

/// What kind of thing an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// The reply to `metadata`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireMeta {
    pub kind: WireKind,
    pub len: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// Whole seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
}

/// One entry of a `scan` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireScan {
    pub name: String,
    pub path: PathBuf,
    /// Absent when the backend could not describe the entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<WireMeta>,
}

/// The reply to `read_link`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkTarget {
    pub target: PathBuf,
}

/// The reply to `open`: the handle to `read`, and the file's whole length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opened {
    pub handle: u64,
    pub len: u64,
}

/// The reply to `home`: the path to open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolved {
    pub path: PathBuf,
}

/// The reply to `read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bytes {
    #[serde(with = "hex_string")]
    pub data: Vec<u8>,
}

/// The `io::ErrorKind`s a failure can be handed over as. The host turns the kind back into an
/// `io::Error`, so a caller that looks at the kind (`PermissionDenied` for an encrypted entry)
/// sees what it would have seen from a backend in its own process; anything else is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IoKind {
    NotFound,
    PermissionDenied,
    InvalidInput,
    InvalidData,
    FileTooLarge,
    IsADirectory,
    UnexpectedEof,
    /// A remote machine that could not be reached.
    NotConnected,
    /// A connection that went away mid-request.
    BrokenPipe,
    TimedOut,
    Unsupported,
    Other,
}

impl From<io::ErrorKind> for IoKind {
    fn from(kind: io::ErrorKind) -> Self {
        match kind {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            io::ErrorKind::InvalidInput => Self::InvalidInput,
            io::ErrorKind::InvalidData => Self::InvalidData,
            io::ErrorKind::FileTooLarge => Self::FileTooLarge,
            io::ErrorKind::IsADirectory => Self::IsADirectory,
            io::ErrorKind::UnexpectedEof => Self::UnexpectedEof,
            io::ErrorKind::NotConnected => Self::NotConnected,
            io::ErrorKind::BrokenPipe => Self::BrokenPipe,
            io::ErrorKind::TimedOut => Self::TimedOut,
            io::ErrorKind::Unsupported => Self::Unsupported,
            _ => Self::Other,
        }
    }
}

impl From<IoKind> for io::ErrorKind {
    fn from(kind: IoKind) -> Self {
        match kind {
            IoKind::NotFound => Self::NotFound,
            IoKind::PermissionDenied => Self::PermissionDenied,
            IoKind::InvalidInput => Self::InvalidInput,
            IoKind::InvalidData => Self::InvalidData,
            IoKind::FileTooLarge => Self::FileTooLarge,
            IoKind::IsADirectory => Self::IsADirectory,
            IoKind::UnexpectedEof => Self::UnexpectedEof,
            IoKind::NotConnected => Self::NotConnected,
            IoKind::BrokenPipe => Self::BrokenPipe,
            IoKind::TimedOut => Self::TimedOut,
            IoKind::Unsupported => Self::Unsupported,
            IoKind::Other => Self::Other,
        }
    }
}

/// Why a request failed, as the plugin hands it over. It travels as the `data` of the reply's
/// `error`, whose `message` is the same words for a host that only shows text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fault", rename_all = "snake_case")]
pub enum VfsFault {
    NotFound,
    NotADirectory,
    AlreadyExists,
    /// The path is something the plugin does not serve.
    Unsupported {
        reason: String,
    },
    Io {
        kind: IoKind,
        message: String,
    },
    /// The handle is not open (never was, or was closed).
    BadHandle,
    /// The plugin already holds [`MAX_OPEN_HANDLES`] files for the host.
    TooManyOpen,
}

impl fmt::Display for VfsFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::NotADirectory => f.write_str("not a directory"),
            Self::AlreadyExists => f.write_str("already exists"),
            Self::Unsupported { reason } => f.write_str(reason),
            Self::Io { message, .. } => f.write_str(message),
            Self::BadHandle => f.write_str("that file is no longer open"),
            Self::TooManyOpen => write!(f, "more than {MAX_OPEN_HANDLES} files are open"),
        }
    }
}

/// Which life of the plugin's process an answer came from. A handle means something only to the
/// process that issued it, so the host stamps every handle with the epoch it was opened in and a
/// request that names an older one is refused rather than sent to a restarted plugin that may
/// have given the same number to another file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Epoch(pub u64);

/// A plugin's reply, and the epoch of the process that gave it.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub epoch: Epoch,
    pub value: Value,
}

/// Why a request got no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerError {
    /// No plugin serves these paths: none is installed, or it could not be started.
    Unavailable,
    /// The plugin's process ended while the request waited.
    Stopped,
    /// The plugin did not answer in time.
    TimedOut,
    /// The request continues a handle from an earlier process of the plugin.
    Stale,
    /// The plugin answered that it could not.
    Fault(VfsFault),
    /// The plugin answered with something that is not what was asked.
    Garbled(String),
}

impl fmt::Display for PeerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("the plugin that serves these paths is not running"),
            Self::Stopped => f.write_str("the plugin stopped while answering"),
            Self::TimedOut => f.write_str("the plugin did not answer in time"),
            Self::Stale => f.write_str("the plugin was restarted since this file was opened"),
            Self::Fault(fault) => fault.fmt(f),
            Self::Garbled(why) => write!(f, "unreadable answer from the plugin: {why}"),
        }
    }
}

/// The way from the interface to the plugin that serves a kind of path, as the part of the
/// interface that needs an answer sees it. `plugin_host` provides the real one; a test provides a
/// stand-in.
pub trait Peer: Send + Sync {
    /// Sends `request` and waits for the reply. `after` is the epoch in which the handle the
    /// request names was opened, and `None` for a request that names none.
    ///
    /// # Errors
    ///
    /// See [`PeerError`].
    fn call(&self, request: &VfsRequest, after: Option<Epoch>) -> Result<Answer, PeerError>;

    /// Sends `request` for a handle opened in epoch `after` and does not wait for the reply: for
    /// closing a file from a destructor, which must not stall the interface on a plugin that has
    /// stopped answering. A request that cannot be sent is dropped, since a plugin that is gone
    /// has already let go of everything.
    fn release(&self, request: &VfsRequest, after: Epoch) {
        let _ = self.call(request, Some(after));
    }
}

/// A host -> plugin request line.
#[derive(Debug, Clone, Serialize)]
struct RequestLine<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: &'a VfsRequest,
}

/// Serializes `request` to exactly one `\n`-terminated line for the plugin's stdin.
pub fn request_to_line(id: u64, request: &VfsRequest) -> String {
    crate::client::line(&RequestLine {
        jsonrpc: "2.0",
        id,
        method: METHOD_VFS,
        params: request,
    })
}

/// A plugin -> host reply line to a [`METHOD_VFS`] request.
pub fn reply_line(id: &Value, result: &impl Serialize) -> String {
    #[derive(Serialize)]
    struct Line<'a, R> {
        jsonrpc: &'static str,
        id: &'a Value,
        result: &'a R,
    }
    crate::client::line(&Line {
        jsonrpc: "2.0",
        id,
        result,
    })
}

/// A plugin -> host reply line saying a [`METHOD_VFS`] request failed.
pub fn fault_line(id: &Value, fault: &VfsFault) -> String {
    #[derive(Serialize)]
    struct Body<'a> {
        code: i64,
        message: String,
        data: &'a VfsFault,
    }
    #[derive(Serialize)]
    struct Line<'a> {
        jsonrpc: &'static str,
        id: &'a Value,
        error: Body<'a>,
    }
    crate::client::line(&Line {
        jsonrpc: "2.0",
        id,
        error: Body {
            code: -32000,
            message: fault.to_string(),
            data: fault,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_request_is_one_line_naming_its_operation() {
        let line = request_to_line(
            7,
            &VfsRequest::Read {
                handle: 3,
                offset: 10,
                len: 20,
            },
        );
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
        let json: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(json["method"], METHOD_VFS);
        assert_eq!(json["id"], 7);
        assert_eq!(
            json["params"],
            serde_json::json!({"op": "read", "handle": 3, "offset": 10, "len": 20})
        );
    }

    #[test]
    fn every_request_that_changes_something_survives_the_trip() {
        let requests = [
            VfsRequest::Home {
                path: "ssh://h".into(),
            },
            VfsRequest::Create {
                path: "ssh://h/a".into(),
            },
            VfsRequest::Write {
                handle: 2,
                offset: 1 << 40,
                data: vec![0, 255, 16],
            },
            VfsRequest::Flush { handle: 2 },
            VfsRequest::CreateDir {
                path: "ssh://h/d".into(),
            },
            VfsRequest::SetMode {
                path: "ssh://h/a".into(),
                mode: 0o644,
            },
            VfsRequest::Symlink {
                target: "a".into(),
                link: "ssh://h/b".into(),
            },
            VfsRequest::CopyFile {
                from: "ssh://h/a".into(),
                to: "ssh://h/c".into(),
            },
            VfsRequest::RemoveDirAll {
                path: "ssh://h/d".into(),
            },
        ];
        for request in requests {
            let line = request_to_line(1, &request);
            let json: Value = serde_json::from_str(&line).unwrap();
            let back: VfsRequest = serde_json::from_value(json["params"].clone()).unwrap();
            assert_eq!(back, request);
        }
        let line = request_to_line(
            1,
            &VfsRequest::Write {
                handle: 2,
                offset: 0,
                data: vec![0, 255, 16],
            },
        );
        assert!(
            line.contains(r#""data":"00ff10""#),
            "bytes go as hex: {line}"
        );
    }

    #[test]
    fn bytes_travel_as_hex_and_a_fault_keeps_its_kind() {
        let bytes = Bytes {
            data: vec![0, 255, 16],
        };
        assert_eq!(
            serde_json::to_string(&bytes).unwrap(),
            r#"{"data":"00ff10"}"#
        );

        let fault = VfsFault::Io {
            kind: IoKind::PermissionDenied,
            message: "encrypted".into(),
        };
        let line = fault_line(&Value::from(4), &fault);
        let json: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(json["error"]["message"], "encrypted");
        let back: VfsFault = serde_json::from_value(json["error"]["data"].clone()).unwrap();
        assert_eq!(back, fault);
    }

    #[test]
    fn io_kinds_survive_the_trip_and_unknown_ones_become_other() {
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidInput,
            io::ErrorKind::InvalidData,
            io::ErrorKind::FileTooLarge,
            io::ErrorKind::IsADirectory,
            io::ErrorKind::UnexpectedEof,
            io::ErrorKind::NotConnected,
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::TimedOut,
            io::ErrorKind::Unsupported,
        ] {
            assert_eq!(io::ErrorKind::from(IoKind::from(kind)), kind);
        }
        assert_eq!(IoKind::from(io::ErrorKind::Interrupted), IoKind::Other);
    }

    proptest! {
        /// Whatever bytes a read carries come back exactly, so a file copied out of an archive
        /// through the plugin is the file that was in it.
        #[test]
        fn bytes_round_trip(data in proptest::collection::vec(any::<u8>(), 0..512)) {
            let json = serde_json::to_string(&Bytes { data: data.clone() }).unwrap();
            let back: Bytes = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(back.data, data);
        }
    }
}
