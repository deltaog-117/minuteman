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

//! A worked example of a Minuteman plugin in Rust, run in the WebAssembly sandbox.
//!
//! Build it and put the module next to its `plugin.toml`:
//!
//! ```sh
//! rustup target add wasm32-wasip1
//! cargo build --release --target wasm32-wasip1
//! cp target/wasm32-wasip1/release/wasm_stats.wasm .
//! ```
//!
//! Copy this folder (with `wasm_stats.wasm` in it) to `~/.config/minuteman/plugins/wasm-stats/`,
//! switch it on in the plugin menu (`space p`), and run `:wstats`: the status bar shows how many
//! files and folders the browsed directory holds and how many bytes the files add up to. The host
//! must be built with the `wasm` feature (`cargo install --path crates/tui --features wasm`).
//!
//! `:wtouch NAME` asks the host to create a file, which the sandbox refuses: a module may read the
//! browsed folder and below it, and may not change anything. The refusal comes back as an error
//! reply and is logged, which is how a plugin should treat any request it is not allowed.
//!
//! The conversation is the same one a process plugin has: one JSON object per line, events in on
//! standard input, `log` notifications and requests out on standard output, each request answered
//! by one reply line carrying its `id`. The module has no folders, environment or network of its
//! own; everything it learns about the world is a request.

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

/// The plugin API this module is written for. The host sends its own in `init`, and a module
/// built for another version ends instead of misreading the messages.
const API_VERSION: u64 = 1;

/// A host that is gone, or a line that is not JSON: nothing a plugin can recover from.
#[derive(Debug)]
enum Fault {
    /// The host closed the conversation.
    Closed,
    /// A line the host sent could not be read.
    Unreadable(String),
    /// Standard output failed.
    Output(io::Error),
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "the host closed the connection"),
            Self::Unreadable(why) => write!(f, "unreadable line from the host: {why}"),
            Self::Output(why) => write!(f, "cannot write to the host: {why}"),
        }
    }
}

/// One end of the conversation. Events that arrive while a request waits for its reply are held
/// in `pending` and handled afterwards, so a reply is never taken for an event or the reverse.
struct Host<R, W> {
    input: R,
    output: W,
    pending: VecDeque<Value>,
    next_id: u64,
}

impl<R: BufRead, W: Write> Host<R, W> {
    fn new(input: R, output: W) -> Self {
        Self {
            input,
            output,
            pending: VecDeque::new(),
            next_id: 1,
        }
    }

    fn send(&mut self, message: &Value) -> Result<(), Fault> {
        writeln!(self.output, "{message}")
            .and_then(|()| self.output.flush())
            .map_err(Fault::Output)
    }

    fn log(&mut self, level: &str, message: &str) -> Result<(), Fault> {
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": "log",
            "params": { "level": level, "message": message },
        }))
    }

    /// The next non-blank line, parsed.
    fn read(&mut self) -> Result<Value, Fault> {
        loop {
            let mut line = String::new();
            let read = self
                .input
                .read_line(&mut line)
                .map_err(|why| Fault::Unreadable(why.to_string()))?;
            if read == 0 {
                return Err(Fault::Closed);
            }
            if !line.trim().is_empty() {
                return serde_json::from_str(&line)
                    .map_err(|why| Fault::Unreadable(why.to_string()));
            }
        }
    }

    /// The next message to handle: a held event first, then a new line.
    fn next_event(&mut self) -> Result<Value, Fault> {
        match self.pending.pop_front() {
            Some(held) => Ok(held),
            None => self.read(),
        }
    }

    /// Sends a request and returns its result, or the host's reason when it answered with an
    /// error. The outer `Result` is the conversation itself failing, the inner one a refusal.
    fn request(&mut self, method: &str, params: Value) -> Result<Result<Value, String>, Fault> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        loop {
            let message = self.read()?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                self.pending.push_back(message);
                continue;
            }
            if let Some(error) = message.get("error") {
                let reason = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("refused");
                return Ok(Err(reason.to_owned()));
            }
            return Ok(Ok(message.get("result").cloned().unwrap_or(Value::Null)));
        }
    }
}

/// `"3 files, 1 folders, 2048 bytes"` for the entries `read_dir` returned.
fn summarize(entries: &[Value]) -> String {
    let folders = entries
        .iter()
        .filter(|entry| entry["is_dir"].as_bool().unwrap_or(false))
        .count();
    let bytes: u64 = entries
        .iter()
        .filter(|entry| !entry["is_dir"].as_bool().unwrap_or(false))
        .filter_map(|entry| entry["size"].as_u64())
        .sum();
    format!(
        "{} files, {folders} folders, {bytes} bytes",
        entries.len() - folders
    )
}

