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

//! The finder as the host sees it: the real `mman-finder` binary on pipes, with this test playing
//! the host. Nothing here knows how the finder is built, only what it writes in answer to the
//! lines it is sent.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::{Duration, Instant};

use plugin_api::{
    API_VERSION, HostEvent, PickerIndexing, PickerUpdate, TreeEntry, WalkEnd, WalkOutcome,
};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(10);

struct Host {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
}

impl Host {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mman-finder"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start mman-finder");
        let stdin = child.stdin.take();
        let stdout: ChildStdout = child.stdout.take().expect("piped stdout");
        let (tx, lines) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let value: Value = serde_json::from_str(&line).expect("every line is JSON");
                if tx.send(value).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
        }
    }

    fn send(&mut self, event: &HostEvent) {
        let stdin = self.stdin.as_mut().expect("input still open");
        stdin.write_all(event.to_line().as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    /// The next line with `method`, skipping the ones before it.
    fn until(&mut self, method: &str) -> Value {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) if line["method"] == method => return line,
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    panic!("no {method} line arrived")
                }
            }
        }
    }

    /// The newest picker update that answers `generation`, is settled and sees the walk over.
    fn settled(&mut self, generation: u64) -> PickerUpdate {
        let deadline = Instant::now() + WAIT;
        loop {
            let line = self.until("picker_items");
            let update: PickerUpdate = serde_json::from_value(line["params"].clone()).unwrap();
            if update.generation == generation
                && update.settled
                && update.indexing != PickerIndexing::Walking
            {
                return update;
            }
            assert!(Instant::now() < deadline, "the finder never settled");
        }
    }

    fn finish(mut self) -> std::process::ExitStatus {
        drop(self.stdin.take());
        self.child.wait().expect("the finder ends")
    }
}

fn entry(rel: &str, is_dir: bool) -> TreeEntry {
    TreeEntry {
        path: PathBuf::from("/root").join(rel),
        rel: rel.to_string(),
        is_dir,
    }
}

fn rels(update: &PickerUpdate) -> Vec<&str> {
    update.items.iter().map(|item| item.rel.as_str()).collect()
}

#[test]
fn a_find_action_opens_a_picker_and_ranks_what_the_walk_sends() {
    let mut host = Host::start();
    host.send(&HostEvent::new("init", PathBuf::from("/root"), Vec::new()));
    host.send(&HostEvent::action(
        "find",
        PathBuf::from("/root"),
        Vec::new(),
        false,
    ));

    let show = host.until("show_picker");
    let picker = show["params"]["picker"].as_u64().unwrap();
    let walk = host.until("walk");
    let walk_number = walk["params"]["walk"].as_u64().unwrap();
    assert_eq!(walk["params"]["root"], "/root");

    host.send(&HostEvent::walk_batch(
        walk_number,
        vec![entry("src", true), entry("README.md", false)],
    ));
    host.send(&HostEvent::walk_batch(
        walk_number,
        vec![entry("src/main.rs", false), entry("src/lib.rs", false)],
    ));
    host.send(&HostEvent::walk_done(
        walk_number,
        WalkEnd {
            outcome: WalkOutcome::Done,
            truncated: false,
        },
    ));
    let everything = host.settled(0);
    assert_eq!(everything.picker, picker);
    assert_eq!((everything.indexed, everything.matches), (4, 4));
    assert_eq!(everything.indexing, PickerIndexing::Done);

    host.send(&HostEvent::picker_query(picker, 1, "main"));
    let narrowed = host.settled(1);
    assert_eq!(rels(&narrowed), ["src/main.rs"]);
    assert_eq!(narrowed.matches, 1);

    host.send(&HostEvent::picker_query(picker, 2, ""));
    assert_eq!(host.settled(2).matches, 4);

    host.send(&HostEvent::picker_closed(picker));
    let cancel = host.until("walk_cancel");
    assert_eq!(cancel["params"]["walk"], walk_number);

    assert!(host.finish().success(), "closing its input ends it quietly");
}

#[test]
fn an_action_it_does_not_provide_is_ignored() {
    let mut host = Host::start();
    host.send(&HostEvent::action(
        "sort",
        PathBuf::from("/root"),
        Vec::new(),
        false,
    ));
    host.send(&HostEvent::new("init", PathBuf::from("/root"), Vec::new()));
    assert!(host.lines.recv_timeout(Duration::from_millis(300)).is_err());
    assert!(host.finish().success());
}

#[test]
fn a_host_speaking_another_protocol_version_is_told_and_left() {
    let mut host = Host::start();
    let mut line = HostEvent::new("init", PathBuf::from("/root"), Vec::new()).to_line();
    line = line.replace(
        &format!("\"api_version\":{API_VERSION}"),
        &format!("\"api_version\":{}", API_VERSION + 1),
    );
    let stdin = host.stdin.as_mut().unwrap();
    stdin.write_all(line.as_bytes()).unwrap();
    stdin.flush().unwrap();
    let said = host.until("log");
    assert!(
        said["params"]["message"]
            .as_str()
            .unwrap()
            .contains("protocol")
    );
    assert!(!host.finish().success());
}
