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

//! The newline-delimited JSON-RPC 2.0 wire protocol a plugin process speaks over its own stdio.
//! One JSON object per line, in both directions:
//!
//! - host -> plugin: always an `"event"` notification (no `id`, no reply expected), naming what
//!   happened (`init`, sent once right after the process starts, or `key`, sent when the
//!   plugin's configured key is pressed) plus the current directory and selection. A plugin that
//!   lists `dir_changed`, `selection_changed` or `job_finished` under `events` also gets those as
//!   they happen; see `EventKind`.
//! - plugin -> host: a request with an `id` (`read_dir`, `copy`, `mv`, `delete`, `create_dir`,
//!   `create_file`, `touch`, `rename` — the same `file_ops` orchestration the built-in keys use —
//!   `get_state`, which asks for the browser's cwd, cursor, marks and view settings, and
//!   `register_command`/`unregister_command`, which add and remove a `:` command word, and
//!   `run_action`, which runs one of the interface's own actions by name, such as `paste` or
//!   `sort`, with `list_actions` naming them)
//!   gets exactly one reply line carrying that same `id`; a `log` notification (no `id`) is
//!   fire-and-forget and surfaces as the status bar message.
//!
//! Deliberately close to JSON-RPC 2.0 rather than a bespoke shape, and line-delimited rather
//! than LSP's `Content-Length` framing, so a plugin author in any language needs nothing more
//! than "read a line, parse JSON, print a line" — no header parser, no schema compiler, no
//! build step.

use std::path::PathBuf;

use crossterm::event::KeyCode;
use file_ops::ConflictPolicy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared::DirEntryInfo;

/// Bumped only for a breaking change to the shapes below; sent with every event so a plugin can
/// check it against what it was built for and refuse to run rather than misbehave silently.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub from: Option<PathBuf>,
    pub to: Option<PathBuf>,
}

/// What ended, on a `job_finished` event: `kind` is the verb (`copy`, `move`, `delete`, `trash`,
/// `extract`, `compress` or `command`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Job {
    pub kind: String,
    pub outcome: JobOutcome,
}

/// A host -> plugin line: always a notification, never expects a reply.
#[derive(Debug, Clone, Serialize)]
pub struct HostEvent {
    jsonrpc: &'static str,
    method: &'static str,
    params: EventParams,
}

#[derive(Debug, Clone, Serialize)]
struct EventParams {
    name: String,
    api_version: u32,
    cwd: PathBuf,
    selection: Vec<PathBuf>,
    /// Which key fired a `key` event, spelled as in the config (`"b"`, `"enter"`, `"space"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    /// The word a `command` event was run as: the `:` command the plugin declared, or `"plugin"`
    /// for `:plugin <name>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    /// The words after the command, split the way a shell splits plain arguments.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    /// Present on `dir_changed` and `selection_changed`; its `from` and `to` sit beside the
    /// other params.
    #[serde(flatten)]
    change: Option<Change>,
    /// Present on `job_finished`; its `kind` and `outcome` sit beside the other params.
    #[serde(flatten)]
    job: Option<Job>,
}

/// Spells `code` the way a config file does, so a plugin bound to several keys can tell which one
/// fired. Keys the config cannot name come out as their lowercase variant name.
pub fn key_name(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        other => format!("{other:?}").to_lowercase(),
    }
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
                change: None,
                job: None,
            },
        }
    }

    /// Records which key fired this event (see `key_name`).
    pub fn with_key(mut self, code: KeyCode) -> Self {
        self.params.key = Some(key_name(code));
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

    /// Serializes to exactly one `\n`-terminated line, ready to write to the plugin's stdin.
    pub fn to_line(&self) -> String {
        line(self)
    }
}

/// What a `get_state` reply carries: the browser as the user sees it, as of the last render tick.
/// Published by the host on a `watch` channel each tick (see `PluginManager::publish_state`), so
/// answering a request never touches the render thread, and a plugin that asks twice in one tick
/// gets the same answer twice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
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

/// What to do when a `copy`/`mv`/`rename` request's destination already exists — the wire form of
/// `file_ops::ConflictPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConflictPolicyWire {
    Abort,
    Skip,
    Overwrite,
}