fn stats<R: BufRead, W: Write>(host: &mut Host<R, W>, cwd: &str) -> Result<(), Fault> {
    match host.request("read_dir", json!({ "path": cwd }))? {
        Ok(Value::Array(entries)) => host.log("info", &summarize(&entries)),
        Ok(other) => host.log("error", &format!("read_dir returned {other}")),
        Err(reason) => host.log("warn", &format!("could not read {cwd}: {reason}")),
    }
}

fn touch<R: BufRead, W: Write>(host: &mut Host<R, W>, cwd: &str, name: &str) -> Result<(), Fault> {
    match host.request("create_file", json!({ "path": format!("{cwd}/{name}") }))? {
        Ok(_) => host.log("info", &format!("created {name}")),
        Err(reason) => host.log("warn", &format!("the sandbox said no: {reason}")),
    }
}

/// What a `command` event asks for. `:wtouch` without a name is explained, not guessed at.
fn on_command<R: BufRead, W: Write>(host: &mut Host<R, W>, params: &Value) -> Result<(), Fault> {
    let cwd = params["cwd"].as_str().unwrap_or(".");
    let first_arg = params["args"].get(0).and_then(Value::as_str);
    match (params["command"].as_str(), first_arg) {
        (Some("wstats"), _) => stats(host, cwd),
        (Some("wtouch"), Some(name)) => touch(host, cwd, name),
        (Some("wtouch"), None) => host.log("warn", "usage: :wtouch NAME"),
        _ => host.log("warn", "try :wstats or :wtouch NAME"),
    }
}

/// Handles one message. Returns `Ok(false)` when the module should end.
fn handle<R: BufRead, W: Write>(host: &mut Host<R, W>, message: &Value) -> Result<bool, Fault> {
    if message["method"] != "event" {
        return Ok(true);
    }
    let params = &message["params"];
    match params["name"].as_str() {
        Some("init") => {
            if params["api_version"].as_u64() != Some(API_VERSION) {
                host.log(
                    "error",
                    &format!(
                        "built for API {API_VERSION}, the host speaks {}",
                        params["api_version"]
                    ),
                )?;
                return Ok(false);
            }
            host.log("info", "wasm-stats ready")?;
        }
        Some("command") => on_command(host, params)?,
        _ => {}
    }
    Ok(true)
}

fn run<R: BufRead, W: Write>(host: &mut Host<R, W>) -> Result<(), Fault> {
    loop {
        let message = match host.next_event() {
            Ok(message) => message,
            // The host ending the conversation is how a plugin is asked to stop.
            Err(Fault::Closed) => return Ok(()),
            Err(other) => return Err(other),
        };
        if !handle(host, &message)? {
            return Ok(());
        }
    }
}

fn main() {
    let stdin = io::stdin();
    let mut host = Host::new(stdin.lock(), io::stdout().lock());
    if let Err(fault) = run(&mut host) {
        // stderr is not wired to anything in the sandbox, so the log line is the only way to say why.
        let _ = host.log("error", &fault.to_string());
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(input: &str) -> Vec<Value> {
        let mut output = Vec::new();
        let mut host = Host::new(input.as_bytes(), &mut output);
        run(&mut host).expect("a clean end");
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn summarize_counts_files_folders_and_the_bytes_of_files_only() {
        let entries = vec![
            json!({"is_dir": false, "size": 10}),
            json!({"is_dir": true, "size": 4096}),
            json!({"is_dir": false, "size": 5}),
        ];
        assert_eq!(summarize(&entries), "2 files, 1 folders, 15 bytes");
    }

    #[test]
    fn a_host_speaking_another_api_ends_the_module_with_a_logged_reason() {
        let sent = exchange(
            r#"{"jsonrpc":"2.0","method":"event","params":{"name":"init","api_version":99}}"#,
        );
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["params"]["level"], "error");
    }

    #[test]
    fn an_event_arriving_while_a_reply_is_awaited_is_handled_afterwards() {
        let sent = exchange(concat!(
            r#"{"jsonrpc":"2.0","method":"event","params":{"name":"command","command":"wstats","cwd":"/d"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"event","params":{"name":"command","command":"nope","cwd":"/d"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":1,"result":[{"is_dir":false,"size":7}]}"#,
            "\n",
        ));
        assert_eq!(sent[0]["method"], "read_dir");
        assert_eq!(sent[1]["params"]["message"], "1 files, 0 folders, 7 bytes");
        assert_eq!(sent[2]["params"]["level"], "warn");
    }

    #[test]
    fn a_refused_request_is_logged_as_a_warning_with_the_reason() {
        let sent = exchange(concat!(
            r#"{"jsonrpc":"2.0","method":"event","params":{"name":"command","command":"wtouch","args":["x"],"cwd":"/d"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":1,"error":{"message":"a sandboxed plugin cannot create files"}}"#,
            "\n",
        ));
        assert_eq!(sent[0]["method"], "create_file");
        assert!(
            sent[1]["params"]["message"]
                .as_str()
                .unwrap()
                .contains("cannot create files")
        );
    }
}
