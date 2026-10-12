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

//! The real `mman-wiring` program, played against by a test that stands in for the host: it sends
//! `canvas` events, reads the canvases and notes that come back and closes the pipe. This is the
//! only place the stdin/stdout loop, the tick and the program's exit are exercised together,
//! against how this machine's own storage is wired.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant};

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
        let mut child = Command::new(env!("CARGO_BIN_EXE_mman-wiring"))
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

    /// Sends a `canvas` event for the wiring diagram; `input` is the tagged object (`{"input": "key",
    /// ...}`), which the wire flattens beside `surface`.
    fn canvas(&mut self, input: Value) {
        let mut canvas = json!({ "surface": "wiring" });
        if let (Some(canvas), Some(input)) = (canvas.as_object_mut(), input.as_object()) {
            canvas.extend(input.clone());
        }
        self.send(&event("canvas", json!({ "canvas": canvas })));
    }

    /// The next line with this `method` whose params satisfy `wanted`, skipping others.
    fn next_where(&self, method: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        let until = Instant::now() + WAIT;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let value = self.lines.recv_timeout(left).expect("a line in time");
            if value["method"] == method && wanted(&value["params"]) {
                return value["params"].clone();
            }
        }
    }

    fn next(&self, method: &str) -> Value {
        self.next_where(method, |_| true)
    }
}

fn event(name: &str, extra: Value) -> Value {
    let mut params = json!({
        "name": name,
        "api_version": API_VERSION,
        "cwd": "/",
        "selection": [],
    });
    if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
        params.extend(extra.clone());
    }
    json!({"jsonrpc": "2.0", "method": "event", "params": params})
}

fn open() -> Value {
    json!({"input": "open", "width": 160, "height": 40, "here": "/"})
}

/// A view big enough for the boxes to be drawn at their full size, which is where a filesystem has
/// room to say whether the load fits.
fn open_big() -> Value {
    json!({"input": "open", "width": 400, "height": 150, "here": "/"})
}

/// The text of every `text` part of a canvas.
fn texts(set: &Value) -> Vec<String> {
    set["canvas"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["part"] == "text")
        .filter_map(|item| item["text"].as_str().map(String::from))
        .collect()
}

/// Whether the canvas has been drawn after the first look at the machine: it holds a frame, one
/// for each box.
fn has_boxes(set: &Value) -> bool {
    set["canvas"]["items"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["part"] == "frame"))
}

#[test]
fn opening_gets_a_canvas_of_this_machines_storage_that_claims_its_keys() {
    let mut plugin = Plugin::start();
    plugin.canvas(open());
    let set = plugin.next_where("set_canvas", has_boxes);
    assert_eq!(set["surface"], "wiring");
    assert_eq!(set["canvas"]["width"], 160);
    assert_eq!(set["canvas"]["height"], 40);
    let claims: Vec<&str> = set["canvas"]["claims"]
        .as_array()
        .expect("claims")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for key in ["enter", "p", "m", "e", "right"] {
        assert!(claims.contains(&key), "{key} in {claims:?}");
    }
    assert!(
        !claims.contains(&"esc") && !claims.contains(&"tab"),
        "those are the browser's"
    );
    assert!(
        set["canvas"]["hint"]
            .as_str()
            .is_some_and(|h| h.starts_with("wiring:"))
    );
}

#[test]
fn the_folder_the_browser_is_in_is_unfolded_and_chosen_and_enter_opens_it() {
    let mut plugin = Plugin::start();
    plugin.canvas(json!({"input": "open", "width": 160, "height": 40, "here": "/usr"}));
    plugin.next_where("set_canvas", |set| {
        texts(set).iter().any(|t| t.contains("usr"))
    });
    plugin.canvas(json!({"input": "key", "key": "enter"}));
    let note = plugin.next("canvas_note");
    assert_eq!(note["kind"], "goto");
    assert_eq!(note["path"], "/usr");
}

#[test]
fn what_the_browser_carries_is_judged_against_each_filesystem() {
    let mut plugin = Plugin::start();
    plugin.canvas(open_big());
    plugin.next_where("set_canvas", has_boxes);
    plugin.canvas(json!({
        "input": "context", "here": "/",
        "carrying": {"bytes": 1, "exact": true, "largest": 1, "source": "/nowhere"}
    }));
    plugin.next_where("set_canvas", |set| {
        texts(set)
            .iter()
            .any(|t| t.contains("fits") || t.contains("tight"))
    });
}

#[test]
fn a_resize_lays_out_again_for_the_new_size() {
    let mut plugin = Plugin::start();
    plugin.canvas(open());
    plugin.next_where("set_canvas", has_boxes);
    plugin.canvas(json!({"input": "resize", "width": 60, "height": 20}));
    let resized = plugin.next_where("set_canvas", |set| set["canvas"]["width"] == 60);
    assert_eq!(resized["canvas"]["height"], 20);
}

#[test]
fn a_closed_plugin_stays_quiet_and_the_program_ends_when_the_host_closes_the_pipe() {
    let mut plugin = Plugin::start();
    assert!(
        plugin
            .lines
            .recv_timeout(Duration::from_millis(1500))
            .is_err(),
        "nothing is sent before the view opens"
    );
    plugin.canvas(open());
    plugin.next("set_canvas");
    plugin.canvas(json!({"input": "close"}));
    // Whatever was in flight when the view closed is read, then nothing more comes.
    while plugin
        .lines
        .recv_timeout(Duration::from_millis(400))
        .is_ok()
    {}
    assert!(
        plugin
            .lines
            .recv_timeout(Duration::from_millis(1500))
            .is_err(),
        "a closed view sends nothing"
    );
    plugin.stdin = None;
    let until = Instant::now() + WAIT;
    loop {
        if let Some(status) = plugin.child.try_wait().expect("waitable") {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < until, "the program ends with its input");
        thread::sleep(Duration::from_millis(20));
    }
}