impl From<ConflictPolicyWire> for ConflictPolicy {
    fn from(wire: ConflictPolicyWire) -> Self {
        match wire {
            ConflictPolicyWire::Abort => ConflictPolicy::Abort,
            ConflictPolicyWire::Skip => ConflictPolicy::Skip,
            ConflictPolicyWire::Overwrite => ConflictPolicy::Overwrite,
        }
    }
}

fn default_policy() -> ConflictPolicyWire {
    ConflictPolicyWire::Abort
}

/// A plugin -> host request, already validated and converted to native types — reaching the same
/// `file_ops` orchestration the built-in keys use, not a bolted-on subset of it.
#[derive(Debug, Clone, PartialEq)]
pub enum PluginRequest {
    /// Asks for the latest `StateSnapshot`; takes no parameters.
    GetState,
    /// Asks for `:name` to reach this plugin from now on.
    RegisterCommand {
        name: String,
    },
    /// Gives up a `:name` this plugin registered while running.
    UnregisterCommand {
        name: String,
    },
    /// Runs one of the interface's own actions by its stable name, as its key would.
    RunAction {
        name: String,
    },
    /// Asks for the name of every action `run_action` can run; takes no parameters.
    ListActions,
    ReadDir {
        path: PathBuf,
    },
    Copy {
        src: PathBuf,
        dst: PathBuf,
        policy: ConflictPolicy,
    },
    Move {
        src: PathBuf,
        dst: PathBuf,
        policy: ConflictPolicy,
    },
    Delete {
        path: PathBuf,
    },
    CreateDir {
        path: PathBuf,
    },
    CreateFile {
        path: PathBuf,
    },
    Touch {
        path: PathBuf,
    },
    Rename {
        path: PathBuf,
        new_name: String,
        policy: ConflictPolicy,
    },
}

#[derive(Debug, Deserialize)]
struct PathParams {
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct NameParams {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CopyMoveParams {
    src: PathBuf,
    dst: PathBuf,
    #[serde(default = "default_policy")]
    policy: ConflictPolicyWire,
}

#[derive(Debug, Deserialize)]
struct RenameParams {
    path: PathBuf,
    new_name: String,
    #[serde(default = "default_policy")]
    policy: ConflictPolicyWire,
}

impl PluginRequest {
    /// Parses a request's `method` and `params` into a typed `PluginRequest`, or a human-readable
    /// reason it couldn't (an unknown method, or params that don't match what that method needs).
    fn parse(method: &str, params: Value) -> Result<Self, String> {
        let bad = |e: serde_json::Error| format!("invalid params for '{method}': {e}");
        match method {
            "get_state" => Ok(Self::GetState),
            "register_command" => Ok(Self::RegisterCommand {
                name: serde_json::from_value::<NameParams>(params)
                    .map_err(bad)?
                    .name,
            }),
            "unregister_command" => Ok(Self::UnregisterCommand {
                name: serde_json::from_value::<NameParams>(params)
                    .map_err(bad)?
                    .name,
            }),
            "run_action" => Ok(Self::RunAction {
                name: serde_json::from_value::<NameParams>(params)
                    .map_err(bad)?
                    .name,
            }),
            "list_actions" => Ok(Self::ListActions),
            "read_dir" => Ok(Self::ReadDir {
                path: serde_json::from_value::<PathParams>(params)
                    .map_err(bad)?
                    .path,
            }),
            "copy" => {
                let p: CopyMoveParams = serde_json::from_value(params).map_err(bad)?;
                Ok(Self::Copy {
                    src: p.src,
                    dst: p.dst,
                    policy: p.policy.into(),
                })
            }
            "mv" => {
                let p: CopyMoveParams = serde_json::from_value(params).map_err(bad)?;
                Ok(Self::Move {
                    src: p.src,
                    dst: p.dst,
                    policy: p.policy.into(),
                })
            }
            "delete" => Ok(Self::Delete {
                path: serde_json::from_value::<PathParams>(params)
                    .map_err(bad)?
                    .path,
            }),
            "create_dir" => Ok(Self::CreateDir {
                path: serde_json::from_value::<PathParams>(params)
                    .map_err(bad)?
                    .path,
            }),
            "create_file" => Ok(Self::CreateFile {
                path: serde_json::from_value::<PathParams>(params)
                    .map_err(bad)?
                    .path,
            }),
            "touch" => Ok(Self::Touch {
                path: serde_json::from_value::<PathParams>(params)
                    .map_err(bad)?
                    .path,
            }),
            "rename" => {
                let p: RenameParams = serde_json::from_value(params).map_err(bad)?;
                Ok(Self::Rename {
                    path: p.path,
                    new_name: p.new_name,
                    policy: p.policy.into(),
                })
            }
            other => Err(format!("unknown method '{other}'")),
        }
    }
}

/// How urgent a plugin's `log` line is — purely advisory, the host shows every level the same way
/// (the status bar has one line, not a severity-colored log view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Deserialize)]
struct LogParams {
    #[serde(default)]
    level: Option<LogLevel>,
    message: String,
}

/// One parsed line from a plugin's stdout.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Request {
        id: Value,
        request: PluginRequest,
    },
    Log {
        level: LogLevel,
        message: String,
    },
    /// A well-formed notification (no `id`) whose method isn't `log` — ignored rather than
    /// treated as an error, so a plugin can send something a future host version understands
    /// without breaking against this one.
    UnknownNotification,
}

