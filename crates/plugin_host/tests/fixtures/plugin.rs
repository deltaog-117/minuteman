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

//! A tiny reference plugin, built only for `plugins`' own integration tests. It hand-writes JSON
//! text rather than reusing the crate's `protocol` module, so the test that drives it through
//! `PluginManager` is exercising the wire format itself — the same thing a plugin written in any
//! other language would have to get right — not two ends of the same Rust types agreeing with
//! themselves.
//!
//! On `init` it logs "ready pid=N". On `key` it reads the directory it was told is the cwd, logs how
//! many entries it found, asks the host to create a marker file next to the first selected path
//! (proving a plugin reaches real file operations, not just read-only queries), asks for the
//! browser state with `get_state` and logs its cursor, and logs "done". It also logs each
//! `dir_changed`, `selection_changed` and `job_finished` it is sent, as `name: from -> to` or
//! `job_finished: kind outcome`.
//!
//! `--register=WORD` (repeatable) asks the host at `init` for `:WORD`, and logs `register WORD: ok`
//! or the host's refusal; `--unregister-on-key=WORD` gives it up again on a `key`, logging
//! `unregister WORD: ok` or the refusal. A `command` event is logged as `command: WORD args...`.
//! `--list-actions` asks at `init` for the actions it may run and logs `actions: a,b,...`, and
//! `--run-action=NAME` (repeatable) then runs each, logging `run NAME: ok` or the refusal.
//! `--bench=METHOD:N` (repeatable) sends N back-to-back `METHOD` requests (`get_state`,
//! `list_actions` or `run_action`, which asks for `paste`) at `init`, then logs `bench METHOD done`.
//! Because it takes the next line on stdin as a request's reply, a test must wait for these logs
//! before sending it anything else.
//!
//! An `action` event is logged as `action: NAME hidden=BOOL`; with `--picker-on-action` the plugin
//! then asks for a picker (`picker: ok` or the host's refusal), sends one `picker_items` notification
//! and asks for a walk of the directory it was told (`walk: ok` or the refusal). It logs each
//! `picker_query` as `picker_query: PICKER GENERATION TEXT`, each `walk_batch` as `walk_batch: WALK
//! COUNT`, each `walk_done` as `walk_done: WALK OUTCOME truncated=BOOL`, and on `picker_closed`
//! logs `picker_closed: PICKER` and cancels its walk.
//!
//! Three flags make it end on purpose, so the tests can play a one-shot plugin and a plugin in a
//! crash loop: `--exit-on-init=N`, `--exit-after-key=N` and `--exit-after-event=N` exit with
//! status N right at `init`, after finishing a `key`, or after logging a change or job event.

use std::io::{self, BufRead, Write};

/// The status asked for by `--<flag>=N`, if that flag was given.
fn exit_code(flag: &str) -> Option<i32> {
    std::env::args()
        .find_map(|arg| arg.strip_prefix(&format!("--{flag}=")).map(str::to_string))
        .and_then(|code| code.parse().ok())
}

/// Events that arrived while a request waited for its reply, handled once it has one.
static PENDING: std::sync::Mutex<std::collections::VecDeque<String>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());

/// The next line to handle: an event set aside by `send_request` first, then stdin.
fn next_line(lines: &mut io::Lines<io::StdinLock<'_>>) -> Option<String> {
    let held = PENDING.lock().ok()?.pop_front();
    held.or_else(|| lines.next().and_then(Result::ok))
}

