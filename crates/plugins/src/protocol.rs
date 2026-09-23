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
//!   plugin's configured key is pressed) plus the current directory and selection.
//! - plugin -> host: a request with an `id` (`read_dir`, `copy`, `mv`, `delete`, `create_dir`,
//!   `create_file`, `touch`, `rename` — the same `file_ops` orchestration the built-in keys use)
//!   gets exactly one reply line carrying that same `id`; a `log` notification (no `id`) is
//!   fire-and-forget and surfaces as the status bar message.
//!
//! Deliberately close to JSON-RPC 2.0 rather than a bespoke shape, and line-delimited rather
//! than LSP's `Content-Length` framing, so a plugin author in any language needs nothing more
//! than "read a line, parse JSON, print a line" — no header parser, no schema compiler, no
//! build step.

use std::path::PathBuf;

use file_ops::ConflictPolicy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared::DirEntryInfo;

/// Bumped only for a breaking change to the shapes below; sent with every event so a plugin can
/// check it against what it was built for and refuse to run rather than misbehave silently.
pub const API_VERSION: u32 = 1;

/// The event name a plugin gets exactly once, right after its process starts.
pub const EVENT_INIT: &str = "init";
/// The event name a plugin gets when its configured key is pressed.
pub const EVENT_KEY: &str = "key";

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
            },
        }
    }

    /// Serializes to exactly one `\n`-terminated line, ready to write to the plugin's stdin.
    pub fn to_line(&self) -> String {
        line(self)
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
