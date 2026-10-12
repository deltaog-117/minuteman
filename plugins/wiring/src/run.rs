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
//! every `TICK` to pick up what the scans have found, finish what was started on a device, and
//! draw again if the picture changed; while it is closed it sleeps until the host writes, so a
//! closed diagram costs nothing.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::thread;
use std::time::{Duration, Instant};

use plugin_api::client::{Incoming, notification_line, parse_incoming};
use plugin_api::{
    API_VERSION, Button, Canvas, CanvasEvent, CanvasInput, CanvasNote, Carrying, EVENT_CANVAS,
    EVENT_INIT, METHOD_CANVAS_NOTE, METHOD_SET_CANVAS, SetCanvas, Surface,
};
use serde_json::json;
use shared::cargo_fit::{Carry, Load};
use shared::devices::actions::Event;
use shared::{LocalVfs, Vfs};

use crate::canvas::build;
use crate::state::{Machine, Outcome, RealMachine, WiringState};

/// How often an open view looks at what the scans have found.
const TICK: Duration = Duration::from_millis(100);

enum Message {
    /// A line the host wrote.
    Host(String),
    /// The host closed its end.
    Gone,
}

/// A diagram that is showing.
struct Open {
    state: WiringState,
    width: u16,
    height: u16,
    /// The folder the browser is in, which the diagram unfolds toward and lights.
    here: PathBuf,
    /// The last canvas sent, so an unchanged picture is not sent again.
    sent: Option<Canvas>,
}

/// What the plugin looks at: the machine and the disk the folders are read from.
struct World {
    machine: Arc<dyn Machine>,
    vfs: Arc<dyn Vfs>,
}

/// Runs the plugin on this machine until the host closes its input.
pub fn run(input: impl BufRead + Send + 'static, output: impl Write) -> io::Result<()> {
    run_on(
        input,
        output,
        Arc::new(RealMachine::new()),
        Arc::new(LocalVfs),
    )
}

/// `run` over a machine and a disk of the caller's choosing, so a conversation can be played in a
/// test.
pub fn run_on(
    input: impl BufRead + Send + 'static,
    mut output: impl Write,
    machine: Arc<dyn Machine>,
    vfs: Arc<dyn Vfs>,
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

    let world = World { machine, vfs };
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
                if !handle(&line, &mut open, &world, &mut output)? {
                    return Ok(());
                }
            }
            Ok(Message::Gone) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {}
        }
        if let Some(view) = open.as_mut() {
            view.state.poll(Instant::now(), &view.here);
            tell(view, &mut output)?;
            draw(view, &mut output)?;
        }
    }
}

/// Acts on one line from the host. `false` ends the plugin.
fn handle(
    line: &str,
    open: &mut Option<Open>,
    world: &World,
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
                surface: Surface::Wiring,
                input,
            }) = params.canvas
            {
                apply(input, open, world, output)?;
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
    world: &World,
    output: &mut impl Write,
) -> io::Result<()> {
    match input {
        CanvasInput::Open {
            width,
            height,
            here,
        } => {
            // A fresh state every time: the view never kept anything between showings, and the
            // old one's folders are forgotten as it is dropped.
            let mut state =
                WiringState::new(Arc::clone(&world.vfs), Arc::clone(&world.machine), false);
            state.set_size(width, height);
            *open = Some(Open {
                state,
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
                view.state.set_size(width, height);
            }
        }
        CanvasInput::Context {
            here,
            hidden,
            carrying,
        } => {
            if let Some(view) = open.as_mut() {
                view.here = here;
                view.state.set_hidden(hidden);
                view.state.set_load(carrying.map(load_of));
            }
        }
        CanvasInput::Key { key, mods } => {
            if let Some(view) = open.as_mut() {
                let outcome = view.state.key(&key, mods, view.width);
                note(outcome, output)?;
                tell(view, output)?;
            }
        }
        CanvasInput::Click {
            x,
            y,
            button: Button::Left,
            ..
        } => {
            if let Some(view) = open.as_mut() {
                let outcome = view.state.click(x, y);
                note(outcome, output)?;
            }
        }
        CanvasInput::Wheel { down, .. } => {
            if let Some(view) = open.as_mut() {
                view.state.wheel(down, view.width);
            }
        }
        // The diagram has no menu.
        CanvasInput::Click { .. } => {}
    }
    Ok(())
}

fn load_of(carrying: Carrying) -> Load {
    Load {
        bytes: carrying.bytes,
        exact: carrying.exact,
        largest: carrying.largest,
        carry: if carrying.moving {
            Carry::Move
        } else {
            Carry::Copy
        },
        source: carrying.source,
    }
}

fn send(note: &CanvasNote, output: &mut impl Write) -> io::Result<()> {
    output.write_all(notification_line(METHOD_CANVAS_NOTE, note).as_bytes())?;
    output.flush()
}

/// Tells the host what the diagram asked of the browser.
fn note(outcome: Outcome, output: &mut impl Write) -> io::Result<()> {
    match outcome {
        Outcome::Pass | Outcome::Handled => Ok(()),
        Outcome::Goto(path) => send(
            &CanvasNote::Goto {
                path,
                text: String::new(),
            },
            output,
        ),
        Outcome::Send(into) => send(&CanvasNote::Send { into }, output),
    }
}

/// Passes on what finished in the background (a mount, an eject) and what the diagram has to say.
fn tell(view: &mut Open, output: &mut impl Write) -> io::Result<()> {
    while let Some(event) = view.state.take_event() {
        let note = match event {
            Event::Status(text) => CanvasNote::Status { text },
            Event::Goto(path, text) => CanvasNote::Goto { path, text },
        };
        send(&note, output)?;
    }
    Ok(())
}

/// Sends the picture if it differs from the last one sent.
fn draw(view: &mut Open, output: &mut impl Write) -> io::Result<()> {
    let canvas = build(&mut view.state, &view.here, view.width, view.height);
    if view.sent.as_ref() == Some(&canvas) {
        return Ok(());
    }
    let set = SetCanvas {
        surface: Surface::Wiring,
        canvas: Some(canvas.clone()),
    };
    output.write_all(notification_line(METHOD_SET_CANVAS, &set).as_bytes())?;
    output.flush()?;
    view.sent = Some(canvas);
    Ok(())
}