/// Answers a `vfs` request the way a plugin that serves paths would, with fixed replies: a path
/// with `missing` in it is not found, one with `die` in it ends the process without a reply, and
/// anything else gets an answer that names the operation and this process.
fn answer_vfs(stdout: &mut io::Stdout, request: &serde_json::Value) {
    let path = request["params"]["path"].as_str().unwrap_or("");
    let op = request["params"]["op"].as_str().unwrap_or("");
    if path.contains("die") {
        std::process::exit(3);
    }
    let line = if path.contains("missing") {
        serde_json::json!({ "jsonrpc": "2.0", "id": request["id"], "error": {
            "code": -32000, "message": "not found", "data": { "fault": "not_found" } } })
    } else {
        serde_json::json!({ "jsonrpc": "2.0", "id": request["id"],
            "result": { "op": op, "pid": std::process::id() } })
    };
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

/// Every value given as `--<flag>=VALUE`.
fn flag_values(flag: &str) -> Vec<String> {
    std::env::args()
        .filter_map(|arg| arg.strip_prefix(&format!("--{flag}=")).map(str::to_string))
        .collect()
}

fn main() {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut stdout = io::stdout();
    let mut next_id: u64 = 1;
    let serve_vfs = std::env::args().any(|arg| arg == "--serve-vfs");

    while let Some(line) = next_line(&mut lines) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if event["method"] == "vfs" {
            if serve_vfs {
                answer_vfs(&mut stdout, &event);
            }
            continue;
        }
        let Some(name) = event["params"]["name"].as_str() else {
            continue;
        };

        match name {
            "init" => {
                send_log(
                    &mut stdout,
                    "info",
                    &format!("ready pid={}", std::process::id()),
                );
                for word in flag_values("register") {
                    let result = send_request(
                        &mut stdout,
                        &mut lines,
                        &mut next_id,
                        "register_command",
                        serde_json::json!({ "name": word }),
                    );
                    let outcome = result.map_or_else(|e| e, |_| "ok".to_string());
                    send_log(&mut stdout, "info", &format!("register {word}: {outcome}"));
                }
                if std::env::args().any(|arg| arg == "--list-actions") {
                    let result = send_request(
                        &mut stdout,
                        &mut lines,
                        &mut next_id,
                        "list_actions",
                        serde_json::json!({}),
                    );
                    let names: Vec<String> = result
                        .ok()
                        .and_then(|v| v.as_array().cloned())
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect();
                    send_log(
                        &mut stdout,
                        "info",
                        &format!("actions: {}", names.join(",")),
                    );
                }
                for action in flag_values("run-action") {
                    let result = send_request(
                        &mut stdout,
                        &mut lines,
                        &mut next_id,
                        "run_action",
                        serde_json::json!({ "name": action }),
                    );
                    let outcome = result.map_or_else(|e| e, |_| "ok".to_string());
                    send_log(&mut stdout, "info", &format!("run {action}: {outcome}"));
                }
                // Timing is the test's job; here the plugin only keeps asking, back to back.
                for spec in flag_values("bench") {
                    let (method, count) = spec.split_once(':').unwrap_or((&spec, "1"));
                    let params = match method {
                        "run_action" => serde_json::json!({ "name": "paste" }),
                        _ => serde_json::json!({}),
                    };
                    for _ in 0..count.parse().unwrap_or(1) {
                        let _ = send_request(
                            &mut stdout,
                            &mut lines,
                            &mut next_id,
                            method,
                            params.clone(),
                        );
                    }
                    send_log(&mut stdout, "info", &format!("bench {method} done"));
                }
                if let Some(code) = exit_code("exit-on-init") {
                    std::process::exit(code);
                }
            }
            "command" => {
                let params = &event["params"];
                let args: Vec<&str> = params["args"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                send_log(
                    &mut stdout,
                    "info",
                    &format!(
                        "command: {} {}",
                        params["command"].as_str().unwrap_or("?"),
                        args.join(" ")
                    ),
                );
            }
            "key" => {
                for word in flag_values("unregister-on-key") {
                    let result = send_request(
                        &mut stdout,
                        &mut lines,
                        &mut next_id,
                        "unregister_command",
                        serde_json::json!({ "name": word }),
                    );
                    let outcome = result.map_or_else(|e| e, |_| "ok".to_string());
                    send_log(
                        &mut stdout,
                        "info",
                        &format!("unregister {word}: {outcome}"),
                    );
                }
                handle_key(&event, &mut stdout, &mut lines, &mut next_id);
                if let Some(code) = exit_code("exit-after-key") {
                    std::process::exit(code);
                }
            }
            "dir_changed" | "selection_changed" => {
                let params = &event["params"];
                send_log(
                    &mut stdout,
                    "info",
                    &format!(
                        "{name}: {} -> {}",
                        params["from"].as_str().unwrap_or("none"),
                        params["to"].as_str().unwrap_or("none")
                    ),
                );
                if let Some(code) = exit_code("exit-after-event") {
                    std::process::exit(code);
                }
            }
            "job_finished" => {
                let params = &event["params"];
                send_log(
                    &mut stdout,
                    "info",
                    &format!(
                        "job_finished: {} {}",
                        params["kind"].as_str().unwrap_or("?"),
                        params["outcome"].as_str().unwrap_or("?")
                    ),
                );
                if let Some(code) = exit_code("exit-after-event") {
                    std::process::exit(code);
                }
            }
            "action" => {
                let params = &event["params"];
                let hidden = params["show_hidden"].as_bool().unwrap_or(false);
                send_log(
                    &mut stdout,
                    "info",
                    &format!(
                        "action: {} hidden={hidden}",
                        params["action"].as_str().unwrap_or("?")
                    ),
                );
                if std::env::args().any(|arg| arg == "--picker-on-action") {
                    let shown = send_request(
                        &mut stdout,
                        &mut lines,
                        &mut next_id,
                        "show_picker",
                        serde_json::json!({ "picker": 1, "title": "fixture", "preview": false }),
                    );
                    let outcome = shown.map_or_else(|e| e, |_| "ok".to_string());
                    send_log(&mut stdout, "info", &format!("picker: {outcome}"));
                    send_notification(
                        &mut stdout,
                        "picker_items",
                        serde_json::json!({
                            "picker": 1,
                            "generation": 0,
                            "items": [{ "path": "/r/a.txt", "rel": "a.txt", "is_dir": false }],
                            "matches": 1,
                            "indexed": 1,
                            "indexing": "walking",
                            "settled": true,
                        }),
                    );
                    let walked = send_request(
                        &mut stdout,
                        &mut lines,
                        &mut next_id,
                        "walk",
                        serde_json::json!({
                            "walk": 7,
                            "root": params["cwd"],
                            "show_hidden": hidden,
                        }),
                    );
                    let outcome = walked.map_or_else(|e| e, |_| "ok".to_string());
                    send_log(&mut stdout, "info", &format!("walk: {outcome}"));
                }
            }
            "picker_query" => {
                let picker = &event["params"]["picker"];
                send_log(
                    &mut stdout,
                    "info",
                    &format!(
                        "picker_query: {} {} {}",
                        picker["picker"],
                        picker["generation"],
                        picker["text"].as_str().unwrap_or("")
                    ),
                );
            }
            "picker_closed" => {
                send_log(
                    &mut stdout,
                    "info",
                    &format!("picker_closed: {}", event["params"]["picker"]["picker"]),
                );
                send_notification(&mut stdout, "walk_cancel", serde_json::json!({ "walk": 7 }));
            }
            "walk_batch" => {
                let walk = &event["params"]["walk"];
                send_log(
                    &mut stdout,
                    "info",
                    &format!(
                        "walk_batch: {} {}",
                        walk["walk"],
                        walk["entries"].as_array().map_or(0, Vec::len)
                    ),
                );
            }
            "walk_done" => {
                let walk = &event["params"]["walk"];
                send_log(
                    &mut stdout,
                    "info",
                    &format!(
                        "walk_done: {} {} truncated={}",
                        walk["walk"],
                        walk["end"]["outcome"].as_str().unwrap_or("?"),
                        walk["end"]["truncated"]
                    ),
                );
            }
            _ => {}
        }
    }
}

fn handle_key(
    event: &serde_json::Value,
    stdout: &mut io::Stdout,
    lines: &mut io::Lines<io::StdinLock<'_>>,
    next_id: &mut u64,
) {
    let cwd = event["params"]["cwd"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let selection: Vec<String> = event["params"]["selection"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let entries = send_request(
        stdout,
        lines,
        next_id,
        "read_dir",
        serde_json::json!({ "path": cwd }),
    );
    let count = entries
        .as_ref()
        .ok()
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);
    send_log(stdout, "info", &format!("entries: {count}"));

    if let Some(target) = selection.first() {
        let marker = format!("{target}.touched-by-plugin");
        let result = send_request(
            stdout,
            lines,
            next_id,
            "create_file",
            serde_json::json!({ "path": marker }),
        );
        send_log(
            stdout,
            "info",
            if result.is_ok() {
                "touched: ok"
            } else {
                "touched: failed"
            },
        );
    }

    let state = send_request(stdout, lines, next_id, "get_state", serde_json::json!({}));
    let cursor = state
        .as_ref()
        .ok()
        .and_then(|v| v["cursor"].as_str())
        .unwrap_or("none");
    send_log(stdout, "info", &format!("cursor: {cursor}"));

    send_log(stdout, "info", "done");
}

fn send_notification(stdout: &mut io::Stdout, method: &str, params: serde_json::Value) {
    let line = serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params });
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

fn send_log(stdout: &mut io::Stdout, level: &str, message: &str) {
    let line = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "log",
        "params": { "level": level, "message": message },
    });
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

/// Sends one request and blocks for its reply, matched by `id`. An event that arrives first (a
/// host may write one before it has answered) is set aside for the main loop, as a real client
/// must, since nothing on the wire promises a reply comes before the next event.
fn send_request(
    stdout: &mut io::Stdout,
    lines: &mut io::Lines<io::StdinLock<'_>>,
    next_id: &mut u64,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id = *next_id;
    *next_id += 1;
    let request =
        serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let _ = writeln!(stdout, "{request}");
    let _ = stdout.flush();

    loop {
        let Some(Ok(line)) = lines.next() else {
            return Err("no reply".into());
        };
        let message: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if message["id"] != serde_json::json!(id) {
            if let Ok(mut held) = PENDING.lock() {
                held.push_back(line);
            }
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(error["message"].as_str().unwrap_or("error").to_string());
        }
        return Ok(message["result"].clone());
    }
}
