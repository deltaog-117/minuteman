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

//! What to do to a device and in what order: open it (mounting it first if it is not mounted),
//! mount or unmount it, eject it. A [`Plan`] is plain data until [`Plan::run`], which runs
//! `udisksctl` and waits, so it must run off the interface thread. The deck's plugin and the
//! wiring diagram both run their actions through it, so a device behaves the same wherever it is
//! chosen.

use std::path::PathBuf;

use super::udisks::{self, Action};
use super::{Device, Kind};

/// What a finished plan came to, for the caller to show or do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Status(String),
    /// Go to this folder, saying the text once there.
    Goto(PathBuf, String),
}

/// What to do to one device, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    title: String,
    steps: Vec<(Action, String)>,
    /// Go to where it was mounted, once it is.
    then_open: bool,
}

impl Plan {
    /// Mounts a device that is not mounted and goes to it.
    pub fn mount_and_open(device: &Device) -> Self {
        Self {
            title: device.title(),
            steps: vec![(Action::Mount, device.node.clone())],
            then_open: true,
        }
    }

    /// Mounts a device that is not mounted and unmounts one that is.
    ///
    /// # Errors
    ///
    /// Why a network share cannot be: it is mounted by the system, not from here.
    pub fn toggle(device: &Device) -> Result<Self, String> {
        if device.kind == Kind::Network {
            return Err("a network share cannot be mounted or unmounted from here".to_string());
        }
        let action = if device.is_mounted() {
            Action::Unmount
        } else {
            Action::Mount
        };
        Ok(Self {
            title: device.title(),
            steps: vec![(action, device.node.clone())],
            then_open: false,
        })
    }

    /// Unmounts a removable device if it is mounted and powers its disk off, so it is safe to
    /// pull out.
    ///
    /// # Errors
    ///
    /// Why the device cannot be ejected: it is not removable.
    pub fn eject(device: &Device) -> Result<Self, String> {
        if !device.can_eject() {
            return Err(format!("{} cannot be ejected", device.title()));
        }
        let mut steps = Vec::new();
        if device.is_mounted() {
            steps.push((Action::Unmount, device.node.clone()));
        }
        steps.push((Action::PowerOff, device.disk.clone()));
        Ok(Self {
            title: device.title(),
            steps,
            then_open: false,
        })
    }

    /// Unmounts every volume in `mounted`, all on the disk `disk`, then powers the disk off, so
    /// the whole stick can be pulled out. `title` names it in the messages.
    pub fn eject_all(title: &str, mounted: &[Device], disk: &str) -> Self {
        let mut steps: Vec<(Action, String)> = mounted
            .iter()
            .map(|device| (Action::Unmount, device.node.clone()))
            .collect();
        steps.push((Action::PowerOff, disk.to_string()));
        Self {
            title: title.to_string(),
            steps,
            then_open: false,
        }
    }

    /// The message to show while the plan runs.
    pub fn starting(&self) -> String {
        match self.steps.last() {
            Some((Action::Mount, _)) => format!("mounting {}…", self.title),
            Some((Action::Unmount, _)) => format!("unmounting {}…", self.title),
            _ => format!("ejecting {}…", self.title),
        }
    }

    /// Does each step in turn, stopping at the first that fails, and says what came of it.
    pub fn run(&self) -> Outcome {
        let mut mounted_at = None;
        for (action, node) in &self.steps {
            match udisks::run(*action, node) {
                Ok(done) => mounted_at = done.mounted_at.or(mounted_at),
                Err(why) => return Outcome::Status(format!("{}: {why}", self.title)),
            }
        }
        let title = &self.title;
        match (self.steps.last(), mounted_at) {
            (Some((Action::Mount, _)), Some(at)) if self.then_open => {
                Outcome::Goto(at, format!("mounted {title}"))
            }
            (Some((Action::Mount, _)), at) => Outcome::Status(match at {
                Some(at) => format!("mounted {title} at {}", at.display()),
                None => format!("mounted {title}"),
            }),
            (Some((Action::Unmount, _)), _) => Outcome::Status(format!("unmounted {title}")),
            _ => Outcome::Status(format!("{title} ejected, safe to remove")),
        }
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

    fn steps(plan: &Plan) -> Vec<(Action, &str)> {
        plan.steps
            .iter()
            .map(|(action, node)| (*action, node.as_str()))
            .collect()
    }

    #[test]
    fn a_network_share_cannot_be_toggled_and_a_fixed_disk_cannot_be_ejected() {
        let share = Plan::toggle(&device(Kind::Network, Some("/mnt/nas")));
        assert!(share.unwrap_err().contains("network share"));
        let fixed = Plan::eject(&device(Kind::Internal, Some("/")));
        assert!(fixed.unwrap_err().contains("cannot be ejected"));
    }

    #[test]
    fn toggling_mounts_what_is_not_mounted_and_unmounts_what_is() {
        let off = Plan::toggle(&device(Kind::Removable, None)).unwrap();
        assert_eq!(steps(&off), [(Action::Mount, "sdb1")]);
        assert_eq!(off.starting(), "mounting STICK…");
        let on = Plan::toggle(&device(Kind::Removable, Some("/mnt/s"))).unwrap();
        assert_eq!(steps(&on), [(Action::Unmount, "sdb1")]);
        assert_eq!(on.starting(), "unmounting STICK…");
    }

    #[test]
    fn ejecting_unmounts_first_only_when_mounted_then_powers_the_disk_off() {
        let mounted = Plan::eject(&device(Kind::Removable, Some("/mnt/s"))).unwrap();
        assert_eq!(
            steps(&mounted),
            [(Action::Unmount, "sdb1"), (Action::PowerOff, "sdb")]
        );
        let bare = Plan::eject(&device(Kind::Removable, None)).unwrap();
        assert_eq!(steps(&bare), [(Action::PowerOff, "sdb")]);
        assert_eq!(bare.starting(), "ejecting STICK…");
    }

    #[test]
    fn ejecting_a_whole_stick_unmounts_every_volume_before_the_power_goes() {
        let volumes = [
            device(Kind::Removable, Some("/a")),
            Device {
                node: "sdb2".into(),
                ..device(Kind::Removable, Some("/b"))
            },
        ];
        let plan = Plan::eject_all("STICK", &volumes, "sdb");
        assert_eq!(
            steps(&plan),
            [
                (Action::Unmount, "sdb1"),
                (Action::Unmount, "sdb2"),
                (Action::PowerOff, "sdb")
            ]
        );
    }

    #[test]
    fn opening_a_device_mounts_it_and_goes_to_where_it_landed() {
        let plan = Plan::mount_and_open(&device(Kind::Removable, None));
        assert!(plan.then_open);
        assert_eq!(steps(&plan), [(Action::Mount, "sdb1")]);
    }
}
