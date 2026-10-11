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

//! The real `mman-archive` program answering the host's questions about the inside of an archive.
//! The test stands in for the host on both sides of the wire: it sends `vfs` requests, and answers
//! the `read_file` and `stat` requests the plugin makes in the middle of them from the real disk,
//! as the host does. This is where the process boundary, the handles and the plugin's own requests
//! are exercised together.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::{Duration, Instant};

use plugin_api::{
    API_VERSION, Bytes, ChunkKind, FileChunk, IoKind, MAX_VFS_READ, Opened, ReadFile, VfsFault,
    VfsRequest, WireEntry, WireKind, WireMeta, request_to_line,
};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(30);

struct Plugin {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
    next_id: u64,
    /// How many `read_file` requests the plugin made of the host.
    reads: usize,
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
            next_id: 100,
            reads: 0,
        };
        plugin.write(&json!({ "jsonrpc": "2.0", "method": "event", "params": {
                "name": "init", "api_version": API_VERSION, "cwd": "/", "selection": [] } }));
        plugin
    }

    fn write(&mut self, value: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        writeln!(stdin, "{value}").expect("the plugin is listening");
        stdin.flush().expect("flushed");
    }

    /// Answers one request the plugin made of the host, from the disk.
    fn answer_host_request(&mut self, line: &Value) {
        let id = line["id"].clone();
        let reply = match line["method"].as_str() {
            Some("read_file") => {
                self.reads += 1;
                let request: ReadFile = serde_json::from_value(line["params"].clone()).unwrap();
                read_file(&request)
            }
            other => Err(format!("this stand-in host does not answer {other:?}")),
        };
        self.write(&match reply {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(message) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "message": message } })
            }
        });
    }

    /// Sends `request` and returns the reply to it, answering what the plugin asks of the host on
    /// the way.
    fn ask(&mut self, request: &VfsRequest) -> Result<Value, VfsFault> {
        self.next_id += 1;
        let id = self.next_id;
        let line = request_to_line(id, request);
        self.stdin
            .as_mut()
            .expect("stdin is open")
            .write_all(line.as_bytes())
            .expect("the plugin is listening");
        loop {
            let line = match self.lines.recv_timeout(WAIT) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => panic!("no reply to {request:?} in {WAIT:?}"),
                Err(RecvTimeoutError::Disconnected) => panic!("the plugin ended mid-request"),
            };
            if line["method"].as_str().is_some() {
                if line["id"].is_null() {
                    continue; // a notification (a log line)
                }
                self.answer_host_request(&line);
                continue;
            }
            assert_eq!(line["id"], id, "a reply to another request: {line}");
            if let Some(error) = line.get("error") {
                return Err(serde_json::from_value(error["data"].clone())
                    .unwrap_or_else(|e| panic!("a fault with its data: {error} ({e})")));
            }
            return Ok(line["result"].clone());
        }
    }

    fn ask_as<T: serde::de::DeserializeOwned>(
        &mut self,
        request: &VfsRequest,
    ) -> Result<T, VfsFault> {
        self.ask(request)
            .map(|value| serde_json::from_value(value).expect("the reply has the shape asked"))
    }

    fn finish(mut self) {
        drop(self.stdin.take());
        let started = Instant::now();
        while started.elapsed() < WAIT {
            if let Some(status) = self.child.try_wait().expect("waitable") {
                assert!(status.success(), "the plugin ended badly: {status}");
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the plugin did not end when the host closed its input");
    }
}

/// The host's `read_file`: part of a file on the disk, as `FileChunk` carries it.
fn read_file(request: &ReadFile) -> Result<Value, String> {
    let meta = std::fs::metadata(&request.path).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    if request.len > 0 {
        let mut file = std::fs::File::open(&request.path).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(request.offset))
            .map_err(|e| e.to_string())?;
        file.take(request.len.min(plugin_api::MAX_READ_BYTES))
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
    }
    let seconds = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs());
    serde_json::to_value(FileChunk {
        kind: ChunkKind::File,
        size: meta.len(),
        modified: seconds,
        local_path: Some(request.path.clone()),
        data,
    })
    .map_err(|e| e.to_string())
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mman-serve-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn body(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 7) % 251) as u8).collect()
}

