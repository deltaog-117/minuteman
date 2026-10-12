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

//! The data a plugin and the host agree on, kept apart from the host so a plugin (the bundled ones
//! and any third-party Rust one) can depend on it without pulling in the host's process handling,
//! `file_ops` or the terminal. Everything here is plain serializable data and the names that
//! go with it; reading and writing the lines themselves stays in `plugin_host::protocol`, which
//! re-exports these types under their old paths.
//!
//! Both directions derive `Serialize` and `Deserialize`: the host writes an event a plugin reads,
//! and a plugin writes a log line the host reads.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use shared::DirEntryInfo;

mod archive;
mod canvas;
pub mod client;
mod devices;
mod event;
mod inspect;
mod picker;
mod preview;
pub mod remote;
mod serve;
mod view;

pub use archive::{
    ArchiveCancel, ArchiveDone, ArchiveError, ArchiveProgress, ArchiveRequest, ArchiveStatus,
    ArchiveWork, CompressJob, Conflict, EVENT_ARCHIVE, EVENT_ARCHIVE_CANCEL, ExtractJob, Format,
    Level, MAX_ARCHIVE_JOBS, MAX_ARCHIVE_TEXT_CHARS, MAX_PASSWORD_CHARS, METHOD_ARCHIVE_DONE,
    METHOD_ARCHIVE_PROGRESS, PROVIDES_ARCHIVE, Password,
};
pub use canvas::{
    Button, Canvas, CanvasError, CanvasEvent, CanvasInput, CanvasNote, Cells, EVENT_CANVAS, HUES,
    Item, MAX_CANVAS_CHARS, MAX_CANVAS_ITEMS, MAX_CANVAS_SIDE, MAX_CANVAS_TEXT_CHARS, MAX_CLAIMS,
    MAX_PATH_POINTS, METHOD_CANVAS_NOTE, METHOD_SET_CANVAS, Mark, Mods, PROVIDES_CANVAS_ATLAS,
    PROVIDES_CANVAS_WIRING, Paint, SetCanvas, Surface,
};
pub use devices::{
    DeviceKind, DevicesCommand, DevicesNote, EVENT_DEVICES, MAX_DEVICE_TEXT_CHARS, MAX_DEVICES,
    METHOD_DEVICES_NOTE, METHOD_SET_DEVICES, PROVIDES_DEVICES, SetDevices, WireDevice,
};
pub use event::{EventParams, HostEvent};
pub use inspect::{
    EVENT_INSPECT, EVENT_TALLY_DONE, InspectError, InspectRow, InspectTarget, MAX_INSPECT_ROWS,
    MAX_LABEL_CHARS, MAX_VALUE_CHARS, METHOD_SET_INSPECT, METHOD_STAT, METHOD_TALLY,
    METHOD_TALLY_CANCEL, PROVIDES_INSPECT, SetInspect, Stat, StatKind, StatReply, TallyCancel,
    TallyDone, TallyEnding, TallyRequest,
};
pub use picker::{
    METHOD_PICKER_CLOSE, METHOD_PICKER_ITEMS, METHOD_SHOW_PICKER, METHOD_WALK, METHOD_WALK_CANCEL,
    PickerClose, PickerEvent, PickerIndexing, PickerSpec, PickerUpdate, TreeEntry, WalkCancel,
    WalkEnd, WalkEvent, WalkOutcome, WalkRequest,
};
pub use preview::{
    ArchiveEntry, ArchiveKind, ArchiveListing, ChunkKind, EVENT_PREVIEW, FileChunk, HANDOFF_PREFIX,
    HexHead, Hidden, ImageSource, MAX_ARCHIVE_ENTRIES, MAX_HEX_BYTES, MAX_READ_BYTES,
    MAX_TEXT_CHARS, METHOD_READ_FILE, METHOD_SET_PREVIEW, PROVIDES_PREVIEW, Piece, PreviewContent,
    PreviewError, PreviewLine, PreviewTarget, ReadFile, SetPreview, Token, handoff_path,
    hex_string, is_handoff_path,
};
pub use serve::{
    Answer, Bytes, Epoch, IoKind, LinkTarget, MAX_OPEN_HANDLES, MAX_VFS_READ, METHOD_VFS, Opened,
    PROVIDES_VFS_ARCHIVE, PROVIDES_VFS_SSH, Peer, PeerError, Reply, Resolved, VfsFault, VfsRequest,
    WireKind, WireMeta, WireScan, fault_line, reply_line, request_to_line,
};
pub use view::{
    Glyph, Line, MAX_BADGE_CHARS, MAX_BADGES, MAX_CHARS, MAX_KEY_CHARS, MAX_SPANS, METHOD_SET_VIEW,
    SetView, Slot, Span, Tone, View, ViewError,
};

/// Bumped only for a breaking change to the shapes below and in `plugin_host::protocol`; sent
/// with every event so a plugin can check it against what it was built for and refuse to run
/// rather than misbehave silently.
pub const API_VERSION: u32 = 1;

