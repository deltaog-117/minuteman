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
//!   `sort`, with `list_actions` naming them, `show_picker`, which opens the list the interface
//!   draws for the plugin, and `walk`, which has the tree below a folder listed through whatever
//!   serves the browsed paths)
//!   gets exactly one reply line carrying that same `id`; a `log` notification (no `id`) is
//!   fire-and-forget and surfaces as the status bar message, and so are `picker_items` (what the
//!   picker lists), `picker_close`, `walk_cancel`, `set_view` (what fills a slot of the
//!   interface, such as a status bar segment or a mark beside each row) and `set_preview` (the
//!   document the preview column shows). A `read_file` request reads part of a file through
//!   whatever serves the browsed paths.
//! - host -> plugin events for these are `action` (the user triggered an action the plugin
//!   provides), `picker_query` and `picker_closed` (what is typed into its picker, and its end)
//!   `walk_batch` and `walk_done` (what a walk found, and its end) and `preview` (the file the
//!   preview column is about, sent to the plugin that provides `preview`). The shapes are in
//!   `plugin_api`.
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

pub use plugin_api::{
    API_VERSION, ArchiveDone, ArchiveProgress, ArchiveStatus, ArchiveWork, Change, EVENT_ACTION,
    EVENT_COMMAND, EVENT_DIR_CHANGED, EVENT_INIT, EVENT_JOB_FINISHED, EVENT_KEY,
    EVENT_PICKER_CLOSED, EVENT_PICKER_QUERY, EVENT_SELECTION_CHANGED, EVENT_WALK_BATCH,
    EVENT_WALK_DONE, EventKind, Glyph, HostEvent, Job, JobOutcome, Line, LogLevel, PickerClose,
    PickerIndexing, PickerSpec, PickerUpdate, ReadFile, SetInspect, SetPreview, SetView, Slot,
    Span, Stat, StateSnapshot, TallyCancel, TallyDone, TallyEnding, TallyRequest, Tone, TreeEntry,
    View, WalkCancel, WalkEnd, WalkOutcome, WalkRequest, WireEntry,
};
use plugin_api::{
    METHOD_ARCHIVE_DONE, METHOD_ARCHIVE_PROGRESS, METHOD_PICKER_CLOSE, METHOD_PICKER_ITEMS,
    METHOD_READ_FILE, METHOD_SET_INSPECT, METHOD_SET_PREVIEW, METHOD_SET_VIEW, METHOD_SHOW_PICKER,
    METHOD_STAT, METHOD_TALLY, METHOD_TALLY_CANCEL, METHOD_WALK, METHOD_WALK_CANCEL,
};

