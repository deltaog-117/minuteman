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

//! What the plugin does with each `devices` event the host writes: pass a `watch` to the scanner,
//! or turn a chosen device into an action on a thread of its own. An action reports through the
//! channel it was given, so the session never touches a stream and a whole conversation can be
//! played in a test.
//!
//! One action runs at a time, the way the interface used to hold them: a second one is refused
//! with a message instead of queued behind a `udisksctl` that may be waiting for a password.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, PoisonError};
use std::thread;

use plugin_api::client::notification_line;
use plugin_api::{DevicesCommand, DevicesNote, EVENT_DEVICES, EventParams, METHOD_DEVICES_NOTE};
use shared::devices::Device;
use shared::devices::plan::{Outcome, Plan};

use crate::scanner::{self, Command, Latest, Machine};

pub struct Session {
    out: Sender<String>,
    machine: Arc<dyn Machine>,
    commands: Sender<Command>,
    latest: Latest,
    busy: Arc<AtomicBool>,
}

impl Session {
    /// A session whose scanner and actions write their lines to `out`.
    pub fn new(out: Sender<String>, machine: Arc<dyn Machine>) -> Self {
        let latest = Latest::default();
        let busy = Arc::new(AtomicBool::new(false));
        let commands = scanner::spawn(
            Arc::clone(&machine),
            out.clone(),
            Arc::clone(&latest),
            Arc::clone(&busy),
        );
        Self {
            out,
            machine,
            commands,
            latest,
            busy,
        }
    }

    /// Acts on an event the host sent. Anything but a `devices` event is not this plugin's
    /// business.
    pub fn on_event(&self, event: &EventParams) {
        if event.name != EVENT_DEVICES {
            return;
        }
        match &event.devices {
            Some(DevicesCommand::Watch { on }) => {
                let _ = self.commands.send(Command::Watch(*on));
            }
            Some(DevicesCommand::Open { node }) => self.open(node),
            Some(DevicesCommand::Toggle { node }) => {
                self.act(node, Plan::toggle);
            }
            Some(DevicesCommand::Eject { node }) => {
                self.act(node, Plan::eject);
            }
            None => {}
        }
    }

    /// Goes to a mounted device's folder, or mounts it and then goes there.
    fn open(&self, node: &str) {
        let Some(device) = self.find(node) else {
            self.say(gone(node));
            return;
        };
        match device.mount {
            Some(path) => self.send(&DevicesNote::Goto {
                path,
                text: "opened device".to_string(),
            }),
            None => self.start(Plan::mount_and_open(&device)),
        }
    }

    /// Builds the plan for the device called `node` and starts it, or says why it cannot be done.
    fn act(&self, node: &str, plan: impl FnOnce(&Device) -> Result<Plan, String>) {
        let Some(device) = self.find(node) else {
            self.say(gone(node));
            return;
        };
        match plan(&device) {
            Ok(plan) => self.start(plan),
            Err(why) => self.say(why),
        }
    }

    fn find(&self, node: &str) -> Option<Device> {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|device| device.node == node)
            .cloned()
    }

    fn say(&self, text: impl Into<String>) {
        self.send(&DevicesNote::Status { text: text.into() });
    }

    fn send(&self, note: &DevicesNote) {
        let _ = self.out.send(notification_line(METHOD_DEVICES_NOTE, note));
    }

    /// Runs `plan` on a thread, one thing at a time, and reports what came of it.
    fn start(&self, plan: Plan) {
        if self.busy.swap(true, Ordering::AcqRel) {
            self.say("still working on the last one");
            return;
        }
        self.say(plan.starting());
        // The list says a mount is under way, so the rail marks the row at once.
        let _ = self.commands.send(Command::Refresh);
        let (out, machine, busy, commands) = (
            self.out.clone(),
            Arc::clone(&self.machine),
            Arc::clone(&self.busy),
            self.commands.clone(),
        );
        let spawned = thread::Builder::new().name("devices-act".into()).spawn({
            let (out, busy) = (out.clone(), Arc::clone(&busy));
            move || {
                let note = match machine.run(&plan) {
                    Outcome::Status(text) => DevicesNote::Status { text },
                    Outcome::Goto(path, text) => DevicesNote::Goto { path, text },
                };
                busy.store(false, Ordering::Release);
                let _ = out.send(notification_line(METHOD_DEVICES_NOTE, &note));
                // A mount changes what is mounted: look again at once.
                let _ = commands.send(Command::Refresh);
            }
        });
        if spawned.is_err() {
            busy.store(false, Ordering::Release);
            self.say("could not start the action");
        }
    }
}

