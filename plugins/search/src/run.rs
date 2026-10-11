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

//! Drives the search from the lines on stdin. A reader thread turns each line into a `Command`;
//! the loop below owns the `Session` and the output. While there is ranking to do it does one
//! slice at a time and only peeks at the queue, so a letter typed while a large index is still
//! being scored takes over within a few milliseconds; with nothing to do it sleeps until a command
//! arrives or an answer is due. Answers go out at most every `INTERVAL`, the first after a query
//! change at once, so a walk delivering thousands of folders a second does not make the host
//! rebuild its list for each one.

use std::io::{self, BufRead, ErrorKind, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::thread;
use std::time::{Duration, Instant};

use plugin_api::client::{Incoming, parse_incoming};
use plugin_api::{
    API_VERSION, EVENT_ACTION, EVENT_INIT, EVENT_PICKER_CLOSED, EVENT_PICKER_QUERY,
    EVENT_WALK_BATCH, EVENT_WALK_DONE, EventParams,
};

use crate::session::{ACTION, Command, Session, log_line};

/// How many entries are scored between looks at the queue: a few milliseconds of work, so that is
/// also the longest a new query waits to be noticed.
const SLICE: usize = 2048;

/// The least time between two answers to the host.
const INTERVAL: Duration = Duration::from_millis(40);

/// Why the loop did not get a command this turn.
enum Wait {
    /// Nothing arrived, but the reader is still there.
    Idle,
    /// The host closed our input: the program is ending.
    Gone,
}

impl From<TryRecvError> for Wait {
    fn from(e: TryRecvError) -> Self {
        match e {
            TryRecvError::Empty => Wait::Idle,
            TryRecvError::Disconnected => Wait::Gone,
        }
    }
}

impl From<RecvTimeoutError> for Wait {
    fn from(e: RecvTimeoutError) -> Self {
        match e {
            RecvTimeoutError::Timeout => Wait::Idle,
            RecvTimeoutError::Disconnected => Wait::Gone,
        }
    }
}

/// Reads the host's lines from `input` and answers on `output` until the host closes the input.
pub fn run(input: impl BufRead + Send + 'static, mut output: impl Write) -> io::Result<()> {
    let (commands, queue) = channel();
    thread::spawn(move || read_lines(input, &commands));
    drive(&queue, &mut output)
}

fn read_lines(input: impl BufRead, commands: &Sender<Command>) {
    for line in input.lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let command = match parse_incoming(&line) {
            Ok(Incoming::Event(params)) => command_of(*params),
            Ok(Incoming::Reply { id, error, .. }) => {
                id.as_u64().map(|id| Command::Replied { id, error })
            }
            // A line this plugin cannot read is not worth stopping for.
            Err(_) => None,
        };
        if let Some(command) = command
            && commands.send(command).is_err()
        {
            return;
        }
    }
}

/// What an event asks of the search, or `None` for one it has no use for.
fn command_of(params: EventParams) -> Option<Command> {
    match params.name.as_str() {
        EVENT_INIT if params.api_version != API_VERSION => Some(Command::Incompatible {
            host: params.api_version,
        }),
        EVENT_ACTION if params.action.as_deref() == Some(ACTION) => Some(Command::Open {
            root: params.cwd,
            show_hidden: params.show_hidden.unwrap_or(false),
        }),
        EVENT_PICKER_QUERY => {
            let picker = params.picker?;
            Some(Command::Query {
                picker: picker.picker,
                generation: picker.generation.unwrap_or(0),
                text: picker.text.unwrap_or_default(),
            })
        }
        EVENT_PICKER_CLOSED => Some(Command::Closed {
            picker: params.picker?.picker,
        }),
        EVENT_WALK_BATCH => {
            let walk = params.walk?;
            Some(Command::Batch {
                walk: walk.walk,
                entries: walk.entries,
            })
        }
        EVENT_WALK_DONE => {
            let walk = params.walk?;
            Some(Command::Walked {
                walk: walk.walk,
                end: walk.end?,
            })
        }
        _ => None,
    }
}

fn write_all(output: &mut impl Write, lines: &[String]) -> io::Result<()> {
    for line in lines {
        output.write_all(line.as_bytes())?;
    }
    output.flush()
}

/// A closed pipe means the host is gone, which is how this program is meant to end.
fn finished(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(e) if e.kind() == ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

fn drive(queue: &Receiver<Command>, output: &mut impl Write) -> io::Result<()> {
    let mut session = Session::default();
    let mut settled = true;
    // Something changed since the last answer, so one is owed.
    let mut owed = false;
    let mut next_send = Instant::now();
    loop {
        let received = if !settled {
            queue.try_recv().map_err(Wait::from)
        } else if owed {
            queue
                .recv_timeout(next_send.saturating_duration_since(Instant::now()))
                .map_err(Wait::from)
        } else {
            queue.recv().map_err(|_| Wait::Gone)
        };
        match received {
            Ok(first) => {
                let mut lines = Vec::new();
                let mut retyped = false;
                for command in
                    std::iter::once(first).chain(std::iter::from_fn(|| queue.try_recv().ok()))
                {
                    if let Command::Incompatible { host } = command {
                        let message = format!(
                            "the host speaks plugin protocol {host}, this search was built for {API_VERSION}"
                        );
                        let _ = write_all(output, &[log_line(&message)]);
                        return Err(io::Error::other(message));
                    }
                    retyped |= session.apply(command, &mut lines);
                }
                finished(write_all(output, &lines))?;
                settled = false;
                owed = true;
                if retyped {
                    next_send = Instant::now();
                }
            }
            Err(Wait::Idle) => {}
            Err(Wait::Gone) => return Ok(()),
        }
        if !settled {
            settled = session.advance(SLICE);
        }
        if owed && Instant::now() >= next_send {
            if let Some(line) = session.update(settled) {
                finished(write_all(output, &[line]))?;
            }
            next_send = Instant::now() + INTERVAL;
            owed = !settled;
        }
    }
}
