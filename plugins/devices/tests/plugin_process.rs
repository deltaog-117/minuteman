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

//! The real `mman-devices` program, played against by a test that stands in for the host: it
//! sends `devices` events, reads the lists and notes that come back and closes the pipe. This is
//! the only place the stdin/stdout loop, the threads and the program's exit are exercised
//! together, against this machine's own `/proc`.

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
        let mut child = Command::new(env!("CARGO_BIN_EXE_mman-devices"))
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

    /// The next line with this `method`, skipping others.
    fn next(&self, method: &str) -> Value {
        let until = Instant::now() + WAIT;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let value = self.lines.recv_timeout(left).expect("a line in time");
            if value["method"] == method {
                return value["params"].clone();
            }
        }
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

fn devices(command: Value) -> Value {
    event("devices", json!({ "devices": command }))
}

#[test]
fn watching_gets_this_machines_devices_and_each_has_a_node_and_a_kind() {
    let mut plugin = Plugin::start();
    plugin.send(&devices(json!({"command": "watch", "on": true})));
    let set = plugin.next("set_devices");
    let list = set["devices"].as_array().expect("a list");
    assert!(!list.is_empty(), "every machine has a root filesystem");
    for device in list {
        assert!(device["node"].as_str().is_some_and(|node| !node.is_empty()));
        assert!(["internal", "removable", "network"].contains(&device["kind"].as_str().unwrap()));
    }
    assert_eq!(set["busy"], false);
}

#[test]
fn an_action_on_a_device_that_is_not_there_is_answered_with_a_note() {
    let mut plugin = Plugin::start();
    plugin.send(&devices(json!({"command": "watch", "on": true})));
    plugin.next("set_devices");
    plugin.send(&devices(
        json!({"command": "eject", "node": "no-such-disk9"}),
    ));
    let note = plugin.next("devices_note");
    assert_eq!(note["kind"], "status");
    assert_eq!(note["text"], "no-such-disk9 is no longer there");
}

#[test]
fn an_unwatched_plugin_stays_quiet_and_the_program_ends_when_the_host_closes_the_pipe() {
    let mut plugin = Plugin::start();
    assert!(
        plugin
            .lines
            .recv_timeout(Duration::from_millis(1500))
            .is_err(),
        "nothing is sent before a watch"
    );
    plugin.stdin = None;
    let until = Instant::now() + WAIT;
    let status = loop {
        if let Some(status) = plugin.child.try_wait().expect("waitable") {
            break status;
        }
        assert!(Instant::now() < until, "the program did not end");
        thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success());
}

#[test]
fn a_host_speaking_another_api_version_is_refused_with_a_log_and_an_end() {
    let mut plugin = Plugin::start();
    let mut init = event("init", json!({}));
    init["params"]["api_version"] = json!(API_VERSION + 1);
    plugin.send(&init);
    assert_eq!(plugin.next("log")["level"], "error");
}
