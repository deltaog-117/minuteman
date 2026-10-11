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

//! The line the host writes to a plugin: always a notification (method `event`), never expecting
//! a reply. `HostEvent` builds and serializes one; `EventParams` is what a plugin reads back.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    API_VERSION, ArchiveCancel, ArchiveRequest, ArchiveWork, Change, EVENT_ACTION, EVENT_ARCHIVE,
    EVENT_ARCHIVE_CANCEL, EVENT_INSPECT, EVENT_PICKER_CLOSED, EVENT_PICKER_QUERY, EVENT_PREVIEW,
    EVENT_TALLY_DONE, EVENT_WALK_BATCH, EVENT_WALK_DONE, InspectTarget, Job, JobOutcome,
    PickerEvent, PreviewTarget, TallyDone, TreeEntry, WalkEnd, WalkEvent,
};

/// A host -> plugin line: always a notification, never expects a reply.
#[derive(Debug, Clone, Serialize)]
pub struct HostEvent {
    jsonrpc: &'static str,
    method: &'static str,
    params: EventParams,
}

/// The parameters of a host event. Which of the optional ones are present depends on `name`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventParams {
    pub name: String,
    pub api_version: u32,
    pub cwd: PathBuf,
    pub selection: Vec<PathBuf>,
    /// Which key fired a `key` event, spelled as in the config (`"b"`, `"enter"`, `"space"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The word a `command` event was run as: the `:` command the plugin declared, or `"plugin"`
    /// for `:plugin <name>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The words after the command, split the way a shell splits plain arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// On an `action` event: the name of the action the user triggered that this plugin provides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// On an `action` event: whether the browser is showing dot-files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_hidden: Option<bool>,
    /// On the picker events (`picker_query`, `picker_closed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picker: Option<PickerEvent>,
    /// On the walk events (`walk_batch`, `walk_done`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walk: Option<WalkEvent>,
    /// On a `preview` event: the file the preview column is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<PreviewTarget>,
    /// On an `inspect` event: the entry the Inspect panel is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspect: Option<InspectTarget>,
    /// On a `tally_done` event: how a count the plugin asked for ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tally: Option<TallyDone>,
    /// On an `archive` event: the work to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveRequest>,
    /// On an `archive_cancel` event: the request to stop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_cancel: Option<ArchiveCancel>,
    /// Present on `dir_changed` and `selection_changed`; its `from` and `to` sit beside the
    /// other params. (A reader of a `job_finished` may see an empty one: go by `name`.)
    #[serde(default, flatten)]
    pub change: Option<Change>,
    /// Present on `job_finished`; its `kind` and `outcome` sit beside the other params.
    #[serde(default, flatten)]
    pub job: Option<Job>,
}

impl HostEvent {
    pub fn new(name: impl Into<String>, cwd: PathBuf, selection: Vec<PathBuf>) -> Self {
        Self {
            jsonrpc: "2.0",
            method: "event",
            params: EventParams {
                name: name.into(),
                api_version: API_VERSION,
                cwd,
                selection,
                key: None,
                command: None,
                args: Vec::new(),
                action: None,
                show_hidden: None,
                picker: None,
                walk: None,
                preview: None,
                inspect: None,
                tally: None,
                archive: None,
                archive_cancel: None,
                change: None,
                job: None,
            },
        }
    }

    /// Records which key fired this event, spelled as the config spells it.
    pub fn with_key_name(mut self, key: String) -> Self {
        self.params.key = Some(key);
        self
    }

    /// Records the `:` command this event was run as, and its arguments.
    pub fn with_command(mut self, command: impl Into<String>, args: Vec<String>) -> Self {
        self.params.command = Some(command.into());
        self.params.args = args;
        self
    }

    /// Records what moved on a `dir_changed` or `selection_changed` event.
    pub fn with_change(mut self, from: Option<PathBuf>, to: Option<PathBuf>) -> Self {
        self.params.change = Some(Change { from, to });
        self
    }

