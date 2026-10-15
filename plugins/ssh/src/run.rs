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

//! Moves lines between the host and the [`Server`]: reads the host's lines from `input`, answers
//! each `vfs` request on a thread of its own and writes the reply to `output`, and ends when the
//! host closes its end, once the requests still running have written their replies.
//!
//! A request gets its own thread because a remote call can take seconds (a connection being made,
//! a folder with many entries) and must not hold up the next line, which may be a read on a file
//! that is already open. That thread runs inside the request's `shared::cancel` token, which a
//! `vfs_cancel` line naming the request sets, so whatever it waits for gives up with it.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use plugin_api::client::{
    Incoming, VfsAskError, notification_line, parse_incoming, parse_vfs_cancel, parse_vfs_request,
};
use plugin_api::{API_VERSION, EVENT_INIT, VfsFault, VfsRequest, fault_line, reply_line};
use serde_json::{Value, json};
use shared::Cancel;

use crate::serve::Server;

/// The requests that have been started and have not yet written their reply.
#[derive(Default)]
struct InFlight {
    count: Mutex<usize>,
    ended: Condvar,
}

impl InFlight {
    fn begin(&self) {
        *self.count.lock().unwrap_or_else(PoisonError::into_inner) += 1;
    }

    fn end(&self) {
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        *count -= 1;
        self.ended.notify_all();
    }

    fn wait_for_none(&self) {
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        while *count > 0 {
            count = self
                .ended
                .wait(count)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// The token of each request being answered, by its id as the host wrote it, so a `vfs_cancel`
/// reaches the one it names.
#[derive(Default)]
struct Tokens(Mutex<HashMap<String, Cancel>>);

impl Tokens {
    fn map(&self) -> MutexGuard<'_, HashMap<String, Cancel>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn start(&self, id: &Value) -> Cancel {
        let token = Cancel::new();
        self.map().insert(id.to_string(), token.clone());
        token
    }

    fn end(&self, id: &Value) {
        self.map().remove(&id.to_string());
    }

    /// Gives up on request `id`. One already answered, or never seen, is nothing to give up.
    fn cancel(&self, id: &Value) {
        if let Some(token) = self.map().get(&id.to_string()) {
            token.cancel();
        }
    }
}

/// The one place that owns `output`, so two replies never interleave on a line.
struct Out<W>(Mutex<W>);

impl<W: Write> Out<W> {
    /// Writes one line. A failure means the host is gone, and the loop reading its lines ends on
    /// its own, so there is no one to tell.
    fn send(&self, line: &str) {
        let mut out = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = out.write_all(line.as_bytes()).and_then(|()| out.flush());
    }
}

fn serve<W: Write + Send + 'static>(
    server: &Arc<Server>,
    out: &Arc<Out<W>>,
    in_flight: &Arc<InFlight>,
    tokens: &Arc<Tokens>,
    id: Value,
    request: VfsRequest,
) {
    let (server, out_thread, ended) = (Arc::clone(server), Arc::clone(out), Arc::clone(in_flight));
    let token = tokens.start(&id);
    let (tokens_thread, id_thread) = (Arc::clone(tokens), id.clone());
    in_flight.begin();
    let spawned = thread::Builder::new()
        .name("ssh-vfs".to_string())
        .spawn(move || {
            let answer = {
                let _scope = token.enter();
                server.answer(&request)
            };
            tokens_thread.end(&id);
            let line = match answer {
                Ok(value) => reply_line(&id, &value),
                Err(fault) => fault_line(&id, &fault),
            };
            out_thread.send(&line);
            ended.end();
        });
    if spawned.is_err() {
        // Out of threads: the host's wait ends by its own timeout rather than hanging here.
        tokens.end(&id_thread);
        in_flight.end();
        out.send(&notification_line(
            "log",
            &json!({ "level": "error", "message": "could not start a thread to answer" }),
        ));
    }
}

/// Reads the host's lines from `input` and writes the plugin's to `output` until the host closes
/// the input and every request has been answered.
///
/// # Errors
///
/// Only a failure to read `input`.
pub fn run<W: Write + Send + 'static>(
    input: impl BufRead,
    output: W,
    server: Server,
) -> io::Result<()> {
    let server = Arc::new(server);
    let out = Arc::new(Out(Mutex::new(output)));
    let in_flight = Arc::new(InFlight::default());
    let tokens = Arc::new(Tokens::default());
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(asked) = parse_vfs_request(&line) {
            match asked {
                Ok((id, request)) => serve(&server, &out, &in_flight, &tokens, id, request),
                // A request this plugin does not know (a newer host) is refused, not left to
                // time out.
                Err(VfsAskError {
                    id: Some(id),
                    message,
                }) => out.send(&fault_line(&id, &VfsFault::Unsupported { reason: message })),
                Err(_) => {}
            }
            continue;
        }
        if let Some(id) = parse_vfs_cancel(&line) {
            tokens.cancel(&id);
            continue;
        }
        // Anything else the host writes is an event; only the greeting matters here.
        if let Ok(Incoming::Event(params)) = parse_incoming(&line)
            && params.name == EVENT_INIT
            && params.api_version != API_VERSION
        {
            let message = format!(
                "the host speaks plugin API {} and this was built for {API_VERSION}",
                params.api_version
            );
            out.send(&notification_line(
                "log",
                &json!({ "level": "error", "message": message }),
            ));
            break;
        }
    }
    in_flight.wait_for_none();
    Ok(())
}
