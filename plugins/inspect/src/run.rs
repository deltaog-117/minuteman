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
//! the answers to `output`, and ends when the host closes its end.

use std::io::{self, BufRead, Write};

use plugin_api::client::{Incoming, notification_line, parse_incoming};
use plugin_api::{API_VERSION, EVENT_INIT};
use serde_json::json;

use crate::session::Session;

/// Reads the host's lines from `input` and answers on `output` until the host closes the input.
///
/// # Errors
///
/// Only a failure to write to `output`, which means the host is gone.
pub fn run(input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut session = Session::new();
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let out = match parse_incoming(&line) {
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
                    return Ok(());
                }
                Vec::new()
            }
            Ok(Incoming::Event(params)) => session.on_event(&params),
            Ok(Incoming::Reply { id, error, result }) => match id.as_u64() {
                Some(id) => session.on_reply(id, error, result),
                None => Vec::new(),
            },
            // A line this plugin cannot read is not worth stopping for.
            Err(_) => Vec::new(),
        };
        for line in out {
            output.write_all(line.as_bytes())?;
        }
        output.flush()?;
    }
    for line in session.on_gone() {
        output.write_all(line.as_bytes())?;
    }
    output.flush()
}