/// Why a line couldn't be turned into an `Incoming`. `id` is `Some` when the line carried one (it
/// was meant to be a request), so the host can still send back an error response instead of
/// leaving the plugin's request hanging forever; malformed JSON has no `id` to recover.
#[derive(Debug, PartialEq)]
pub struct LineError {
    pub id: Option<Value>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct RawIncoming {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

/// Parses one line of a plugin's stdout.
pub fn parse_line(line: &str) -> Result<Incoming, LineError> {
    let raw: RawIncoming = serde_json::from_str(line).map_err(|e| LineError {
        id: None,
        message: format!("malformed JSON: {e}"),
    })?;

    if raw.method == "log" {
        let params: LogParams = serde_json::from_value(raw.params).map_err(|e| LineError {
            id: raw.id.clone(),
            message: format!("invalid log params: {e}"),
        })?;
        return Ok(Incoming::Log {
            level: params.level.unwrap_or(LogLevel::Info),
            message: params.message,
        });
    }

    match raw.id {
        Some(id) => {
            let request =
                PluginRequest::parse(&raw.method, raw.params).map_err(|message| LineError {
                    id: Some(id.clone()),
                    message,
                })?;
            Ok(Incoming::Request { id, request })
        }
        None => Ok(Incoming::UnknownNotification),
    }
}

/// A host -> plugin reply line carrying a successful result.
pub fn ok_response(id: &Value, result: Value) -> String {
    line(&RawOutgoing {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    })
}

/// A host -> plugin reply line reporting that a request failed.
pub fn err_response(id: &Value, message: impl Into<String>) -> String {
    line(&RawOutgoing {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcErrorBody {
            message: message.into(),
        }),
    })
}

#[derive(Debug, Serialize)]
struct RawOutgoing<'a> {
    jsonrpc: &'static str,
    id: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcErrorBody>,
}

#[derive(Debug, Serialize)]
struct RpcErrorBody {
    message: String,
}

fn line(value: &impl Serialize) -> String {
    let mut text = serde_json::to_string(value).expect("protocol values always serialize");
    text.push('\n');
    text
}

