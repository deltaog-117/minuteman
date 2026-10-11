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

use crate::EventParams;

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
