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
//! the scanner and any action under way have stopped and written their last lines.

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::thread;

use plugin_api::client::{Incoming, notification_line, parse_incoming};
use plugin_api::{API_VERSION, EVENT_INIT};
use serde_json::json;

use crate::scanner::RealMachine;
use crate::session::Session;

enum Message {
    /// A line the host wrote.
    Host(String),
    /// A line the scanner or an action wants written.
    Out(String),
    /// The host closed its end.
    Gone,
}

/// Reads the host's lines from `input` and writes the plugin's to `output` until the host closes
/// the input and everything the plugin started has ended.
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

    // The scanner and the actions write through their own sender; this thread forwards them so
    // there is one place that owns `output`.
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

    let mut session = Some(Session::new(out_tx, Arc::new(RealMachine::new())));
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
                let Some(session) = session.as_ref() else {
                    continue;
                };
                if !handle(session, &line, &mut output)? {
                    return Ok(());
                }
            }
            // Dropping the session drops the scanner's inbox, which ends it; the loop then ends
            // once the last action has written its note and dropped its sender.
            Message::Gone => drop(session.take()),
        }
    }
    Ok(())
}

/// Acts on one line from the host. `false` means the plugin should stop (a host it cannot speak
/// to).
fn handle(session: &Session, line: &str, output: &mut impl Write) -> io::Result<bool> {
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
        // This plugin sends no requests, so there is no reply to read; a line it cannot read is
        // not worth stopping for.
        Ok(Incoming::Reply { .. }) | Err(_) => {}
    }
    Ok(true)
}
