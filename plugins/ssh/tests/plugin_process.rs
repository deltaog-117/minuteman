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

//! The real `mman-ssh` program on the other end of a pipe: the line protocol, the order of its
//! replies, what it does with a line it cannot read, and how it ends. Nothing here needs a
//! server; the requests used are ones the plugin answers before it would connect anywhere.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::Duration;

use plugin_api::{API_VERSION, VfsFault, VfsRequest, request_to_line};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);

struct Plugin {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
}

impl Plugin {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mman-ssh"))
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
        plugin.write(&json!({ "jsonrpc": "2.0", "method": "event", "params": {
            "name": "init", "api_version": API_VERSION, "cwd": "/", "selection": [] } }));
        plugin
    }

    fn write(&mut self, value: &Value) {
        self.write_raw(&format!("{value}\n"));
    }

    fn write_raw(&mut self, text: &str) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        stdin
            .write_all(text.as_bytes())
            .expect("the plugin is listening");
        stdin.flush().expect("flushed");
    }

    fn next(&self) -> Value {
        match self.lines.recv_timeout(WAIT) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => panic!("no line from the plugin in {WAIT:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("the plugin ended"),
        }
    }

    fn fault(reply: &Value) -> VfsFault {
        serde_json::from_value(reply["error"]["data"].clone()).expect("a fault in the reply")
    }

    /// Closes the plugin's input and waits for it to end.
    fn finish(mut self) -> std::process::ExitStatus {
        self.stdin = None;
        self.child.wait().expect("the plugin ends")
    }
}

#[test]
fn each_request_is_answered_under_its_own_id() {
    let mut plugin = Plugin::start();
    for id in [7u64, 3, 11] {
        plugin.write_raw(&request_to_line(
            id,
            &VfsRequest::List {
                path: format!("/not/remote/{id}").into(),
            },
        ));
    }
    let mut answered: Vec<u64> = (0..3)
        .map(|_| {
            let reply = plugin.next();
            assert!(matches!(
                Plugin::fault(&reply),
                VfsFault::Unsupported { .. }
            ));
            reply["id"].as_u64().expect("an id")
        })
        .collect();
    answered.sort_unstable();
    assert_eq!(answered, [3, 7, 11]);
    assert!(plugin.finish().success());
}

#[test]
fn a_request_it_does_not_know_is_refused_not_left_to_time_out() {
    let mut plugin = Plugin::start();
    plugin.write(&json!({ "jsonrpc": "2.0", "id": 5, "method": "vfs",
        "params": { "op": "defragment", "path": "ssh://h/x" } }));
    let reply = plugin.next();
    assert_eq!(reply["id"], 5);
    assert!(matches!(
        Plugin::fault(&reply),
        VfsFault::Unsupported { .. }
    ));
    assert!(plugin.finish().success());
}

#[test]
fn lines_it_cannot_read_and_events_it_does_not_use_change_nothing() {
    let mut plugin = Plugin::start();
    plugin.write_raw("\n");
    plugin.write_raw("this is not json\n");
    plugin.write(&json!({ "jsonrpc": "2.0", "method": "event",
        "params": { "name": "dir_changed", "api_version": API_VERSION, "cwd": "/", "selection": [] } }));
    plugin.write_raw(&request_to_line(
        9,
        &VfsRequest::Metadata {
            path: "ssh:".into(),
            follow: true,
        },
    ));
    let reply = plugin.next();
    assert_eq!(reply["id"], 9);
    assert_eq!(Plugin::fault(&reply), VfsFault::NotFound);
    assert!(plugin.finish().success());
}

#[test]
fn a_host_that_speaks_another_api_version_is_told_and_left() {
    let mut plugin = Plugin::start();
    plugin.write(&json!({ "jsonrpc": "2.0", "method": "event", "params": {
        "name": "init", "api_version": API_VERSION + 1, "cwd": "/", "selection": [] } }));
    let said = plugin.next();
    assert_eq!(said["method"], "log");
    assert_eq!(said["params"]["level"], "error");
    assert!(plugin.finish().success());
}

#[test]
fn a_cancel_ends_a_request_stuck_connecting_long_before_its_timeout() {
    // A server that accepts and never speaks: `ssh` waits for its greeting until the connect
    // timeout (ten seconds), so the request is stuck in the connect.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = listener.local_addr().expect("bound").port();
    let held = thread::spawn(move || listener.accept().map(|(socket, _)| socket));

    let mut plugin = Plugin::start();
    plugin.write_raw(&request_to_line(
        4,
        &VfsRequest::List {
            path: format!("ssh://127.0.0.1:{port}/").into(),
        },
    ));
    let mut socket = held
        .join()
        .expect("accepted")
        .expect("the plugin connected");
    let asked = std::time::Instant::now();
    plugin.write_raw(&plugin_api::cancel_line(4));
    let reply = plugin.next();
    assert_eq!(reply["id"], 4);
    match Plugin::fault(&reply) {
        VfsFault::Io { message, .. } => assert_eq!(message, "cancelled"),
        other => panic!("expected the cancel's error, got {other:?}"),
    }
    assert!(asked.elapsed() < Duration::from_secs(5));
    // The `ssh` it started is killed with the attempt, not left to its own timeout: the server
    // sees its connection close (after the greeting `ssh` sends first) within a few seconds.
    socket
        .set_read_timeout(Some(Duration::from_secs(4)))
        .expect("a timeout");
    let mut greeting = [0; 512];
    let closed = loop {
        match socket.read(&mut greeting) {
            Ok(0) => break true,
            Ok(_) => {}
            Err(_) => break false,
        }
    };
    assert!(closed, "the cancelled attempt's ssh is still connected");
    // A cancel for a request already answered, or never made, is nothing.
    plugin.write_raw(&plugin_api::cancel_line(4));
    plugin.write_raw(&plugin_api::cancel_line(999));
    assert!(plugin.finish().success());
}