fn zip_with(path: &Path, files: &[(&str, Vec<u8>)]) {
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in files {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn tar_gz_with(path: &Path, files: &[(&str, Vec<u8>)]) {
    let encoder = flate2::write::GzEncoder::new(
        std::fs::File::create(path).unwrap(),
        flate2::Compression::fast(),
    );
    let mut builder = tar::Builder::new(encoder);
    for (name, bytes) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(1_700_000_000);
        builder
            .append_data(&mut header, name, bytes.as_slice())
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap();
}

fn inside(archive: &Path) -> PathBuf {
    PathBuf::from(format!("{}!", archive.display()))
}

fn read_all(plugin: &mut Plugin, path: &Path) -> Vec<u8> {
    let opened: Opened = plugin
        .ask_as(&VfsRequest::Open {
            path: path.to_path_buf(),
        })
        .expect("opens");
    let mut out = Vec::new();
    while (out.len() as u64) < opened.len {
        let bytes: Bytes = plugin
            .ask_as(&VfsRequest::Read {
                handle: opened.handle,
                offset: out.len() as u64,
                len: MAX_VFS_READ,
            })
            .expect("reads");
        assert!(!bytes.data.is_empty(), "no progress before the end");
        out.extend(bytes.data);
    }
    plugin
        .ask(&VfsRequest::Close {
            handle: opened.handle,
        })
        .expect("closes");
    out
}

#[test]
fn an_archive_on_the_disk_is_listed_described_and_read_through_the_wire() {
    let dir = scratch("zip");
    let data = body(300_000);
    zip_with(
        &dir.join("a.zip"),
        &[("d/f.bin", data.clone()), ("top.txt", b"top".to_vec())],
    );
    let root = inside(&dir.join("a.zip"));
    let mut plugin = Plugin::start();

    let listed: Vec<WireEntry> = plugin
        .ask_as(&VfsRequest::List { path: root.clone() })
        .unwrap();
    let names: Vec<&str> = listed.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["d", "top.txt"]);
    assert!(listed[0].is_dir);

    let meta: WireMeta = plugin
        .ask_as(&VfsRequest::Metadata {
            path: root.join("d/f.bin"),
            follow: true,
        })
        .unwrap();
    assert_eq!((meta.kind, meta.len), (WireKind::File, 300_000));

    assert_eq!(read_all(&mut plugin, &root.join("d/f.bin")), data);
    assert!(
        plugin.reads > 0,
        "the archive was read back through the host"
    );
    plugin.finish();
}

#[test]
fn a_tar_gz_is_read_through_the_wire() {
    let dir = scratch("targz");
    let data = body(70_000);
    tar_gz_with(
        &dir.join("b.tar.gz"),
        &[("x/one.bin", data.clone()), ("two.txt", b"two".to_vec())],
    );
    let root = inside(&dir.join("b.tar.gz"));
    let mut plugin = Plugin::start();
    assert_eq!(read_all(&mut plugin, &root.join("x/one.bin")), data);
    assert_eq!(read_all(&mut plugin, &root.join("two.txt")), b"two");
    plugin.finish();
}

#[test]
fn failures_come_back_as_faults_and_the_program_carries_on() {
    let dir = scratch("faults");
    zip_with(&dir.join("a.zip"), &[("f.txt", b"x".to_vec())]);
    std::fs::write(dir.join("broken.zip"), b"not an archive at all".repeat(30)).unwrap();
    let root = inside(&dir.join("a.zip"));
    let mut plugin = Plugin::start();

    assert_eq!(
        plugin.ask(&VfsRequest::List {
            path: root.join("nope")
        }),
        Err(VfsFault::NotFound)
    );
    assert_eq!(
        plugin.ask(&VfsRequest::List {
            path: root.join("f.txt")
        }),
        Err(VfsFault::NotADirectory)
    );
    assert_eq!(
        plugin.ask(&VfsRequest::Read {
            handle: 999,
            offset: 0,
            len: 1
        }),
        Err(VfsFault::BadHandle)
    );
    // The host's own refusal (no such file) reaches the caller as the reason.
    let missing = inside(&dir.join("gone.zip"));
    assert!(matches!(
        plugin.ask(&VfsRequest::List { path: missing }),
        Err(VfsFault::NotFound | VfsFault::Io { .. })
    ));
    assert!(matches!(
        plugin.ask(&VfsRequest::List {
            path: inside(&dir.join("broken.zip"))
        }),
        Err(VfsFault::Io {
            kind: IoKind::InvalidData | IoKind::Other,
            ..
        })
    ));
    // Still alive and right afterwards.
    let listed: Vec<WireEntry> = plugin.ask_as(&VfsRequest::List { path: root }).unwrap();
    assert_eq!(listed.len(), 1);
    plugin.finish();
}

#[test]
fn a_request_the_plugin_does_not_know_is_refused_not_left_to_time_out() {
    let mut plugin = Plugin::start();
    plugin.next_id += 1;
    let id = plugin.next_id;
    plugin.write(&json!({
        "jsonrpc": "2.0", "id": id, "method": "vfs", "params": { "op": "from_the_future" }
    }));
    let reply = plugin.lines.recv_timeout(WAIT).expect("an answer");
    assert_eq!(reply["id"], id);
    assert_eq!(reply["error"]["data"]["fault"], "unsupported");
    plugin.finish();
}

#[test]
fn an_archive_that_changed_on_disk_is_listed_again() {
    let dir = scratch("changed");
    let archive = dir.join("a.zip");
    zip_with(&archive, &[("one", b"1".to_vec())]);
    let root = inside(&archive);
    let mut plugin = Plugin::start();
    let names = |plugin: &mut Plugin| -> Vec<String> {
        plugin
            .ask_as::<Vec<WireEntry>>(&VfsRequest::List { path: root.clone() })
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect()
    };
    assert_eq!(names(&mut plugin), ["one"]);
    zip_with(&archive, &[("one", b"1".to_vec()), ("two", b"22".to_vec())]);
    assert_eq!(names(&mut plugin), ["one", "two"]);
    plugin.finish();
}

/// What a request across the process boundary costs. Run with
/// `cargo test -p archiver --release --test serve_process -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark"]
fn what_the_boundary_costs() {
    let dir = scratch("bench");
    let data = body(32 << 20);
    zip_with(&dir.join("a.zip"), &[("big.bin", data.clone())]);
    let root = inside(&dir.join("a.zip"));
    let mut plugin = Plugin::start();

    // The first listing builds the table of contents.
    let started = Instant::now();
    plugin
        .ask(&VfsRequest::List { path: root.clone() })
        .unwrap();
    let cold = started.elapsed();

    let started = Instant::now();
    let rounds = 2_000;
    for _ in 0..rounds {
        plugin
            .ask(&VfsRequest::Metadata {
                path: root.join("big.bin"),
                follow: true,
            })
            .unwrap();
    }
    let per_stat = started.elapsed() / rounds;

    let started = Instant::now();
    let read = read_all(&mut plugin, &root.join("big.bin"));
    let whole = started.elapsed();
    assert_eq!(read, data);
    eprintln!(
        "first listing {cold:?}; a stat {per_stat:?} (this includes the plugin asking the host \
         for the archive's length each time); open, unpack and read 32 MiB {whole:?} \
         ({:.0} MiB/s)",
        32.0 / whole.as_secs_f64()
    );
    plugin.finish();
}
