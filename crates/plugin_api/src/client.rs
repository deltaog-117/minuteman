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

//! What a plugin written in Rust needs to talk to the host: build the lines it writes and read
//! the lines it gets. A plugin in any other language does the same with its own JSON library; the
//! shapes are the ones in this crate.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{EventParams, METHOD_VFS, METHOD_VFS_CANCEL, VfsRequest};

/// Serializes `value` to exactly one `\n`-terminated line.
pub(crate) fn line(value: &impl Serialize) -> String {
    let mut text = serde_json::to_string(value).expect("protocol values always serialize");
    text.push('\n');
    text
}

#[derive(Serialize)]
struct Request<'a, P> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: P,
}

#[derive(Serialize)]
struct Notification<'a, P> {
    jsonrpc: &'static str,
    method: &'a str,
    params: P,
}

/// A plugin -> host request line: the host answers it with one reply carrying the same `id`.
pub fn request_line(id: u64, method: &str, params: &impl Serialize) -> String {
    line(&Request {
        jsonrpc: "2.0",
        id,
        method,
        params,
    })
}

/// A plugin -> host notification line: nothing is sent back.
pub fn notification_line(method: &str, params: &impl Serialize) -> String {
    line(&Notification {
        jsonrpc: "2.0",
        method,
        params,
    })
}

/// One line the host wrote to a plugin.
#[derive(Debug, Clone)]
pub enum Incoming {
    Event(Box<EventParams>),
    /// The answer to a request the plugin sent: `error` holds the reason when it failed, and
    /// `result` what it returned when it did not.
    Reply {
        id: Value,
        error: Option<String>,
        result: Option<Value>,
    },
}

#[derive(Deserialize)]
struct EventLine {
    params: EventParams,
}

/// A request the host made of a plugin that provides [`crate::PROVIDES_VFS_ARCHIVE`]: the `id` to
/// reply under, and what is asked.
pub type VfsAsk = (Value, VfsRequest);

/// Why a [`METHOD_VFS`] request could not be understood. `id` is there when the line carried one,
/// so the plugin can still answer and the host does not wait for a reply that will not come.
#[derive(Debug, Clone, PartialEq)]
pub struct VfsAskError {
    pub id: Option<Value>,
    pub message: String,
}

/// Reads `text` as a [`METHOD_VFS`] request. `None` means the line is something else, for
/// [`parse_incoming`] to read.
///
/// A plugin that answers for paths tries this before [`parse_incoming`], which would take the
/// request for a reply because it has an `id`.
pub fn parse_vfs_request(text: &str) -> Option<Result<VfsAsk, VfsAskError>> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("method").and_then(Value::as_str) != Some(METHOD_VFS) {
        return None;
    }
    let id = value.get("id").cloned();
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    Some(match (id, serde_json::from_value(params)) {
        (Some(id), Ok(request)) => Ok((id, request)),
        (id, Err(e)) => Err(VfsAskError {
            id,
            message: format!("invalid vfs request: {e}"),
        }),
        (None, Ok(_)) => Err(VfsAskError {
            id: None,
            message: "a vfs request with no id".to_string(),
        }),
    })
}

/// Reads `text` as a [`METHOD_VFS_CANCEL`] notification and returns the `id` of the request it
/// gives up on, or `None` for any other line.
pub fn parse_vfs_cancel(text: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("method").and_then(Value::as_str) != Some(METHOD_VFS_CANCEL) {
        return None;
    }
    value.get("params")?.get("id").cloned()
}

/// Reads one line from the host, or says why it is neither an event nor a reply.
pub fn parse_incoming(text: &str) -> Result<Incoming, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("malformed JSON: {e}"))?;
    if value.get("method").and_then(Value::as_str) == Some("event") {
        let event: EventLine =
            serde_json::from_value(value).map_err(|e| format!("invalid event: {e}"))?;
        return Ok(Incoming::Event(Box::new(event.params)));
    }
    match value.get("id") {
        Some(id) => Ok(Incoming::Reply {
            id: id.clone(),
            error: value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string),
            result: value.get("result").cloned(),
        }),
        None => Err("neither an event nor a reply".to_string()),
    }
}
