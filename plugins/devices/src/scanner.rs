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

//! Looks at the machine on a thread of its own, so that a `statvfs` on a network share that has
//! gone away can block this thread and nothing else. While the host is watching it scans once a
//! second and sends the whole list with how busy each disk is; told to stop, it sleeps until told
//! to watch again, so a rail that is not showing costs nothing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use plugin_api::client::notification_line;
use plugin_api::{METHOD_SET_DEVICES, SetDevices, WireDevice};
use shared::devices::Device;
use shared::devices::activity::{Activity, Counters};
use shared::devices::plan::{Outcome, Plan};
use shared::devices::probe::Sys;

/// How often the machine is looked at again while watched.
pub const SCAN_EVERY: Duration = Duration::from_secs(1);

/// How long a stopped scanner sleeps between looks at its inbox; it only ever wakes for a command.
const IDLE: Duration = Duration::from_secs(3600);

/// What the plugin knows of the machine: the devices there are, the disks' running byte counts
/// and the means to change a mount. A trait so a conversation can be played in a test without
/// touching the real disks.
pub trait Machine: Send + Sync + 'static {
    fn scan(&self) -> Vec<Device>;
    fn counters(&self) -> HashMap<String, Counters>;
    /// Carries `plan` out, waiting for it.
    fn run(&self, plan: &Plan) -> Outcome;
}

/// This machine, read from `/proc`, `/sys` and the udev database and changed with `udisksctl`.
pub struct RealMachine(Sys);

impl RealMachine {
    pub fn new() -> Self {
        Self(Sys::real())
    }
}

impl Default for RealMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine for RealMachine {
    fn scan(&self) -> Vec<Device> {
        self.0.scan()
    }

    fn counters(&self) -> HashMap<String, Counters> {
        self.0.counters()
    }

    fn run(&self, plan: &Plan) -> Outcome {
        plan.run()
    }
}

/// What the scanner is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Start (`true`) or stop watching.
    Watch(bool),
    /// Look again now, if watching: something just changed what is mounted.
    Refresh,
}

/// The devices of the last scan, for the session to find the one an action names.
pub type Latest = Arc<Mutex<Vec<Device>>>;

/// Starts the scanner. It ends when every sender of the returned channel is gone.
pub fn spawn(
    machine: Arc<dyn Machine>,
    out: Sender<String>,
    latest: Latest,
    busy: Arc<AtomicBool>,
) -> Sender<Command> {
    let (commands, inbox) = channel();
    let spawned = thread::Builder::new()
        .name("devices-scan".into())
        .spawn(move || scan_loop(machine.as_ref(), &inbox, &out, &latest, &busy));
    // Without the thread nothing is ever listed; the rail then stays empty, which says so.
    drop(spawned);
    commands
}

