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

//! Moves lines between the host and the [`Session`]: reads the host's lines from `input`, writes
//! what the session's threads produce to `output`, and ends when the host closes its end, after
//! stopping the requests still running and writing their results.

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::Duration;

use plugin_api::client::{
    Incoming, VfsAskError, notification_line, parse_incoming, parse_vfs_request,
};
use plugin_api::remote::{Remote, RemoteVfs, Rpc};
use plugin_api::{API_VERSION, EVENT_INIT, VfsFault, VfsRequest, fault_line, reply_line};
use serde_json::{Value, json};

use crate::serve::Server;
use crate::session::Session;

/// How long a request waits for the host to read part of an archive file. The host answers from a
/// thread of its own, so this only matters when the disk or the network has stalled.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// The host as `RemoteVfs` reaches it: each request is a line through the same channel the
/// session's lines take, and its reply comes back to `rpc` from the loop that reads the host.
struct HostRemote {
    rpc: Arc<Rpc>,
    out: Sender<String>,
}

impl Remote for HostRemote {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.rpc.call(
            &|line| self.out.send(line).is_ok(),
            method,
            &params,
            REPLY_TIMEOUT,
        )
    }
}

/// What a line from the host can be about besides an archive request.
struct Serving {
    server: Arc<Server>,
    rpc: Arc<Rpc>,
    out: Sender<String>,
}

impl Serving {
    /// Answers the host's question on a thread of its own, so a slow archive never holds up the
    /// next line (including the reply this very question may be waiting for).
    fn serve(&self, id: Value, request: VfsRequest) {
        let (server, out) = (Arc::clone(&self.server), self.out.clone());
        let spawned = thread::Builder::new()
            .name("archive-vfs".to_string())
            .spawn(move || {
                let line = match server.answer(&request) {
                    Ok(value) => reply_line(&id, &value),
                    Err(fault) => fault_line(&id, &fault),
                };
                let _ = out.send(line);
            });
        if spawned.is_err() {
            // Out of threads: the host's wait ends by its own timeout rather than hanging here.
            let _ = self.out.send(notification_line(
                "log",
                &json!({ "level": "error", "message": "could not start a thread to answer" }),
            ));
        }
    }

    /// Hands the host's reply to the request that waits for it.
    fn reply(&self, id: &Value, error: Option<String>, result: Option<Value>) {
        let Some(id) = id.as_u64() else { return };
        self.rpc.answer(
            id,
            error.map_or_else(|| Ok(result.unwrap_or(Value::Null)), Err),
        );
    }
}

enum Message {
    /// A line the host wrote.
    Host(String),
    /// A line a request wants written.
    Out(String),
    /// The host closed its end.
    Gone,
}

/// Reads the host's lines from `input` and writes the plugin's to `output` until the host closes
/// the input and every request has ended.
///
/// # Errors
///
/// Only a failure to write to `output`, which means the host is gone.
pub fn run(input: impl BufRead + Send + 'static, mut output: impl Write) -> io::Result<()> {
    let (tx, rx) = channel::<Message>();
    let reader = tx.clone();
    thread::spawn(move || {
        for line in input.lines() {
            let Ok(line) = line else { break };
            if reader.send(Message::Host(line)).is_err() {
                return;
            }
        }
        let _ = reader.send(Message::Gone);
    });

    // The session's requests write through their own sender; this thread forwards them so there is
    // one place that owns `output`.
    let (out_tx, out_rx) = channel::<String>();
    let forward = tx.clone();
    thread::spawn(move || {
        for line in out_rx {
            if forward.send(Message::Out(line)).is_err() {
                return;
            }
        }
    });
    drop(tx);

    let rpc = Arc::new(Rpc::default());
    let remote: Arc<dyn Remote> = Arc::new(HostRemote {
        rpc: Arc::clone(&rpc),
        out: out_tx.clone(),
    });
    let mut serving = Some(Serving {
        server: Arc::new(Server::new(Arc::new(RemoteVfs::new(remote)))),
        rpc,
        out: out_tx.clone(),
    });
    let mut session = Some(Session::new(out_tx));
    for message in rx {
        match message {
            Message::Out(line) => {
                output.write_all(line.as_bytes())?;
                output.flush()?;
            }
            Message::Host(line) => {
                if line.trim().is_empty() {
                    continue;
                }
                let (Some(session), Some(serving)) = (session.as_ref(), serving.as_ref()) else {
                    continue;
                };
                if !handle(session, serving, &line, &mut output)? {
                    return Ok(());
                }
            }
            Message::Gone => {
                // Requests waiting on the host's replies would otherwise wait out their timeout;
                // letting go of `serving` lets the loop end once the last of them has written.
                if let Some(serving) = serving.take() {
                    serving.rpc.fail_all();
                }
                if let Some(session) = session.take() {
                    session.cancel_all();
                    // Dropping the session drops its sender; the loop then ends once the last
                    // request thread has written its result and dropped its own.
                }
            }
        }
    }
    Ok(())
}

/// Acts on one line from the host. `false` means the plugin should stop (a host it cannot speak to).
fn handle(
    session: &Session,
    serving: &Serving,
    line: &str,
    output: &mut impl Write,
) -> io::Result<bool> {
    if let Some(asked) = parse_vfs_request(line) {
        match asked {
            Ok((id, request)) => serving.serve(id, request),
            // A request this plugin does not know (a newer host) is refused, not left to time out.
            Err(VfsAskError {
                id: Some(id),
                message,
            }) => {
                let fault = VfsFault::Unsupported { reason: message };
                let _ = serving.out.send(fault_line(&id, &fault));
            }
            Err(_) => {}
        }
        return Ok(true);
    }
    match parse_incoming(line) {
        Ok(Incoming::Event(params)) if params.name == EVENT_INIT => {
            if params.api_version != API_VERSION {
                let message = format!(
                    "the host speaks plugin API {} and this was built for {API_VERSION}",
                    params.api_version
                );
                output.write_all(
                    notification_line("log", &json!({ "level": "error", "message": message }))
                        .as_bytes(),
                )?;
                output.flush()?;
                return Ok(false);
            }
        }
        Ok(Incoming::Event(params)) => session.on_event(&params),
        Ok(Incoming::Reply { id, error, result }) => serving.reply(&id, error, result),
        // A line it cannot read is not worth stopping for.
        Err(_) => {}
    }
    Ok(true)
}
