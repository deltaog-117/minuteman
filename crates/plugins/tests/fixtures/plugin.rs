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
//! On `init` it logs "ready". On `key` it reads the directory it was told is the cwd, logs how
//! many entries it found, asks the host to create a marker file next to the first selected path
//! (proving a plugin reaches real file operations, not just read-only queries), and logs "done".

use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut stdout = io::stdout();
    let mut next_id: u64 = 1;

    while let Some(Ok(line)) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(name) = event["params"]["name"].as_str() else {
            continue;
        };

        match name {
            "init" => send_log(&mut stdout, "info", "ready"),
            "key" => handle_key(&event, &mut stdout, &mut lines, &mut next_id),
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

    send_log(stdout, "info", "done");
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

/// Sends one request and blocks for its reply. This fixture only ever has one request in flight
/// at a time and never receives a new event while waiting, so reading the very next stdin line
/// back as the reply is safe here — a real client would still need to match replies by `id`
/// (nothing on the wire assumes strict ordering) since a host may interleave a new event with an
/// outstanding request's response.
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

    let Some(Ok(line)) = lines.next() else {
        return Err("no reply".into());
    };
    let response: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    if let Some(error) = response.get("error") {
        return Err(error["message"].as_str().unwrap_or("error").to_string());
    }
    Ok(response["result"].clone())
}