/// Spells `code` the way a config file does, so a plugin bound to several keys can tell which one
/// fired. Keys the config cannot name come out as their lowercase variant name.
pub fn key_name(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// The one part of building an event that needs the terminal's key type, kept here so
/// `plugin_api` stays free of it.
pub trait HostEventKey {
    /// Records which key fired this event (see `key_name`).
    fn with_key(self, code: KeyCode) -> Self;
}

impl HostEventKey for HostEvent {
    fn with_key(self, code: KeyCode) -> Self {
        self.with_key_name(key_name(code))
    }
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
    /// Opens the picker the interface draws, with a list this plugin fills.
    ShowPicker(PickerSpec),
    /// Starts listing everything below a folder, through whatever serves the browsed paths.
    Walk(WalkRequest),
    /// Reads part of a file through whatever serves the browsed paths.
    ReadFile(ReadFile),
    /// Describes one entry through whatever serves the browsed paths.
    Stat(Stat),
    /// Starts counting what is below a folder, through whatever serves the browsed paths.
    Tally(TallyRequest),
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
            METHOD_SHOW_PICKER => serde_json::from_value(params)
                .map(Self::ShowPicker)
                .map_err(bad),
            METHOD_WALK => serde_json::from_value(params).map(Self::Walk).map_err(bad),
            METHOD_READ_FILE => serde_json::from_value(params)
                .map(Self::ReadFile)
                .map_err(bad),
            METHOD_STAT => serde_json::from_value(params).map(Self::Stat).map_err(bad),
            METHOD_TALLY => serde_json::from_value(params).map(Self::Tally).map_err(bad),
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
    /// The plugin replaced what its picker lists.
    PickerItems(PickerUpdate),
    /// The plugin closed its picker.
    PickerClose(PickerClose),
    /// The plugin stopped a walk.
    WalkCancel(WalkCancel),
    /// The plugin stopped a count.
    TallyCancel(TallyCancel),
    /// The plugin filled or emptied a slot of the interface; already cleaned by
    /// `SetView::checked`.
    SetView(SetView),
    /// The plugin gave the preview column a document; already cleaned by
    /// `SetPreview::checked`.
    SetPreview(SetPreview),
    /// The plugin gave the Inspect panel its rows; already cleaned by `SetInspect::checked`.
    SetInspect(SetInspect),
    /// The plugin reached another entry of an archive request; already cleaned by
    /// `ArchiveProgress::checked`.
    ArchiveProgress(ArchiveProgress),
    /// The plugin ended an archive request; already cleaned by `ArchiveDone::checked`.
    ArchiveDone(ArchiveDone),
    /// A well-formed notification (no `id`) whose method isn't one of the above — ignored rather than
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

    if raw.id.is_none() {
        let bad = |e: serde_json::Error| LineError {
            id: None,
            message: format!("invalid params for '{}': {e}", raw.method),
        };
        match raw.method.as_str() {
            METHOD_PICKER_ITEMS => {
                return serde_json::from_value(raw.params)
                    .map(Incoming::PickerItems)
                    .map_err(bad);
            }
            METHOD_PICKER_CLOSE => {
                return serde_json::from_value(raw.params)
                    .map(Incoming::PickerClose)
                    .map_err(bad);
            }
            METHOD_WALK_CANCEL => {
                return serde_json::from_value(raw.params)
                    .map(Incoming::WalkCancel)
                    .map_err(bad);
            }
            METHOD_TALLY_CANCEL => {
                return serde_json::from_value(raw.params)
                    .map(Incoming::TallyCancel)
                    .map_err(bad);
            }
            METHOD_SET_VIEW => {
                return serde_json::from_value::<SetView>(raw.params)
                    .map_err(bad)?
                    .checked()
                    .map(Incoming::SetView)
                    .map_err(|e| LineError {
                        id: None,
                        message: format!("refused '{}': {e}", raw.method),
                    });
            }
            METHOD_SET_PREVIEW => {
                return serde_json::from_value::<SetPreview>(raw.params)
                    .map_err(bad)?
                    .checked()
                    .map(Incoming::SetPreview)
                    .map_err(|e| LineError {
                        id: None,
                        message: format!("refused '{}': {e}", raw.method),
                    });
            }
            METHOD_SET_INSPECT => {
                return serde_json::from_value::<SetInspect>(raw.params)
                    .map_err(bad)?
                    .checked()
                    .map(Incoming::SetInspect)
                    .map_err(|e| LineError {
                        id: None,
                        message: format!("refused '{}': {e}", raw.method),
                    });
            }
            METHOD_ARCHIVE_PROGRESS => {
                return serde_json::from_value::<ArchiveProgress>(raw.params)
                    .map(|progress| Incoming::ArchiveProgress(progress.checked()))
                    .map_err(bad);
            }
            METHOD_ARCHIVE_DONE => {
                return serde_json::from_value::<ArchiveDone>(raw.params)
                    .map_err(bad)?
                    .checked()
                    .map(Incoming::ArchiveDone)
                    .map_err(|e| LineError {
                        id: None,
                        message: format!("refused '{}': {e}", raw.method),
                    });
            }
            _ => {}
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use shared::DirEntryInfo;

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

    #[test]
    fn show_picker_and_walk_requests_parse_into_their_typed_forms() {
        let show = r#"{"jsonrpc":"2.0","id":1,"method":"show_picker","params":{"picker":3,"title":"find","preview":true}}"#;
        match parse_line(show).unwrap() {
            Incoming::Request {
                request: PluginRequest::ShowPicker(spec),
                ..
            } => assert_eq!(
                (spec.picker, spec.title.as_str(), spec.preview),
                (3, "find", true)
            ),
            other => panic!("expected show_picker, got {other:?}"),
        }
        let walk = r#"{"jsonrpc":"2.0","id":2,"method":"walk","params":{"walk":4,"root":"/r","show_hidden":true}}"#;
        match parse_line(walk).unwrap() {
            Incoming::Request {
                request: PluginRequest::Walk(request),
                ..
            } => assert_eq!(
                (request.walk, request.root, request.show_hidden),
                (4, PathBuf::from("/r"), true)
            ),
            other => panic!("expected walk, got {other:?}"),
        }
    }

    #[test]
    fn picker_and_walk_notifications_need_no_id_and_get_no_reply() {
        let items = r#"{"jsonrpc":"2.0","method":"picker_items","params":{"picker":1,"generation":2,"items":[{"path":"/r/a","rel":"a","is_dir":false}],"matches":1,"indexed":5,"indexing":"done","settled":true}}"#;
        match parse_line(items).unwrap() {
            Incoming::PickerItems(update) => {
                assert_eq!(
                    (update.picker, update.generation, update.indexed),
                    (1, 2, 5)
                );
                assert_eq!(update.items[0].rel, "a");
            }
            other => panic!("expected picker_items, got {other:?}"),
        }
        assert!(matches!(
            parse_line(r#"{"jsonrpc":"2.0","method":"picker_close","params":{"picker":9}}"#)
                .unwrap(),
            Incoming::PickerClose(close) if close.picker == 9
        ));
        assert!(matches!(
            parse_line(r#"{"jsonrpc":"2.0","method":"walk_cancel","params":{"walk":6}}"#)
                .unwrap(),
            Incoming::WalkCancel(cancel) if cancel.walk == 6
        ));
    }

    #[test]
    fn a_malformed_picker_notification_is_an_error_without_an_id_to_answer() {
        let error =
            parse_line(r#"{"jsonrpc":"2.0","method":"picker_items","params":{"picker":"x"}}"#)
                .unwrap_err();
        assert_eq!(error.id, None);
        assert!(error.message.contains("picker_items"), "{}", error.message);
    }

    #[test]
    fn set_view_is_a_notification_cleaned_before_the_host_sees_it() {
        let line = r#"{"jsonrpc":"2.0","method":"set_view","params":{"slot":"status_right","view":{"lines":[{"spans":[{"text":"a\u001b[2Jb","tone":"accent"}]}]}}}"#;
        match parse_line(line).unwrap() {
            Incoming::SetView(set) => {
                assert_eq!(set.slot, Slot::StatusRight);
                let text = &set.view.unwrap().lines[0].spans[0].text;
                assert_eq!(text, "a[2Jb");
            }
            other => panic!("expected set_view, got {other:?}"),
        }
        assert!(matches!(
            parse_line(r#"{"jsonrpc":"2.0","method":"set_view","params":{"slot":"status_entry"}}"#)
                .unwrap(),
            Incoming::SetView(SetView { view: None, .. })
        ));
    }

    #[test]
    fn a_set_view_that_cannot_be_shown_is_an_error_not_a_crash() {
        let unknown =
            parse_line(r#"{"jsonrpc":"2.0","method":"set_view","params":{"slot":"nowhere"}}"#)
                .unwrap_err();
        assert!(unknown.message.contains("set_view"), "{}", unknown.message);
        let two_lines = parse_line(
            r#"{"jsonrpc":"2.0","method":"set_view","params":{"slot":"status_entry","view":{"lines":[{"spans":[]},{"spans":[]}]}}}"#,
        )
        .unwrap_err();
        assert_eq!(two_lines.id, None);
        assert!(
            two_lines.message.contains("at most 1"),
            "{}",
            two_lines.message
        );
    }

    #[test]
    fn set_preview_is_a_notification_cleaned_before_the_host_sees_it() {
        let line = r#"{"jsonrpc":"2.0","method":"set_preview","params":{"path":"/a.rs","content":{"type":"text","lines":[[["keyword","f\u001bn"],["plain"," x"]]]}}}"#;
        match parse_line(line).unwrap() {
            Incoming::SetPreview(set) => {
                assert_eq!(set.path, PathBuf::from("/a.rs"));
                match set.content {
                    plugin_api::PreviewContent::Text { lines } => {
                        assert_eq!(lines[0][0].1, "fn");
                        assert_eq!(lines[0][1].1, " x");
                    }
                    other => panic!("expected text, got {other:?}"),
                }
            }
            other => panic!("expected set_preview, got {other:?}"),
        }
    }

    #[test]
    fn a_set_preview_that_cannot_be_shown_is_an_error_not_a_crash() {
        let unknown = parse_line(
            r#"{"jsonrpc":"2.0","method":"set_preview","params":{"path":"/a","content":{"type":"nope"}}}"#,
        )
        .unwrap_err();
        assert_eq!(unknown.id, None);
        assert!(
            unknown.message.contains("set_preview"),
            "{}",
            unknown.message
        );
        let over_the_limit = format!(
            r#"{{"jsonrpc":"2.0","method":"set_preview","params":{{"path":"/b","content":{{"type":"hex","bytes":"{}","total":1}}}}}}"#,
            "00".repeat(plugin_api::MAX_HEX_BYTES + 1)
        );
        let refused = parse_line(&over_the_limit).unwrap_err();
        assert!(refused.message.contains("refused"), "{}", refused.message);
    }

    #[test]
    fn a_read_file_request_parses_with_the_offset_optional() {
        let line =
            r#"{"jsonrpc":"2.0","id":4,"method":"read_file","params":{"path":"/a","len":10}}"#;
        match parse_line(line).unwrap() {
            Incoming::Request {
                id,
                request: PluginRequest::ReadFile(read),
            } => {
                assert_eq!(id, serde_json::json!(4));
                assert_eq!((read.path, read.offset, read.len), ("/a".into(), 0, 10));
            }
            other => panic!("expected a read_file request, got {other:?}"),
        }
        let missing =
            parse_line(r#"{"jsonrpc":"2.0","id":5,"method":"read_file","params":{"path":"/a"}}"#)
                .unwrap_err();
        assert_eq!(missing.id, Some(serde_json::json!(5)));
    }

    #[test]
    fn a_stat_request_parses_and_a_pathless_one_is_answered_with_an_error() {
        let line = r#"{"jsonrpc":"2.0","id":6,"method":"stat","params":{"path":"/a"}}"#;
        match parse_line(line).unwrap() {
            Incoming::Request {
                request: PluginRequest::Stat(stat),
                ..
            } => assert_eq!(stat.path, PathBuf::from("/a")),
            other => panic!("expected a stat request, got {other:?}"),
        }
        let missing =
            parse_line(r#"{"jsonrpc":"2.0","id":7,"method":"stat","params":{}}"#).unwrap_err();
        assert_eq!(missing.id, Some(serde_json::json!(7)));
    }

    #[test]
    fn a_tally_request_and_its_cancel_parse() {
        let line = r#"{"jsonrpc":"2.0","id":8,"method":"tally","params":{"tally":3,"root":"/a"}}"#;
        match parse_line(line).unwrap() {
            Incoming::Request {
                request: PluginRequest::Tally(tally),
                ..
            } => assert_eq!((tally.tally, tally.root), (3, PathBuf::from("/a"))),
            other => panic!("expected a tally request, got {other:?}"),
        }
        let cancel = r#"{"jsonrpc":"2.0","method":"tally_cancel","params":{"tally":3}}"#;
        assert_eq!(
            parse_line(cancel).unwrap(),
            Incoming::TallyCancel(TallyCancel { tally: 3 })
        );
    }

    #[test]
    fn set_inspect_is_a_notification_cleaned_before_the_host_sees_it() {
        let line = r#"{"jsonrpc":"2.0","method":"set_inspect","params":{"path":"/a","rows":[["Na\u001bme","x\u202ey"]]}}"#;
        match parse_line(line).unwrap() {
            Incoming::SetInspect(set) => assert_eq!(set.rows, [("Name".into(), "xy".into())]),
            other => panic!("expected set_inspect, got {other:?}"),
        }
        let many = format!(
            r#"{{"jsonrpc":"2.0","method":"set_inspect","params":{{"path":"/a","rows":[{}]}}}}"#,
            vec![r#"["a","b"]"#; 33].join(",")
        );
        let refused = parse_line(&many).unwrap_err();
        assert!(refused.message.contains("refused"), "{}", refused.message);
    }

    #[test]
    fn a_key_event_names_the_key_the_same_through_the_extension_as_before() {
        let event =
            HostEvent::new(EVENT_KEY, PathBuf::from("/"), Vec::new()).with_key(KeyCode::Char(' '));
        assert_eq!(event_json(&event)["params"]["key"], "space");
    }
}
