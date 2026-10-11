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

//! Drives the session from the lines on stdin. A reader thread turns each line into a
//! [`Command`]; the loop below owns the [`Session`] and the output, sleeping until a command
//! arrives or the next status run is due. Git itself runs on a thread of its own per run, so a
//! slow repository never stops the plugin from noticing the folder changed.

use std::io::{self, BufRead, Write};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use plugin_api::client::{Incoming, notification_line, parse_incoming, request_line};
use plugin_api::{
    API_VERSION, EVENT_DIR_CHANGED, EVENT_INIT, EVENT_SELECTION_CHANGED, EventParams,
    METHOD_SET_VIEW, StateSnapshot,
};
use serde_json::json;

use crate::session::{Command, Effect, Session};
use crate::status::read_repo;

/// The id of the one request this plugin sends.
const STATE_REQUEST: u64 = 1;

/// How long to sleep when nothing is waiting on the clock.
const IDLE: Duration = Duration::from_secs(3600);

/// Messages to the loop: a command, or the end of the host's input.
enum Message {
    Command(Command),
    Gone,
}

/// Reads the host's lines from `input` and answers on `output` until the host closes the input.
///
/// # Errors
///
/// Only a failure to write to `output`, which means the host is gone.
pub fn run(input: impl BufRead + Send + 'static, mut output: impl Write) -> io::Result<()> {
    let (messages, queue) = channel();
    let reader = messages.clone();
    thread::spawn(move || read_lines(input, &reader));

    let mut session = Session::new();
    loop {
        let now = Instant::now();
        let wait = session
            .deadline()
            .map_or(IDLE, |at| at.saturating_duration_since(now));
        let effects = match queue.recv_timeout(wait) {
            Ok(Message::Command(command)) => session.handle(command, Instant::now()),
            Ok(Message::Gone) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => session.tick(Instant::now()),
        };
        for effect in effects {
            match effect {
                Effect::AskState => {
                    output.write_all(
                        request_line(STATE_REQUEST, "get_state", &json!({})).as_bytes(),
                    )?;
                }
                Effect::Show(set) => {
                    output.write_all(notification_line(METHOD_SET_VIEW, &set).as_bytes())?;
                }
                Effect::Run { run, dir, cancel } => {
                    let messages = messages.clone();
                    thread::spawn(move || {
                        let answer = read_repo(&dir, &cancel);
                        let _ = messages.send(Message::Command(Command::Answered { run, answer }));
                    });
                }
                Effect::Fatal(message) => {
                    output.write_all(
                        notification_line("log", &json!({ "level": "error", "message": message }))
                            .as_bytes(),
                    )?;
                    output.flush()?;
                    return Ok(());
                }
            }
        }
        output.flush()?;
    }
}

fn read_lines(input: impl BufRead, messages: &Sender<Message>) {
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let command = match parse_incoming(&line) {
            Ok(Incoming::Event(params)) => command_of(*params),
            Ok(Incoming::Reply { id, error, result }) if id.as_u64() == Some(STATE_REQUEST) => {
                // A refused or unreadable reply leaves the folder from `init` and the events.
                error
                    .is_none()
                    .then_some(result)
                    .flatten()
                    .and_then(state_of)
            }
            // A line this plugin cannot read is not worth stopping for.
            Ok(Incoming::Reply { .. }) | Err(_) => None,
        };
        if let Some(command) = command
            && messages.send(Message::Command(command)).is_err()
        {
            return;
        }
    }
    let _ = messages.send(Message::Gone);
}

fn state_of(result: serde_json::Value) -> Option<Command> {
    let state: StateSnapshot = serde_json::from_value(result).ok()?;
    Some(Command::State {
        cwd: state.cwd,
        cursor: state.cursor,
    })
}

/// What an event asks of the plugin, or `None` for one it has no use for.
fn command_of(params: EventParams) -> Option<Command> {
    match params.name.as_str() {
        EVENT_INIT if params.api_version != API_VERSION => Some(Command::Incompatible {
            host: params.api_version,
        }),
        EVENT_INIT => Some(Command::Init { cwd: params.cwd }),
        EVENT_DIR_CHANGED => params.change?.to.map(Command::DirChanged),
        EVENT_SELECTION_CHANGED => Some(Command::CursorChanged(params.change?.to)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    /// A writer the test can read after the plugin has finished with it.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("not poisoned").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn lines_of(shared: &Shared) -> Vec<serde_json::Value> {
        let bytes = shared.0.lock().expect("not poisoned").clone();
        String::from_utf8(bytes)
            .expect("utf-8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("each line is JSON"))
            .collect()
    }

    fn event(name: &str, cwd: &str, extra: &str) -> String {
        format!(
            r#"{{"jsonrpc":"2.0","method":"event","params":{{"name":"{name}","api_version":{API_VERSION},"cwd":"{cwd}","selection":[]{extra}}}}}"#
        )
    }

    #[test]
    fn init_asks_for_the_state_and_an_incompatible_host_gets_a_logged_refusal() {
        let out = Shared::default();
        let input = format!("{}\n", event("init", "/plain-not-a-repo", ""));
        run(Cursor::new(input), out.clone()).expect("the host closing the input ends it");
        let lines = lines_of(&out);
        assert!(
            lines
                .iter()
                .any(|l| l["method"] == "get_state" && l["id"] == 1),
            "{lines:?}"
        );

        let out = Shared::default();
        let wrong = r#"{"jsonrpc":"2.0","method":"event","params":{"name":"init","api_version":999,"cwd":"/","selection":[]}}"#;
        run(Cursor::new(format!("{wrong}\n")), out.clone()).expect("a clean stop");
        let lines = lines_of(&out);
        assert_eq!(lines[0]["method"], "log");
        assert_eq!(lines[0]["params"]["level"], "error");
    }

    #[test]
    fn the_events_the_plugin_follows_become_commands_and_the_rest_are_ignored() {
        let params = |text: &str| -> EventParams {
            serde_json::from_str::<serde_json::Value>(text)
                .and_then(|v| serde_json::from_value(v["params"].clone()))
                .expect("a valid event")
        };
        let dir = params(&event("dir_changed", "/r", r#","from":"/a","to":"/r""#));
        assert!(matches!(command_of(dir), Some(Command::DirChanged(d)) if d.ends_with("r")));
        let empty = params(&event(
            "selection_changed",
            "/r",
            r#","from":"/r/a","to":null"#,
        ));
        assert!(matches!(
            command_of(empty),
            Some(Command::CursorChanged(None))
        ));
        assert!(command_of(params(&event("key", "/r", ""))).is_none());
    }
}