fn gone(node: &str) -> String {
    format!("{node} is no longer there")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    use serde_json::Value;
    use shared::devices::Kind;

    use super::*;
    use crate::scanner::fake::Fake;

    fn device(node: &str, kind: Kind, mount: Option<&str>) -> Device {
        Device {
            node: node.into(),
            disk: "sdb".into(),
            label: Some("STICK".into()),
            fs: None,
            mount: mount.map(PathBuf::from),
            total: 10,
            free: 5,
            kind,
        }
    }

    fn event(command: DevicesCommand) -> EventParams {
        serde_json::from_value(
            serde_json::to_value(plugin_api::HostEvent::devices(command)).unwrap()["params"]
                .clone(),
        )
        .unwrap()
    }

    /// A session over three devices, watched so that the list is known, with the list line read.
    fn watched() -> (Session, Receiver<String>, Arc<Fake>) {
        let machine = Fake::holding(vec![
            device("sdb1", Kind::Removable, Some("/run/media/me/STICK")),
            device("sdc1", Kind::Removable, None),
            device("nas:/x", Kind::Network, Some("/mnt/nas")),
        ]);
        let (out, lines) = channel();
        let session = Session::new(out, Arc::clone(&machine) as Arc<dyn Machine>);
        session.on_event(&event(DevicesCommand::Watch { on: true }));
        let first = lines.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(first.contains(plugin_api::METHOD_SET_DEVICES));
        (session, lines, machine)
    }

    /// The next note, skipping the lists that arrive meanwhile.
    fn next_note(lines: &Receiver<String>) -> Value {
        loop {
            let line = lines.recv_timeout(Duration::from_secs(5)).expect("a note");
            let value: Value = serde_json::from_str(&line).unwrap();
            if value["method"] == METHOD_DEVICES_NOTE {
                return value["params"].clone();
            }
        }
    }

    #[test]
    fn opening_a_mounted_device_goes_to_its_folder() {
        let (session, lines, _) = watched();
        session.on_event(&event(DevicesCommand::Open {
            node: "sdb1".into(),
        }));
        let note = next_note(&lines);
        assert_eq!(note["kind"], "goto");
        assert_eq!(note["path"], "/run/media/me/STICK");
    }

    #[test]
    fn opening_one_that_is_not_mounted_says_it_is_mounting_then_what_came_of_it() {
        let (session, lines, machine) = watched();
        session.on_event(&event(DevicesCommand::Open {
            node: "sdc1".into(),
        }));
        assert_eq!(next_note(&lines)["text"], "mounting STICK…");
        assert_eq!(next_note(&lines)["text"], "done");
        assert_eq!(*machine.ran.lock().unwrap(), ["mounting STICK…"]);
    }

    #[test]
    fn a_network_share_and_a_fixed_disk_refuse_what_they_cannot_do() {
        let (session, lines, machine) = watched();
        session.on_event(&event(DevicesCommand::Toggle {
            node: "nas:/x".into(),
        }));
        assert!(
            next_note(&lines)["text"]
                .as_str()
                .unwrap()
                .contains("network share")
        );
        session.on_event(&event(DevicesCommand::Eject {
            node: "nas:/x".into(),
        }));
        assert!(
            next_note(&lines)["text"]
                .as_str()
                .unwrap()
                .contains("cannot be ejected")
        );
        assert!(machine.ran.lock().unwrap().is_empty(), "nothing was run");
    }

    #[test]
    fn a_device_that_has_gone_is_said_to_have_gone() {
        let (session, lines, _) = watched();
        session.on_event(&event(DevicesCommand::Eject {
            node: "sdz9".into(),
        }));
        assert_eq!(next_note(&lines)["text"], "sdz9 is no longer there");
    }

    #[test]
    fn a_second_action_while_one_runs_is_refused() {
        let (session, lines, _) = watched();
        session.busy.store(true, Ordering::Release);
        session.on_event(&event(DevicesCommand::Toggle {
            node: "sdc1".into(),
        }));
        assert_eq!(next_note(&lines)["text"], "still working on the last one");
        assert!(
            session.busy.load(Ordering::Acquire),
            "the first is still its own"
        );
    }

    #[test]
    fn the_list_says_busy_while_an_action_runs_and_not_after() {
        let (session, lines, _) = watched();
        session.on_event(&event(DevicesCommand::Toggle {
            node: "sdc1".into(),
        }));
        assert_eq!(next_note(&lines)["text"], "mounting STICK…");
        assert_eq!(next_note(&lines)["text"], "done");
        // A list scanned while the action still ran may arrive after its note; the refresh that
        // follows the action must bring one with busy off.
        let busy = loop {
            let line = lines.recv_timeout(Duration::from_secs(5)).expect("a list");
            let value: Value = serde_json::from_str(&line).unwrap();
            if value["method"] == plugin_api::METHOD_SET_DEVICES && value["params"]["busy"] == false
            {
                break false;
            }
        };
        assert!(!busy);
    }

    #[test]
    fn an_event_that_is_not_for_the_rail_is_ignored() {
        let (session, lines, _) = watched();
        let mut other = event(DevicesCommand::Watch { on: false });
        other.name = "dir_changed".into();
        session.on_event(&other);
        // Still watching: the next list comes.
        loop {
            let line = lines
                .recv_timeout(Duration::from_secs(5))
                .expect("still watching");
            if line.contains(plugin_api::METHOD_SET_DEVICES) {
                break;
            }
        }
    }
}
