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

//! The real `mman-archive` program, played against by a test that stands in for the host: it
//! sends `archive` events, reads the progress and results that come back and looks at the disk.
//! This is the only place the stdin/stdout loop, the threads and the program's exit are exercised
//! together.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::Duration;

use plugin_api::API_VERSION;
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(30);

struct Plugin {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
    seen: Vec<String>,
}

impl Plugin {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mman-archive"))
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
            seen: Vec::new(),
        };
        plugin.send(&event("init", json!({})));
        plugin
    }

    fn send(&mut self, value: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        writeln!(stdin, "{value}").expect("the plugin is listening");
        stdin.flush().expect("flushed");
    }

    fn next(&mut self) -> Option<Value> {
        match self.lines.recv_timeout(WAIT) {
            Ok(value) => {
                self.seen.push(value.to_string());
                Some(value)
            }
            Err(RecvTimeoutError::Timeout) => panic!("the plugin said nothing for {WAIT:?}"),
            Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    /// Reads lines until the result of request `job`, returning it and the progress seen first.
    fn until_done(&mut self, job: u64) -> (Value, Vec<Value>) {
        let mut progress = Vec::new();
        loop {
            let line = self
                .next()
                .expect("the result arrives before the program ends");
            match line["method"].as_str() {
                Some("archive_progress") => progress.push(line["params"].clone()),
                Some("archive_done") => {
                    assert_eq!(line["params"]["job"], job);
                    return (line["params"].clone(), progress);
                }
                other => panic!("unexpected line {other:?}: {line}"),
            }
        }
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

fn archive_event(job: u64, work: Value) -> Value {
    event(
        "archive",
        json!({ "archive": { "job": job, "work": work } }),
    )
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mman-archive-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn compress_work(source: &Path, out: &Path, format: &str, password: Option<&str>) -> Value {
    let mut work = json!({
        "op": "compress",
        "jobs": [{ "sources": [source], "out": out }],
        "format": format,
        "level": "normal",
        "policy": "abort",
    });
    if let Some(password) = password {
        work["password"] = json!(password);
    }
    work
}

fn extract_work(archive: &Path, dest: &Path, password: Option<&str>) -> Value {
    let mut work = json!({
        "op": "extract",
        "jobs": [{ "archive": archive, "dest": dest }],
        "policy": "abort",
    });
    if let Some(password) = password {
        work["password"] = json!(password);
    }
    work
}

#[test]
fn a_folder_is_packed_and_unpacked_and_the_program_ends_with_its_input() {
    let dir = scratch("round");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.txt"), "alpha").unwrap();
    let (tar, dest) = (dir.join("src.tar.gz"), dir.join("out"));

    let mut plugin = Plugin::start();
    plugin.send(&archive_event(
        1,
        compress_work(&dir.join("src"), &tar, "tar_gz", None),
    ));
    let (done, progress) = plugin.until_done(1);
    assert_eq!(done["status"]["kind"], "completed", "{done}");
    assert_eq!(done["finished"], json!([0]));
    assert!(!progress.is_empty(), "the first entry is always reported");
    assert!(
        progress
            .iter()
            .all(|p| p["job"] == 1 && p["done"].as_u64() >= Some(1))
    );

    plugin.send(&archive_event(2, extract_work(&tar, &dest, None)));
    let (done, _) = plugin.until_done(2);
    assert_eq!(done["status"]["kind"], "completed", "{done}");
    assert_eq!(
        std::fs::read_to_string(dest.join("src/a.txt")).unwrap(),
        "alpha"
    );

    assert!(plugin.finish().is_empty(), "nothing else was said");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_sealed_zip_asks_for_its_password_and_opens_with_it_and_the_password_is_never_echoed() {
    let dir = scratch("sealed");
    std::fs::write(dir.join("secret.txt"), "classified").unwrap();
    let (zip, dest) = (dir.join("sealed.zip"), dir.join("out"));

    let mut plugin = Plugin::start();
    plugin.send(&archive_event(
        1,
        compress_work(&dir.join("secret.txt"), &zip, "zip", Some("hunter2")),
    ));
    assert_eq!(plugin.until_done(1).0["status"]["kind"], "completed");

    plugin.send(&archive_event(2, extract_work(&zip, &dest, None)));
    let (asked, _) = plugin.until_done(2);
    assert_eq!(asked["status"]["kind"], "password_needed");
    assert_eq!(Path::new(asked["status"]["archive"].as_str().unwrap()), zip);
    assert!(!dest.exists());

    plugin.send(&archive_event(
        3,
        extract_work(&zip, &dest, Some("nope-not-it")),
    ));
    assert_eq!(plugin.until_done(3).0["status"]["kind"], "wrong_password");

    plugin.send(&archive_event(
        4,
        extract_work(&zip, &dest, Some("hunter2")),
    ));
    assert_eq!(plugin.until_done(4).0["status"]["kind"], "completed");
    assert_eq!(
        std::fs::read_to_string(dest.join("secret.txt")).unwrap(),
        "classified"
    );

    let seen = plugin.seen.join("\n");
    assert!(
        !seen.contains("hunter2") && !seen.contains("nope-not-it"),
        "{seen}"
    );
    plugin.finish();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_request_for_nothing_that_exists_fails_in_words_and_the_program_carries_on() {
    let dir = scratch("fail");
    let mut plugin = Plugin::start();
    plugin.send(&archive_event(
        7,
        compress_work(&dir.join("missing"), &dir.join("x.zip"), "zip", None),
    ));
    let (done, _) = plugin.until_done(7);
    assert_eq!(done["status"]["kind"], "failed");
    assert!(!done["status"]["message"].as_str().unwrap().is_empty());

    std::fs::write(dir.join("ok.txt"), "ok").unwrap();
    plugin.send(&archive_event(
        8,
        compress_work(&dir.join("ok.txt"), &dir.join("ok.zip"), "zip", None),
    ));
    assert_eq!(plugin.until_done(8).0["status"]["kind"], "completed");
    plugin.finish();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_request_in_flight_when_the_host_goes_away_is_stopped_and_the_program_still_ends() {
    let dir = scratch("gone");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    for n in 0..300 {
        std::fs::write(dir.join("src").join(format!("f{n}.txt")), vec![b'x'; 4096]).unwrap();
    }
    let mut plugin = Plugin::start();
    plugin.send(&archive_event(
        1,
        compress_work(&dir.join("src"), &dir.join("src.zip"), "zip", None),
    ));
    // The host closes its end without waiting. Whether the request had finished or was cut short
    // is a race; what must hold is that its result is written and the program ends.
    let rest = plugin.finish();
    let done = rest
        .iter()
        .find(|line| line["method"] == "archive_done")
        .expect("the result is written before the program ends");
    assert!(
        ["completed", "cancelled"].contains(&done["params"]["status"]["kind"].as_str().unwrap()),
        "{done}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_host_speaking_another_api_version_is_refused_with_a_log_line() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mman-archive"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut params = event("init", json!({}));
    params["params"]["api_version"] = json!(API_VERSION + 1);
    writeln!(stdin, "{params}").unwrap();
    let mut out = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut out)
        .unwrap();
    assert!(
        out.contains("\"log\"") && out.contains("plugin API"),
        "{out}"
    );
    drop(stdin);
    assert!(child.wait().unwrap().success());
}
