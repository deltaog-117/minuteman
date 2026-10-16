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

//! What the tests in this folder share: a stand-in for the host that runs the real `mman-ssh`
//! program and speaks the plugin protocol to it over pipes, and the `sshd` and scratch folder of
//! `sshd.rs`.

#![allow(dead_code)]

mod sshd;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, Once, PoisonError};
use std::thread;
use std::time::Duration;

use plugin_api::{
    API_VERSION, Answer, Epoch, Peer, PeerError, Reply, VfsFault, VfsRequest, request_to_line,
};
use serde_json::{Value, json};
use shared::LocalVfs;
use vfs_remote::SshVfs;

/// How long a request may wait for the program, so a hung test fails instead of hanging.
const WAIT: Duration = Duration::from_secs(120);

/// Where `mman-ssh` is: next to the folder the test binary runs from. Built first when missing,
/// since nothing in this crate's dependencies would have built it.
pub fn plugin_binary() -> PathBuf {
    static BUILD: Once = Once::new();
    let exe = std::env::current_exe().expect("the test binary's path");
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>/deps/<test>")
        .to_path_buf();
    let binary = profile_dir.join("mman-ssh");
    BUILD.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let mut build = Command::new(cargo);
        build.args(["build", "-q", "-p", "remoter"]);
        if profile_dir
            .file_name()
            .is_some_and(|name| name == "release")
        {
            build.arg("--release");
        }
        let built = build
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("cargo runs");
        assert!(built.success(), "building mman-ssh failed");
    });
    binary
}

/// The callers waiting for a reply, by the id of their request.
type Waiting = Arc<Mutex<HashMap<u64, Sender<Result<Value, PeerError>>>>>;

/// The host's end of the plugin protocol for one running `mman-ssh`: it sends each request as a
/// line and hands the reply that carries the same id back to the caller waiting for it.
pub struct ProcessPeer {
    child: Mutex<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: Mutex<u64>,
    waiting: Waiting,
}

impl ProcessPeer {
    /// A host whose user declines every question the plugin asks.
    pub fn start(env: &[(&str, String)]) -> Arc<Self> {
        Self::start_answering(env, false)
    }

    /// A host whose user answers every yes or no question with `trust`, and declines the rest.
    pub fn start_answering(env: &[(&str, String)], trust: bool) -> Arc<Self> {
        let mut command = Command::new(plugin_binary());
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (name, value) in env {
            command.env(name, value);
        }
        let mut child = command.spawn().expect("the plugin program starts");
        let stdout = child.stdout.take().expect("piped");
        let mut stdin = child.stdin.take().expect("piped");
        writeln!(
            stdin,
            "{}",
            json!({ "jsonrpc": "2.0", "method": "event", "params": {
                "name": "init", "api_version": API_VERSION, "cwd": "/", "selection": [] } })
        )
        .expect("the plugin is listening");

        let stdin = Arc::new(Mutex::new(stdin));
        let waiting: Waiting = Arc::default();
        let readers = Arc::clone(&waiting);
        let replies = Arc::clone(&stdin);
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                // A question the plugin puts to the user, answered as the user would.
                if value["method"] == "ask" {
                    let reply = match (value["params"]["kind"].as_str(), trust) {
                        (Some("confirm"), true) => json!({ "jsonrpc": "2.0", "id": value["id"],
                            "result": { "yes": true } }),
                        _ => json!({ "jsonrpc": "2.0", "id": value["id"],
                            "error": { "code": -32000, "message": "declined" } }),
                    };
                    let mut stdin = replies.lock().unwrap_or_else(PoisonError::into_inner);
                    let _ = writeln!(stdin, "{reply}").and_then(|()| stdin.flush());
                    continue;
                }
                let Some(id) = value["id"].as_u64() else {
                    continue;
                };
                let outcome = if value.get("error").is_some() {
                    Err(PeerError::Fault(
                        serde_json::from_value::<VfsFault>(value["error"]["data"].clone())
                            .unwrap_or_else(|_| VfsFault::Io {
                                kind: plugin_api::IoKind::Other,
                                message: value["error"]["message"].to_string(),
                            }),
                    ))
                } else {
                    Ok(value["result"].clone())
                };
                let sender = readers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id);
                if let Some(sender) = sender {
                    let _ = sender.send(outcome);
                }
            }
            // The program ended: everything still waiting is told so.
            let stranded: Vec<_> = readers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .drain()
                .collect();
            for (_, sender) in stranded {
                let _ = sender.send(Err(PeerError::Stopped));
            }
        });
        Arc::new(Self {
            child: Mutex::new(child),
            stdin,
            next_id: Mutex::new(100),
            waiting,
        })
    }
}

impl Peer for ProcessPeer {
    fn call(&self, request: &VfsRequest, after: Option<Epoch>) -> Result<Answer, PeerError> {
        self.submit(request, after)?.wait()
    }

    fn submit(
        &self,
        request: &VfsRequest,
        _after: Option<Epoch>,
    ) -> Result<Box<dyn Reply>, PeerError> {
        let id = {
            let mut next = self.next_id.lock().unwrap_or_else(PoisonError::into_inner);
            *next += 1;
            *next
        };
        let (tx, rx) = channel();
        self.waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, tx);
        let line = request_to_line(id, request);
        {
            let mut stdin = self.stdin.lock().unwrap_or_else(PoisonError::into_inner);
            stdin
                .write_all(line.as_bytes())
                .and_then(|()| stdin.flush())
                .map_err(|_| PeerError::Stopped)?;
        }
        Ok(Box::new(Awaited(rx)))
    }
}

/// A request written to the process whose reply has not come yet.
struct Awaited(Receiver<Result<Value, PeerError>>);

impl Reply for Awaited {
    fn wait(self: Box<Self>) -> Result<Answer, PeerError> {
        match self.0.recv_timeout(WAIT) {
            Ok(Ok(value)) => Ok(Answer {
                epoch: Epoch(1),
                value,
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(PeerError::TimedOut),
        }
    }
}

impl Drop for ProcessPeer {
    fn drop(&mut self) {
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub use sshd::{Sshd, scratch};

impl Sshd {
    fn vfs_with(&self, accept_new: bool) -> SshVfs {
        SshVfs::new(
            Arc::new(LocalVfs),
            ProcessPeer::start(&self.env(accept_new)),
        )
    }

    /// A file system that remembers this server's key the first time it sees it.
    pub fn vfs(&self) -> SshVfs {
        self.vfs_with(true)
    }

    /// One that refuses a host whose key it does not already know, and whose user declines when
    /// asked whether to trust it.
    pub fn vfs_strict(&self) -> SshVfs {
        self.vfs_with(false)
    }

    /// One whose user says yes when asked whether to trust a host's key.
    pub fn vfs_trusting(&self) -> SshVfs {
        SshVfs::new(
            Arc::new(LocalVfs),
            ProcessPeer::start_answering(&self.env(false), true),
        )
    }
}
