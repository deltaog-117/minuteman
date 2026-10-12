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

//! The plugin's conversation with the host: lines in, lines out, one thread that owns the state.
//!
//! A reader thread hands the host's lines to the loop below. While the view shows, the loop wakes
//! every `TICK` to pick up what the scans have found and draws again if the picture changed;
//! while it is closed it sleeps until the host writes, so a closed atlas costs nothing.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::thread;
use std::time::{Duration, Instant};

use plugin_api::client::{Incoming, notification_line, parse_incoming};
use plugin_api::{
    API_VERSION, Button, Canvas, CanvasEvent, CanvasInput, CanvasNote, EVENT_CANVAS, EVENT_INIT,
    METHOD_CANVAS_NOTE, METHOD_SET_CANVAS, SetCanvas, Surface,
};
use serde_json::json;
use shared::LocalVfs;

use crate::canvas::{build, places};
use crate::state::{AtlasState, Machine, Outcome, RealMachine};

/// How often an open view looks at what the scans have found.
const TICK: Duration = Duration::from_millis(100);

enum Message {
    /// A line the host wrote.
    Host(String),
    /// The host closed its end.
    Gone,
}

/// An atlas that is showing.
struct Open {
    state: AtlasState,
    width: u16,
    height: u16,
    /// The folder the browser was in when the view opened.
    here: PathBuf,
    /// The last canvas sent, so an unchanged picture is not sent again.
    sent: Option<Canvas>,
}

/// Runs the plugin on this machine until the host closes its input.
pub fn run(input: impl BufRead + Send + 'static, output: impl Write) -> io::Result<()> {
    run_on(input, output, Arc::new(RealMachine::new()))
}

/// `run` over a machine of the caller's choosing, so a conversation can be played in a test.
pub fn run_on(
    input: impl BufRead + Send + 'static,
    mut output: impl Write,
    machine: Arc<dyn Machine>,
) -> io::Result<()> {
    let (tx, rx) = channel::<Message>();
    thread::spawn(move || {
        for line in input.lines() {
            let Ok(line) = line else { break };
            if tx.send(Message::Host(line)).is_err() {
                return;
            }
        }
        let _ = tx.send(Message::Gone);
    });

    let mut open: Option<Open> = None;
    loop {
        let message = if open.is_some() {
            rx.recv_timeout(TICK)
        } else {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match message {
            Ok(Message::Host(line)) => {
                if line.trim().is_empty() {
                    continue;
                }
                if !handle(&line, &mut open, &machine, &mut output)? {
                    return Ok(());
                }
            }
            Ok(Message::Gone) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {}
        }
        if let Some(view) = open.as_mut() {
            view.state.poll(Instant::now(), &view.here);
            draw(view, &mut output)?;
        }
    }
}

/// Acts on one line from the host. `false` ends the plugin.
fn handle(
    line: &str,
    open: &mut Option<Open>,
    machine: &Arc<dyn Machine>,
    output: &mut impl Write,
) -> io::Result<bool> {
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
        Ok(Incoming::Event(params)) if params.name == EVENT_CANVAS => {
            if let Some(CanvasEvent {
                surface: Surface::Atlas,
                input,
            }) = params.canvas
            {
                apply(input, open, machine, output)?;
            }
        }
        // This plugin sends no requests, so there is no reply to read; a line it cannot read is
        // not worth stopping for.
        Ok(Incoming::Event(_) | Incoming::Reply { .. }) | Err(_) => {}
    }
    Ok(true)
}

fn apply(
    input: CanvasInput,
    open: &mut Option<Open>,
    machine: &Arc<dyn Machine>,
    output: &mut impl Write,
) -> io::Result<()> {
    match input {
        CanvasInput::Open {
            width,
            height,
            here,
        } => {
            // A fresh state every time: the view never kept anything between showings, and the
            // old one's scans are cancelled as it is dropped.
            *open = Some(Open {
                state: AtlasState::new(Arc::new(LocalVfs), Arc::clone(machine)),
                width,
                height,
                here,
                sent: None,
            });
        }
        CanvasInput::Close => *open = None,
        CanvasInput::Resize { width, height } => {
            if let Some(view) = open.as_mut() {
                view.width = width;
                view.height = height;
            }
        }
        CanvasInput::Key { key, mods } => {
            if let Some(view) = open.as_mut() {
                let map = places(view.width, view.height).map;
                let outcome = view.state.key(&key, mods, map);
                note(outcome, output)?;
            }
        }
        CanvasInput::Click {
            x,
            y,
            button: Button::Left,
            ..
        } => {
            if let Some(view) = open.as_mut() {
                let map = places(view.width, view.height).map;
                let outcome = view.state.click(map, x, y);
                note(outcome, output)?;
            }
        }
        // The map has no menu, the wheel does nothing on it, and it marks the folder the browser
        // was in when it opened and carries nothing.
        CanvasInput::Click { .. } | CanvasInput::Wheel { .. } | CanvasInput::Context { .. } => {}
    }
    Ok(())
}

/// Tells the host what the atlas asked of the browser.
fn note(outcome: Outcome, output: &mut impl Write) -> io::Result<()> {
    let note = match outcome {
        Outcome::Pass | Outcome::Handled => return Ok(()),
        Outcome::Goto(path) => CanvasNote::Goto {
            path,
            text: String::new(),
        },
        Outcome::Status(text) => CanvasNote::Status { text },
    };
    output.write_all(notification_line(METHOD_CANVAS_NOTE, &note).as_bytes())?;
    output.flush()
}

/// Sends the picture if it differs from the last one sent.
fn draw(view: &mut Open, output: &mut impl Write) -> io::Result<()> {
    let canvas = build(&view.state, &view.here, view.width, view.height);
    if view.sent.as_ref() == Some(&canvas) {
        return Ok(());
    }
    let set = SetCanvas {
        surface: Surface::Atlas,
        canvas: Some(canvas.clone()),
    };
    output.write_all(notification_line(METHOD_SET_CANVAS, &set).as_bytes())?;
    output.flush()?;
    view.sent = Some(canvas);
    Ok(())
}
