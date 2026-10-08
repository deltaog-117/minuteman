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

//! What can be done to a device from the deck or the wiring diagram: open it (mounting it first if
//! it is not mounted), mount or unmount it, eject it, and send the carried files to it. The slow
//! ones run `udisksctl` on the blocking pool, one at a time, and report what came of it as an
//! `Event` the caller picks up on a later turn of the loop; none of them waits.
//!
//! Shared by both views, so a device behaves the same wherever it is chosen.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use devices::udisks::{self, Action};
use devices::{Device, Kind};

use crate::cargo_fit::{self, Verdict};

/// What finished in the background, for the caller to show or do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Status(String),
    /// Go to this folder, saying `String` once there.
    Goto(PathBuf, String),
}

/// What to do to one device, in order.
struct Plan {
    title: String,
    steps: Vec<(Action, String)>,
    /// Go to where it was mounted, once it is.
    then_open: bool,
}

pub struct Actions {
    handle: tokio::runtime::Handle,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    /// Whether something is being done right now: one thing at a time.
    working: Arc<AtomicBool>,
    pending: VecDeque<Event>,
}

impl Actions {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        let (tx, rx) = channel();
        Self {
            handle,
            tx,
            rx,
            working: Arc::new(AtomicBool::new(false)),
            pending: VecDeque::new(),
        }
    }

    /// Queues a message for the caller to show.
    pub fn say(&self, text: impl Into<String>) {
        let _ = self.tx.send(Event::Status(text.into()));
    }

    /// Whether a mount, unmount or eject is in progress.
    pub fn is_working(&self) -> bool {
        self.working.load(Ordering::Acquire)
    }

    /// Takes in what has finished since the last call, and says whether anything did, so the
    /// caller can look at the machine again at once: a mount changes what is mounted.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok(event) = self.rx.try_recv() {
            self.pending.push_back(event);
            any = true;
        }
        any
    }

    /// The next message or folder change from background work, if any.
    pub fn take_event(&mut self) -> Option<Event> {
        self.pending.pop_front()
    }

    /// Opening a mounted device is going to its folder, which the caller does; one that is not
    /// mounted is mounted first and opened when that is done.
    pub fn open(&self, device: &Device) -> Option<PathBuf> {
        if let Some(mount) = &device.mount {
            return Some(mount.clone());
        }
        self.start(Plan {
            title: device.title(),
            steps: vec![(Action::Mount, device.node.clone())],
            then_open: true,
        });
        None
    }

    /// Mounts a device that is not mounted and unmounts one that is.
    pub fn toggle(&self, device: &Device) {
        if device.kind == Kind::Network {
            self.say("a network share cannot be mounted or unmounted from here");
            return;
        }
        let action = if device.is_mounted() {
            Action::Unmount
        } else {
            Action::Mount
        };
        self.start(Plan {
            title: device.title(),
            steps: vec![(action, device.node.clone())],
            then_open: false,
        });
    }

    /// Unmounts a removable device if it is mounted and powers its disk off, so it is safe to pull
    /// out.
    pub fn eject(&self, device: &Device) {
        if !device.can_eject() {
            self.say(format!("{} cannot be ejected", device.title()));
            return;
        }
        let mut steps = Vec::new();
        if device.is_mounted() {
            steps.push((Action::Unmount, device.node.clone()));
        }
        steps.push((Action::PowerOff, device.disk.clone()));
        self.start(Plan {
            title: device.title(),
            steps,
            then_open: false,
        });
    }

    /// Unmounts every mounted volume in `mounted`, all on the disk `disk`, then powers the disk
    /// off, so the whole stick can be pulled out. `title` names it in the messages.
    pub fn eject_all(&self, title: &str, mounted: &[Device], disk: &str) {
        let mut steps: Vec<(Action, String)> = mounted
            .iter()
            .map(|device| (Action::Unmount, device.node.clone()))
            .collect();
        steps.push((Action::PowerOff, disk.to_string()));
        self.start(Plan {
            title: title.to_string(),
            steps,
            then_open: false,
        });
    }

    /// Where to send the carried files on `device`, or `None` after saying why not. The check is
    /// made here, before any copy starts, which is the point of asking.
    pub fn send_to(&self, device: &Device, verdict: Option<Verdict>) -> Option<PathBuf> {
        let title = device.title();
        let Some(mount) = device.mount.clone() else {
            self.say(cargo_fit::refusal(&Verdict::Unmounted, &title));
            return None;
        };
        match verdict {
            Some(verdict) if !verdict.allows() => {
                self.say(cargo_fit::refusal(&verdict, &title));
                None
            }
            _ => Some(mount),
        }
    }

    /// Runs `plan` on the blocking pool, one thing at a time, and reports what came of it.
    fn start(&self, plan: Plan) {
        if self.working.swap(true, Ordering::AcqRel) {
            self.say("still working on the last one");
            return;
        }
        self.say(match plan.steps.last() {
            Some((Action::Mount, _)) => format!("mounting {}…", plan.title),
            Some((Action::Unmount, _)) => format!("unmounting {}…", plan.title),
            _ => format!("ejecting {}…", plan.title),
        });
        let (tx, working) = (self.tx.clone(), Arc::clone(&self.working));
        self.handle.spawn_blocking(move || {
            let event = carry_out(&plan);
            let _ = tx.send(event);
            working.store(false, Ordering::Release);
        });
    }
}