fn scan_loop(
    machine: &dyn Machine,
    inbox: &Receiver<Command>,
    out: &Sender<String>,
    latest: &Latest,
    busy: &AtomicBool,
) {
    let mut watching = false;
    let mut activity = Activity::new();
    let mut last: Option<Instant> = None;
    loop {
        let wait = match (watching, last) {
            (true, Some(at)) => SCAN_EVERY.saturating_sub(at.elapsed()),
            (true, None) => Duration::ZERO,
            (false, _) => IDLE,
        };
        let scan_now = match inbox.recv_timeout(wait) {
            Ok(Command::Watch(on)) => {
                watching = on;
                if !on {
                    // The next watch starts its rates from nothing instead of from a gap.
                    activity = Activity::new();
                    last = None;
                }
                on
            }
            Ok(Command::Refresh) => watching,
            Err(RecvTimeoutError::Timeout) => watching,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if !scan_now {
            continue;
        }
        let devices = machine.scan();
        let now = Instant::now();
        let seconds = last.map_or(1.0, |at| now.duration_since(at).as_secs_f64());
        activity.update(&machine.counters(), seconds);
        last = Some(now);
        let wire = devices
            .iter()
            .map(|device| {
                WireDevice::new(
                    device,
                    activity.rates(&device.node),
                    activity.history(&device.node).collect(),
                )
            })
            .collect();
        *latest.lock().unwrap_or_else(PoisonError::into_inner) = devices;
        let set = SetDevices {
            devices: wire,
            busy: busy.load(Ordering::Acquire),
        };
        if out
            .send(notification_line(METHOD_SET_DEVICES, &set))
            .is_err()
        {
            return;
        }
    }
}

/// A machine that holds what it was given and runs every plan to "done", for tests.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    pub struct Fake {
        pub devices: Mutex<Vec<Device>>,
        pub ran: Mutex<Vec<String>>,
    }

    impl Fake {
        pub fn holding(devices: Vec<Device>) -> Arc<Self> {
            Arc::new(Self {
                devices: Mutex::new(devices),
                ran: Mutex::new(Vec::new()),
            })
        }
    }

    impl Machine for Fake {
        fn scan(&self) -> Vec<Device> {
            self.devices.lock().unwrap().clone()
        }

        fn counters(&self) -> HashMap<String, Counters> {
            HashMap::new()
        }

        fn run(&self, plan: &Plan) -> Outcome {
            self.ran.lock().unwrap().push(plan.starting());
            Outcome::Status("done".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use shared::devices::Kind;

    use super::fake::Fake;
    use super::*;

    fn device(node: &str) -> Device {
        Device {
            node: node.into(),
            disk: "sda".into(),
            label: None,
            fs: None,
            mount: Some(PathBuf::from("/")),
            total: 10,
            free: 5,
            kind: Kind::Internal,
        }
    }

    fn started(devices: Vec<Device>) -> (Sender<Command>, Receiver<String>, Latest) {
        let (out, lines) = channel();
        let latest = Latest::default();
        let commands = spawn(
            Fake::holding(devices),
            out,
            Arc::clone(&latest),
            Arc::new(AtomicBool::new(false)),
        );
        (commands, lines, latest)
    }

    fn nodes(line: &str) -> Vec<String> {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["method"], METHOD_SET_DEVICES);
        value["params"]["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["node"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn nothing_is_sent_until_watched_and_a_list_arrives_at_once_when_it_is() {
        let (commands, lines, latest) = started(vec![device("sda1")]);
        assert!(lines.recv_timeout(Duration::from_millis(150)).is_err());
        commands.send(Command::Watch(true)).unwrap();
        let line = lines.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(nodes(&line), ["sda1"]);
        assert_eq!(latest.lock().unwrap().len(), 1);
    }

    #[test]
    fn it_keeps_sending_while_watched_and_stops_when_told() {
        let (commands, lines, _) = started(vec![device("sda1")]);
        commands.send(Command::Watch(true)).unwrap();
        lines.recv_timeout(Duration::from_secs(5)).unwrap();
        lines
            .recv_timeout(Duration::from_secs(5))
            .expect("a second list");
        commands.send(Command::Watch(false)).unwrap();
        // Whatever was already on its way arrives; then it goes quiet.
        thread::sleep(Duration::from_millis(100));
        while lines.try_recv().is_ok() {}
        assert!(lines.recv_timeout(Duration::from_millis(1300)).is_err());
    }

    #[test]
    fn a_refresh_looks_again_at_once_only_while_watched() {
        let (commands, lines, _) = started(vec![device("sda1")]);
        commands.send(Command::Refresh).unwrap();
        assert!(lines.recv_timeout(Duration::from_millis(150)).is_err());
        commands.send(Command::Watch(true)).unwrap();
        lines.recv_timeout(Duration::from_secs(5)).unwrap();
        commands.send(Command::Refresh).unwrap();
        lines
            .recv_timeout(Duration::from_millis(500))
            .expect("not after the whole interval");
    }

    #[test]
    fn the_scanner_ends_when_the_last_sender_is_dropped() {
        let (commands, lines, _) = started(Vec::new());
        commands.send(Command::Watch(true)).unwrap();
        lines.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(commands);
        // Its own sender to the output goes with it, so the output ends.
        let ended = (0..50).any(|_| {
            lines.recv_timeout(Duration::from_millis(100)) == Err(RecvTimeoutError::Disconnected)
        });
        assert!(ended);
    }
}
