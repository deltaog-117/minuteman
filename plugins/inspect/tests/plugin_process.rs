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

//! The real `mman-inspect` program, played against by a test that stands in for the host: it
//! answers `stat` from the real disk, says when a count has ended, and reads the rows that come
//! back. This is the only place the stdin/stdout loop and the program's exit are exercised.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::Duration;

use plugin_api::API_VERSION;
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);

struct Plugin {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
}

impl Plugin {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mman-inspect"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the plugin program starts");
        let stdout = child.stdout.take().expect("piped");
        let stdin = child.stdin.take();
        let (tx, lines) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let value: Value = serde_json::from_str(&line).expect("each line is JSON");
                if tx.send(value).is_err() {
                    return;
                }
            }
        });
        let mut plugin = Self {
            child,
            stdin,
            lines,
        };
        plugin.send(&event("init", json!({})));
        plugin
    }

    fn send(&mut self, value: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        writeln!(stdin, "{value}").expect("the plugin is listening");
        stdin.flush().expect("flushed");
    }

    fn next(&self) -> Option<Value> {
        match self.lines.recv_timeout(WAIT) {
            Ok(value) => Some(value),
            Err(RecvTimeoutError::Timeout) => panic!("the plugin said nothing for {WAIT:?}"),
            Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    fn inspect(&mut self, path: Option<&Path>) {
        self.send(&event("inspect", json!({ "inspect": { "path": path } })));
    }

    fn reply(&mut self, request: &Value, result: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "id": request["id"], "result": result }));
    }

    fn finish(mut self) -> Vec<Value> {
        drop(self.stdin.take());
        let mut rest = Vec::new();
        while let Some(line) = self.next() {
            rest.push(line);
        }
        let status = self.child.wait().expect("it ends");
        assert!(status.success(), "the plugin ended badly: {status}");
        rest
    }
}

fn event(name: &str, extra: Value) -> Value {
    let mut params = json!({
        "name": name,
        "api_version": API_VERSION,
        "cwd": "/",
        "selection": [],
    });
    params
        .as_object_mut()
        .expect("an object")
        .extend(extra.as_object().expect("an object").clone());
    json!({ "jsonrpc": "2.0", "method": "event", "params": params })
}

/// What the host's `stat` says for a path on the disk.
fn stat_from_disk(path: &Path) -> Value {
    let meta = std::fs::symlink_metadata(path).expect("the path exists");
    let kind = if meta.is_dir() { "dir" } else { "file" };
    json!({ "kind": kind, "size": meta.len(), "local_path": path })
}

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mman-inspect-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rows_text(set: &Value) -> String {
    set["params"]["rows"].to_string()
}

#[test]
fn a_file_is_described_from_the_hosts_stat_and_the_program_ends_with_its_input() {
    let dir = scratch("file");
    let file = dir.join("note.txt");
    std::fs::write(&file, b"hello").unwrap();

    let mut plugin = Plugin::start();
    plugin.inspect(Some(&file));
    let request = plugin.next().expect("running");
    assert_eq!(request["method"], "stat");
    assert_eq!(Path::new(request["params"]["path"].as_str().unwrap()), file);
    plugin.reply(&request, stat_from_disk(&file));

    let set = plugin.next().expect("rows");
    assert_eq!(set["method"], "set_inspect");
    let text = rows_text(&set);
    assert!(
        text.contains("note.txt") && text.contains("File (text)"),
        "{text}"
    );
    assert!(text.contains("5 bytes"), "{text}");

    assert!(plugin.finish().is_empty(), "nothing else was said");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_folder_is_sent_counting_and_again_with_its_totals() {
    let dir = scratch("dir");
    let mut plugin = Plugin::start();
    plugin.inspect(Some(&dir));
    let request = plugin.next().expect("running");
    plugin.reply(&request, stat_from_disk(&dir));

    let first = plugin.next().expect("rows at once");
    assert!(rows_text(&first).contains("counting…"), "{first}");
    let tally = plugin.next().expect("a count was asked for");
    assert_eq!(tally["method"], "tally");
    let number = tally["params"]["tally"].clone();
    plugin.reply(&tally, json!({ "tally": number }));

    plugin.send(&event(
        "tally_done",
        json!({ "tally": {
            "tally": number, "files": 2, "dirs": 0, "bytes": 2048,
            "unreadable": 0, "ending": "complete"
        } }),
    ));
    let last = plugin.next().expect("the totals");
    assert!(
        rows_text(&last).contains("2 files, 0 folders, 2.0K"),
        "{last}"
    );

    plugin.finish();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn closing_the_panel_stops_the_count() {
    let dir = scratch("close");
    let mut plugin = Plugin::start();
    plugin.inspect(Some(&dir));
    let request = plugin.next().expect("running");
    plugin.reply(&request, stat_from_disk(&dir));
    let _rows = plugin.next().expect("rows");
    let tally = plugin.next().expect("a count was asked for");

    plugin.inspect(None);
    let cancel = plugin.next().expect("told to stop");
    assert_eq!(cancel["method"], "tally_cancel");
    assert_eq!(cancel["params"]["tally"], tally["params"]["tally"]);

    plugin.finish();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_host_speaking_another_api_version_is_refused_with_a_logged_reason() {
    let mut plugin = Plugin::start();
    let mut init = event("init", json!({}));
    init["params"]["api_version"] = json!(API_VERSION + 1);
    plugin.send(&init);
    let line = plugin.next().expect("it explains");
    assert_eq!(line["method"], "log");
    assert_eq!(line["params"]["level"], "error");
    // It has ended on its own, without waiting for the input to close.
    assert!(plugin.next().is_none());
}