/// Does each step of `plan` in turn, stopping at the first that fails.
fn carry_out(plan: &Plan) -> Event {
    let mut mounted_at = None;
    for (action, node) in &plan.steps {
        match udisks::run(*action, node) {
            Ok(done) => mounted_at = done.mounted_at.or(mounted_at),
            Err(why) => return Event::Status(format!("{}: {why}", plan.title)),
        }
    }
    match (plan.steps.last(), mounted_at) {
        (Some((Action::Mount, _)), Some(at)) if plan.then_open => {
            Event::Goto(at, format!("mounted {}", plan.title))
        }
        (Some((Action::Mount, _)), at) => Event::Status(match at {
            Some(at) => format!("mounted {} at {}", plan.title, at.display()),
            None => format!("mounted {}", plan.title),
        }),
        (Some((Action::Unmount, _)), _) => Event::Status(format!("unmounted {}", plan.title)),
        _ => Event::Status(format!("{} ejected, safe to remove", plan.title)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(kind: Kind, mount: Option<&str>) -> Device {
        Device {
            node: "sdb1".into(),
            disk: "sdb".into(),
            label: Some("STICK".into()),
            fs: Some("ext4".into()),
            mount: mount.map(PathBuf::from),
            total: 1000,
            free: 500,
            kind,
        }
    }

    fn said(actions: &mut Actions) -> Vec<String> {
        actions.poll();
        let mut out = Vec::new();
        while let Some(Event::Status(text)) = actions.take_event() {
            out.push(text);
        }
        out
    }

    fn actions() -> (tokio::runtime::Runtime, Actions) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let actions = Actions::new(runtime.handle().clone());
        (runtime, actions)
    }

    #[test]
    fn opening_a_mounted_device_is_going_to_its_folder() {
        let (_rt, actions) = actions();
        assert_eq!(
            actions.open(&device(Kind::Removable, Some("/run/media/me/STICK"))),
            Some(PathBuf::from("/run/media/me/STICK"))
        );
        assert!(!actions.is_working());
    }

    #[test]
    fn a_network_share_cannot_be_mounted_and_a_fixed_disk_cannot_be_ejected() {
        let (_rt, mut actions) = actions();
        actions.toggle(&device(Kind::Network, Some("/mnt/nas")));
        actions.eject(&device(Kind::Internal, Some("/")));
        let said = said(&mut actions);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].contains("network share") && said[1].contains("cannot be ejected"));
        assert!(!actions.is_working());
    }

    #[test]
    fn sending_is_allowed_where_it_fits_and_refused_with_the_reason_where_it_does_not() {
        let (_rt, mut actions) = actions();
        let mounted = device(Kind::Removable, Some("/run/media/me/STICK"));
        assert_eq!(
            actions.send_to(&mounted, Some(Verdict::Fits { left: 1 })),
            Some(PathBuf::from("/run/media/me/STICK"))
        );
        assert!(
            actions.send_to(&mounted, None).is_some(),
            "nothing carried is the caller's to report"
        );
        assert_eq!(
            actions.send_to(&mounted, Some(Verdict::Short { need: 5 })),
            None
        );
        assert_eq!(actions.send_to(&device(Kind::Removable, None), None), None);
        let said = said(&mut actions);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].contains("needs") && said[1].contains("not mounted"));
    }
}