/// A directory listing entry, in the plain JSON shape a `read_dir` reply carries — `shared`'s own
/// `DirEntryInfo` isn't `Serialize` (its `modified: Option<SystemTime>` isn't JSON-shaped), so
/// this converts it rather than adding a wire concern to shared infrastructure.
#[derive(Debug, Clone, Serialize)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn event_json(event: &HostEvent) -> Value {
        serde_json::from_str(event.to_line().trim()).unwrap()
    }

    #[test]
    fn a_plain_event_carries_neither_key_nor_command_fields() {
        let event = HostEvent::new(EVENT_INIT, PathBuf::from("/"), Vec::new());
        let params = &event_json(&event)["params"];
        assert!(params.get("key").is_none());
        assert!(params.get("command").is_none());
        assert!(params.get("args").is_none());
    }

    #[test]
    fn event_names_round_trip_and_unknown_words_name_nothing() {
        for kind in [
            EventKind::DirChanged,
            EventKind::SelectionChanged,
            EventKind::JobFinished,
        ] {
            assert_eq!(EventKind::parse(kind.name()), Some(kind));
        }
        assert_eq!(EventKind::parse("key"), None);
        assert_eq!(EventKind::parse(""), None);
        assert_eq!(EventKind::parse("Dir_Changed"), None);
    }

    #[test]
    fn a_change_event_carries_from_and_to_beside_the_other_params() {
        let event = HostEvent::new(EVENT_DIR_CHANGED, PathBuf::from("/b"), Vec::new())
            .with_change(Some(PathBuf::from("/a")), Some(PathBuf::from("/b")));
        let params = &event_json(&event)["params"];
        assert_eq!(params["name"], "dir_changed");
        assert_eq!(params["from"], "/a");
        assert_eq!(params["to"], "/b");
        assert!(params.get("kind").is_none());
    }

    #[test]
    fn a_change_to_or_from_no_entry_is_null_not_absent() {
        let event = HostEvent::new(EVENT_SELECTION_CHANGED, PathBuf::from("/"), Vec::new())
            .with_change(None, Some(PathBuf::from("/a")));
        let params = &event_json(&event)["params"];
        assert!(params["from"].is_null());
        assert_eq!(params["to"], "/a");
    }

    #[test]
    fn a_job_event_carries_the_kind_and_outcome() {
        let event = HostEvent::new(EVENT_JOB_FINISHED, PathBuf::from("/"), Vec::new())
            .with_job("copy", JobOutcome::Cancelled);
        let params = &event_json(&event)["params"];
        assert_eq!(params["name"], "job_finished");
        assert_eq!(params["kind"], "copy");
        assert_eq!(params["outcome"], "cancelled");
        assert!(params.get("from").is_none());
    }

    #[test]
    fn a_key_event_names_the_key_the_way_the_config_spells_it() {
        for (code, spelled) in [
            (KeyCode::Char('b'), "b"),
            (KeyCode::Char('B'), "B"),
            (KeyCode::Char(' '), "space"),
            (KeyCode::Enter, "enter"),
            (KeyCode::F(5), "f(5)"),
        ] {
            let event = HostEvent::new(EVENT_KEY, PathBuf::from("/"), Vec::new()).with_key(code);
            assert_eq!(event_json(&event)["params"]["key"], spelled);
        }
    }

    #[test]
    fn a_command_event_carries_the_command_word_and_its_arguments() {
        let event = HostEvent::new(EVENT_COMMAND, PathBuf::from("/"), Vec::new())
            .with_command("blame", vec!["a b".into(), "c".into()]);
        let params = &event_json(&event)["params"];
        assert_eq!(params["name"], "command");
        assert_eq!(params["command"], "blame");
        assert_eq!(params["args"], serde_json::json!(["a b", "c"]));
    }

    #[test]
    fn a_read_dir_request_parses_from_raw_json_text() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"read_dir","params":{"path":"/tmp"}}"#;
        match parse_line(line).unwrap() {
            Incoming::Request {
                id,
                request: PluginRequest::ReadDir { path },
            } => {
                assert_eq!(id, serde_json::json!(1));
                assert_eq!(path, PathBuf::from("/tmp"));
            }
            other => panic!("expected a read_dir request, got {other:?}"),
        }
    }

    #[test]
    fn a_get_state_request_needs_no_params() {
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"method":"get_state"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"get_state","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"get_state","params":null}"#,
        ] {
            match parse_line(line).unwrap() {
                Incoming::Request {
                    request: PluginRequest::GetState,
                    ..
                } => {}
                other => panic!("expected a get_state request for {line}, got {other:?}"),
            }
        }
    }

    #[test]
    fn run_action_takes_a_name_and_list_actions_takes_nothing() {
        let run = r#"{"jsonrpc":"2.0","id":1,"method":"run_action","params":{"name":"paste"}}"#;
        assert!(matches!(
            parse_line(run).unwrap(),
            Incoming::Request {
                request: PluginRequest::RunAction { ref name },
                ..
            } if name == "paste"
        ));
        let list = r#"{"jsonrpc":"2.0","id":2,"method":"list_actions"}"#;
        assert!(matches!(
            parse_line(list).unwrap(),
            Incoming::Request {
                request: PluginRequest::ListActions,
                ..
            }
        ));
        let nameless = r#"{"jsonrpc":"2.0","id":3,"method":"run_action","params":{}}"#;
        assert_eq!(
            parse_line(nameless).unwrap_err().id,
            Some(serde_json::json!(3))
        );
    }

    #[test]
    fn register_and_unregister_command_take_a_name() {
        let register =
            r#"{"jsonrpc":"2.0","id":1,"method":"register_command","params":{"name":"blame"}}"#;
        let unregister =
            r#"{"jsonrpc":"2.0","id":2,"method":"unregister_command","params":{"name":"blame"}}"#;
        assert!(matches!(
            parse_line(register).unwrap(),
            Incoming::Request {
                request: PluginRequest::RegisterCommand { ref name },
                ..
            } if name == "blame"
        ));
        assert!(matches!(
            parse_line(unregister).unwrap(),
            Incoming::Request {
                request: PluginRequest::UnregisterCommand { ref name },
                ..
            } if name == "blame"
        ));
    }

    #[test]
    fn register_command_ignores_params_it_does_not_know_and_needs_a_name() {
        let extra = r#"{"jsonrpc":"2.0","id":1,"method":"register_command","params":{"name":"x","description":"later"}}"#;
        assert!(parse_line(extra).is_ok());
        let missing = r#"{"jsonrpc":"2.0","id":7,"method":"register_command","params":{}}"#;
        assert_eq!(
            parse_line(missing).unwrap_err().id,
            Some(serde_json::json!(7))
        );
    }

    #[test]
    fn a_state_snapshot_serializes_to_the_documented_shape() {
        let snapshot = StateSnapshot {
            cwd: PathBuf::from("/home/x"),
            cursor: Some(PathBuf::from("/home/x/a.txt")),
            marks: vec![PathBuf::from("/home/x/b.txt")],
            show_hidden: true,
            sort: "size".into(),
            sort_reversed: true,
            view: "grid".into(),
        };
        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(value["cwd"], "/home/x");
        assert_eq!(value["cursor"], "/home/x/a.txt");
        assert_eq!(value["marks"], serde_json::json!(["/home/x/b.txt"]));
        assert_eq!(value["show_hidden"], true);
        assert_eq!(value["sort"], "size");
        assert_eq!(value["sort_reversed"], true);
        assert_eq!(value["view"], "grid");
    }

    #[test]
    fn an_empty_directory_has_a_null_cursor() {
        let value = serde_json::to_value(StateSnapshot::default()).unwrap();
        assert!(value["cursor"].is_null());
        assert_eq!(value["marks"], serde_json::json!([]));
    }

    #[test]
    fn a_rename_request_defaults_its_policy_to_abort() {
        let line =
            r#"{"jsonrpc":"2.0","id":2,"method":"rename","params":{"path":"/a","new_name":"b"}}"#;
        match parse_line(line).unwrap() {
            Incoming::Request {
                request: PluginRequest::Rename { policy, .. },
                ..
            } => {
                assert_eq!(policy, ConflictPolicy::Abort);
            }
            other => panic!("expected a rename request, got {other:?}"),
        }
    }

    #[test]
    fn a_copy_request_honors_an_explicit_policy() {
        let line = r#"{"jsonrpc":"2.0","id":3,"method":"copy",
            "params":{"src":"/a","dst":"/b","policy":"overwrite"}}"#;
        match parse_line(line).unwrap() {
            Incoming::Request {
                request: PluginRequest::Copy { policy, .. },
                ..
            } => {
                assert_eq!(policy, ConflictPolicy::Overwrite);
            }
            other => panic!("expected a copy request, got {other:?}"),
        }
    }

    #[test]
    fn a_log_notification_parses_without_an_id() {
        let line =
            r#"{"jsonrpc":"2.0","method":"log","params":{"level":"warn","message":"careful"}}"#;
        assert_eq!(
            parse_line(line).unwrap(),
            Incoming::Log {
                level: LogLevel::Warn,
                message: "careful".into()
            }
        );
    }

    #[test]
    fn a_log_notifications_level_defaults_to_info() {
        let line = r#"{"jsonrpc":"2.0","method":"log","params":{"message":"hi"}}"#;
        assert_eq!(
            parse_line(line).unwrap(),
            Incoming::Log {
                level: LogLevel::Info,
                message: "hi".into()
            }
        );
    }

    #[test]
    fn a_notification_that_is_not_log_is_ignored_rather_than_an_error() {
        let line = r#"{"jsonrpc":"2.0","method":"ping","params":{}}"#;
        assert_eq!(parse_line(line).unwrap(), Incoming::UnknownNotification);
    }

    #[test]
    fn an_unknown_method_on_a_request_carries_the_original_id_in_its_error() {
        let line = r#"{"jsonrpc":"2.0","id":"abc","method":"nonsense","params":{}}"#;
        let err = parse_line(line).unwrap_err();
        assert_eq!(err.id, Some(serde_json::json!("abc")));
        assert!(err.message.contains("nonsense"));
    }

    #[test]
    fn params_that_do_not_match_the_method_carry_the_original_id_in_their_error() {
        let line = r#"{"jsonrpc":"2.0","id":9,"method":"read_dir","params":{}}"#;
        let err = parse_line(line).unwrap_err();
        assert_eq!(err.id, Some(serde_json::json!(9)));
    }

    #[test]
    fn malformed_json_has_no_id_to_reply_to() {
        let err = parse_line("not json at all").unwrap_err();
        assert_eq!(err.id, None);
    }

    #[test]
    fn an_event_line_serializes_to_the_documented_shape() {
        let event = HostEvent::new(
            EVENT_KEY,
            PathBuf::from("/home/x"),
            vec![PathBuf::from("/home/x/a.txt")],
        );
        let text = event.to_line();
        assert!(text.ends_with('\n'));
        let value: Value = serde_json::from_str(text.trim_end()).unwrap();
        assert_eq!(value["method"], "event");
        assert_eq!(value["params"]["name"], "key");
        assert_eq!(value["params"]["api_version"], API_VERSION);
        assert_eq!(value["params"]["cwd"], "/home/x");
        assert_eq!(value["params"]["selection"][0], "/home/x/a.txt");
    }

    #[test]
    fn an_ok_response_carries_the_requests_id_and_no_error_field() {
        let id = serde_json::json!(42);
        let text = ok_response(&id, serde_json::json!({"outcome": "completed"}));
        let value: Value = serde_json::from_str(text.trim_end()).unwrap();
        assert_eq!(value["id"], 42);
        assert_eq!(value["result"]["outcome"], "completed");
        assert!(value.get("error").is_none());
    }

    #[test]
    fn an_error_response_carries_no_result_field() {
        let id = serde_json::json!(1);
        let text = err_response(&id, "boom");
        let value: Value = serde_json::from_str(text.trim_end()).unwrap();
        assert_eq!(value["error"]["message"], "boom");
        assert!(value.get("result").is_none());
    }

    proptest! {
        #[test]
        fn wire_entry_preserves_name_size_dir_flag_and_mode(
            name in "[a-zA-Z0-9_.-]{1,20}",
            size in any::<u64>(),
            is_dir in any::<bool>(),
            secs in 0u64..4_000_000_000,
            mode in any::<u32>(),
        ) {
            let entry = DirEntryInfo {
                name: name.clone(),
                path: PathBuf::from(&name),
                is_dir,
                size,
                modified: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)),
                mode: Some(mode),
            };

            let wire = WireEntry::from(&entry);

            prop_assert_eq!(wire.name, name);
            prop_assert_eq!(wire.size, size);
            prop_assert_eq!(wire.is_dir, is_dir);
            prop_assert_eq!(wire.modified, Some(secs));
            prop_assert_eq!(wire.mode, Some(mode));
        }

        #[test]
        fn wire_entry_reports_no_modified_time_when_the_source_had_none(
            name in "[a-zA-Z0-9_.-]{1,20}",
        ) {
            let entry = DirEntryInfo {
                name: name.clone(),
                path: PathBuf::from(&name),
                is_dir: false,
                size: 0,
                modified: None,
                mode: None,
            };

            prop_assert_eq!(WireEntry::from(&entry).modified, None);
        }
    }
}