/// The event name a plugin gets exactly once, right after its process starts.
pub const EVENT_INIT: &str = "init";
/// The event name a plugin gets when one of its configured keys is pressed.
pub const EVENT_KEY: &str = "key";
/// The event name a plugin gets when a `:` command it declared, or `:plugin <its name>`, is run.
pub const EVENT_COMMAND: &str = "command";

/// The event name a subscribed plugin gets when the browsed directory changes.
pub const EVENT_DIR_CHANGED: &str = "dir_changed";
/// The event name a subscribed plugin gets when the cursor entry or the marks change.
pub const EVENT_SELECTION_CHANGED: &str = "selection_changed";
/// The event name a subscribed plugin gets when a copy, move, delete or similar job ends.
pub const EVENT_JOB_FINISHED: &str = "job_finished";

/// The event name a plugin gets when the user triggers an action the plugin provides (see the
/// `provides` setting of a `[[plugin]]` table).
pub const EVENT_ACTION: &str = "action";
/// The event name a plugin gets when what is typed into its picker changes.
pub const EVENT_PICKER_QUERY: &str = "picker_query";
/// The event name a plugin gets when its picker was closed from the keyboard or a result chosen.
pub const EVENT_PICKER_CLOSED: &str = "picker_closed";
/// The event name a plugin gets with more entries of a walk it asked for.
pub const EVENT_WALK_BATCH: &str = "walk_batch";
/// The event name a plugin gets when a walk it asked for has ended.
pub const EVENT_WALK_DONE: &str = "walk_done";

/// The events a plugin must ask for (`events = [...]` in its `[[plugin]]` table) because, unlike
/// `init`, `key` and `command`, they fire without the user addressing the plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    DirChanged,
    SelectionChanged,
    JobFinished,
}

impl EventKind {
    /// The event as the wire and the config spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::DirChanged => EVENT_DIR_CHANGED,
            Self::SelectionChanged => EVENT_SELECTION_CHANGED,
            Self::JobFinished => EVENT_JOB_FINISHED,
        }
    }

    /// The event called `name`, or `None` for a word that names no event.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::DirChanged, Self::SelectionChanged, Self::JobFinished]
            .into_iter()
            .find(|kind| kind.name() == name)
    }
}

/// How a background job ended, as a `job_finished` event reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobOutcome {
    Completed,
    /// Every item was skipped because its destination already existed.
    Skipped,
    Cancelled,
    Failed,
}

/// What moved, on a `dir_changed` or `selection_changed` event. `from` and `to` are the old and
/// new directory, or the old and new cursor entry (`null` for an empty directory). A burst of
/// changes reaches a plugin as one event from where it began to where it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub from: Option<PathBuf>,
    pub to: Option<PathBuf>,
}

/// What ended, on a `job_finished` event: `kind` is the verb (`copy`, `move`, `delete`, `trash`,
/// `extract`, `compress` or `command`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub kind: String,
    pub outcome: JobOutcome,
}

/// What a `get_state` reply carries: the browser as the user sees it, as of the last render tick.
/// Published by the host on a `watch` channel each tick (see `PluginManager::publish_state`), so
/// answering a request never touches the render thread, and a plugin that asks twice in one tick
/// gets the same answer twice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// The directory being browsed.
    pub cwd: PathBuf,
    /// The entry under the cursor, or `null` for an empty directory.
    pub cursor: Option<PathBuf>,
    /// Every `v` mark, sorted. Marks are the only selection, so this is what a built-in bulk
    /// action would act on.
    pub marks: Vec<PathBuf>,
    pub show_hidden: bool,
    /// The sort key's name (`"name"`, `"size"`, `"modified"` or `"mode"`).
    pub sort: String,
    pub sort_reversed: bool,
    /// The view's name (`"list"`, `"grid"`, `"details"` or `"filmstrip"`).
    pub view: String,
}

/// How urgent a plugin's `log` line is — purely advisory, the host shows every level the same way
/// (the status bar has one line, not a severity-colored log view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// A directory listing entry, in the plain JSON shape a `read_dir` reply carries — `shared`'s own
/// `DirEntryInfo` isn't `Serialize` (its `modified: Option<SystemTime>` isn't JSON-shaped), so
/// this converts it rather than adding a wire concern to shared infrastructure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    /// Whole seconds since the Unix epoch, or `null` when the entry's metadata couldn't be read.
    pub modified: Option<u64>,
    pub mode: Option<u32>,
}

impl From<&DirEntryInfo> for WireEntry {
    fn from(entry: &DirEntryInfo) -> Self {
        Self {
            name: entry.name.clone(),
            path: entry.path.clone(),
            is_dir: entry.is_dir,
            size: entry.size,
            modified: entry
                .modified
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs()),
            mode: entry.mode,
        }
    }
}