    /// Records which job ended, and how, on a `job_finished` event.
    pub fn with_job(mut self, kind: impl Into<String>, outcome: JobOutcome) -> Self {
        self.params.job = Some(Job {
            kind: kind.into(),
            outcome,
        });
        self
    }

    /// An `action` event: the user triggered `action`, which this plugin provides.
    pub fn action(
        action: impl Into<String>,
        cwd: PathBuf,
        selection: Vec<PathBuf>,
        show_hidden: bool,
    ) -> Self {
        let mut event = Self::new(EVENT_ACTION, cwd, selection);
        event.params.action = Some(action.into());
        event.params.show_hidden = Some(show_hidden);
        event
    }

    /// A `picker_query` event: what is typed into the picker changed.
    pub fn picker_query(picker: u64, generation: u64, text: impl Into<String>) -> Self {
        let mut event = Self::new(EVENT_PICKER_QUERY, PathBuf::new(), Vec::new());
        event.params.picker = Some(PickerEvent {
            picker,
            generation: Some(generation),
            text: Some(text.into()),
        });
        event
    }

    /// A `picker_closed` event: the user closed the picker, or chose a result.
    pub fn picker_closed(picker: u64) -> Self {
        let mut event = Self::new(EVENT_PICKER_CLOSED, PathBuf::new(), Vec::new());
        event.params.picker = Some(PickerEvent {
            picker,
            generation: None,
            text: None,
        });
        event
    }

    /// A `walk_batch` event: more entries of a walk.
    pub fn walk_batch(walk: u64, entries: Vec<TreeEntry>) -> Self {
        let mut event = Self::new(EVENT_WALK_BATCH, PathBuf::new(), Vec::new());
        event.params.walk = Some(WalkEvent {
            walk,
            entries,
            end: None,
        });
        event
    }

    /// A `walk_done` event: a walk has ended.
    pub fn walk_done(walk: u64, end: WalkEnd) -> Self {
        let mut event = Self::new(EVENT_WALK_DONE, PathBuf::new(), Vec::new());
        event.params.walk = Some(WalkEvent {
            walk,
            entries: Vec::new(),
            end: Some(end),
        });
        event
    }

    /// A `preview` event: the preview column is now about `path` (`None` when it shows no file),
    /// or the file it already showed changed and wants reading again.
    pub fn preview(path: Option<PathBuf>) -> Self {
        let mut event = Self::new(EVENT_PREVIEW, PathBuf::new(), Vec::new());
        event.params.preview = Some(PreviewTarget { path });
        event
    }

    /// An `inspect` event: the Inspect panel is now about `path` (`None` when it closed).
    pub fn inspect(path: Option<PathBuf>) -> Self {
        let mut event = Self::new(EVENT_INSPECT, PathBuf::new(), Vec::new());
        event.params.inspect = Some(InspectTarget { path });
        event
    }

    /// A `tally_done` event: a count the plugin asked for has ended.
    pub fn tally_done(done: TallyDone) -> Self {
        let mut event = Self::new(EVENT_TALLY_DONE, PathBuf::new(), Vec::new());
        event.params.tally = Some(done);
        event
    }

    /// An `archive` event: do `work`, and report under the number `job`.
    pub fn archive(job: u64, work: ArchiveWork) -> Self {
        let mut event = Self::new(EVENT_ARCHIVE, PathBuf::new(), Vec::new());
        event.params.archive = Some(ArchiveRequest { job, work });
        event
    }

    /// An `archive_cancel` event: stop the request numbered `job`.
    pub fn archive_cancel(job: u64) -> Self {
        let mut event = Self::new(EVENT_ARCHIVE_CANCEL, PathBuf::new(), Vec::new());
        event.params.archive_cancel = Some(ArchiveCancel { job });
        event
    }

    /// Serializes to exactly one `\n`-terminated line, ready to write to the plugin's stdin.
    pub fn to_line(&self) -> String {
        crate::client::line(self)
    }
}
