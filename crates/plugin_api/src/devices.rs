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

//! What a plugin that fills the deck's rail of devices exchanges with the host. The interface
//! tells the plugin when the rail is showing (the `devices` event, sent to the plugin that
//! `provides = ["devices"]`, with a `watch` command) and which device the user chose to open,
//! mount, unmount or eject. The plugin looks at the machine, sends the list of devices with how
//! busy each is (`set_devices`, about once a second while watched) and reports what came of an
//! action (`devices_note`: a message to show, or a folder to go to). The plugin never draws and
//! never decides what fits: the interface keeps the rail, the selection, the keys and the
//! check of whether carried files will fit.
//!
//! A plugin may be wrong or hostile and its text ends up in the user's terminal, so
//! [`SetDevices::checked`] and [`DevicesNote::checked`] clean what the host passes on.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use shared::devices::activity::{HISTORY, Rates};
use shared::devices::{Device, Kind};

use crate::view::is_unsafe;

/// The event name a devices plugin gets from the interface.
pub const EVENT_DEVICES: &str = "devices";
/// The notification that gives the rail its devices: `set_devices`.
pub const METHOD_SET_DEVICES: &str = "set_devices";
/// The notification that says what came of an action: `devices_note`.
pub const METHOD_DEVICES_NOTE: &str = "devices_note";
/// What a plugin lists under `provides` to be the one that fills the rail of devices.
pub const PROVIDES_DEVICES: &str = "devices";

/// Most devices a list holds; more are dropped.
pub const MAX_DEVICES: usize = 256;
/// Most characters of any one text of a device or of a note; longer text is cut.
pub const MAX_DEVICE_TEXT_CHARS: usize = 512;

/// What the interface asks of the plugin, in the `devices` field of a `devices` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum DevicesCommand {
    /// The rail is showing (`on`) or has gone: look at the machine only while it is showing.
    Watch { on: bool },
    /// Open `node`: go to its folder, mounting it first if it is not mounted.
    Open { node: String },
    /// Mount `node` if it is not mounted, unmount it if it is.
    Toggle { node: String },
    /// Unmount `node` if need be and power its disk off.
    Eject { node: String },
}

/// Where a device is attached, as the wire spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Internal,
    Removable,
    Network,
}

impl From<Kind> for DeviceKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Internal => Self::Internal,
            Kind::Removable => Self::Removable,
            Kind::Network => Self::Network,
        }
    }
}

impl From<DeviceKind> for Kind {
    fn from(kind: DeviceKind) -> Self {
        match kind {
            DeviceKind::Internal => Self::Internal,
            DeviceKind::Removable => Self::Removable,
            DeviceKind::Network => Self::Network,
        }
    }
}

/// One device and how busy it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireDevice {
    /// What the kernel calls it, or the remote's own name for a network mount. Unique in a list.
    pub node: String,
    /// The whole disk it is part of.
    pub disk: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount: Option<PathBuf>,
    /// Bytes, and how many of them are free to an ordinary user. Zero when unknown.
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub free: u64,
    pub kind: DeviceKind,
    /// Bytes a second being read and written right now.
    #[serde(default)]
    pub read: u64,
    #[serde(default)]
    pub written: u64,
    /// Combined read and write rate over the last samples, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<u64>,
}

impl WireDevice {
    /// `device` with its current `rates` and `history`.
    pub fn new(device: &Device, rates: Rates, history: Vec<u64>) -> Self {
        Self {
            node: device.node.clone(),
            disk: device.disk.clone(),
            label: device.label.clone(),
            fs: device.fs.clone(),
            mount: device.mount.clone(),
            total: device.total,
            free: device.free,
            kind: device.kind.into(),
            read: rates.read,
            written: rates.written,
            history,
        }
    }

    /// The device itself, without how busy it is.
    pub fn device(&self) -> Device {
        Device {
            node: self.node.clone(),
            disk: self.disk.clone(),
            label: self.label.clone(),
            fs: self.fs.clone(),
            mount: self.mount.clone(),
            total: self.total,
            free: self.free,
            kind: self.kind.into(),
        }
    }

    pub fn rates(&self) -> Rates {
        Rates {
            read: self.read,
            written: self.written,
        }
    }
}

/// The parameters of `set_devices`: every device there is now, which replaces the last list.
/// `busy` says a mount, unmount or eject is in progress, so the interface can mark the chosen row.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SetDevices {
    pub devices: Vec<WireDevice>,
    #[serde(default)]
    pub busy: bool,
}

/// The parameters of `devices_note`: what an action came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DevicesNote {
    /// A message for the status line.
    Status { text: String },
    /// Go to `path`, saying `text` once there.
    Goto { path: PathBuf, text: String },
}

fn cleaned(text: &mut String) {
    text.retain(|c| !is_unsafe(c));
    if let Some((cut, _)) = text.char_indices().nth(MAX_DEVICE_TEXT_CHARS) {
        text.truncate(cut);
    }
}

fn cleaned_option(text: &mut Option<String>) {
    if let Some(inner) = text {
        cleaned(inner);
    }
}

/// Whether a path carries a character that could act on the terminal when shown.
fn path_is_unsafe(path: &std::path::Path) -> bool {
    path.to_string_lossy().chars().any(is_unsafe)
}

impl SetDevices {
    /// This list with every text cleaned of control and direction-changing characters and cut
    /// short, the history cut to what a sparkline keeps, a device with no node or a node already
    /// listed dropped (the interface follows a device by its node), a mount path that could act
    /// on the terminal treated as no mount, and the list cut to [`MAX_DEVICES`].
    pub fn checked(mut self) -> Self {
        let mut seen = std::collections::HashSet::new();
        self.devices.retain_mut(|device| {
            cleaned(&mut device.node);
            cleaned(&mut device.disk);
            cleaned_option(&mut device.label);
            cleaned_option(&mut device.fs);
            if device.mount.as_deref().is_some_and(path_is_unsafe) {
                device.mount = None;
            }
            if device.history.len() > HISTORY {
                let excess = device.history.len() - HISTORY;
                device.history.drain(..excess);
            }
            !device.node.is_empty() && seen.insert(device.node.clone())
        });
        self.devices.truncate(MAX_DEVICES);
        self
    }
}

impl DevicesNote {
    /// This note with its text cleaned and cut short, or `None` for a folder to go to whose path
    /// could act on the terminal.
    pub fn checked(mut self) -> Option<Self> {
        match &mut self {
            Self::Status { text } => cleaned(text),
            Self::Goto { path, text } => {
                if path_is_unsafe(path) {
                    return None;
                }
                cleaned(text);
            }
        }
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;

    fn wire(node: &str) -> WireDevice {
        WireDevice {
            node: node.into(),
            disk: "sda".into(),
            label: None,
            fs: Some("ext4".into()),
            mount: Some("/".into()),
            total: 1000,
            free: 400,
            kind: DeviceKind::Internal,
            read: 0,
            written: 0,
            history: Vec::new(),
        }
    }

    #[test]
    fn a_command_has_its_documented_wire_shape() {
        assert_eq!(
            serde_json::to_value(DevicesCommand::Watch { on: true }).unwrap(),
            json!({"command": "watch", "on": true})
        );
        assert_eq!(
            serde_json::to_value(DevicesCommand::Eject {
                node: "sdb1".into()
            })
            .unwrap(),
            json!({"command": "eject", "node": "sdb1"})
        );
    }

    #[test]
    fn a_list_with_only_the_required_fields_reads_with_the_rest_empty() {
        let set: SetDevices = serde_json::from_value(json!({
            "devices": [{"node": "sda1", "disk": "sda", "kind": "removable"}]
        }))
        .unwrap();
        assert!(!set.busy);
        assert_eq!(set.devices[0].kind, DeviceKind::Removable);
        assert_eq!(
            (set.devices[0].total, set.devices[0].mount.clone()),
            (0, None)
        );
    }

    #[test]
    fn a_device_survives_the_trip_to_the_wire_and_back() {
        let device = Device {
            node: "sdb1".into(),
            disk: "sdb".into(),
            label: Some("STICK".into()),
            fs: Some("vfat".into()),
            mount: Some("/run/media/me/STICK".into()),
            total: 10,
            free: 5,
            kind: Kind::Removable,
        };
        let wire = WireDevice::new(
            &device,
            Rates {
                read: 3,
                written: 4,
            },
            vec![1, 2],
        );
        assert_eq!(wire.device(), device);
        assert_eq!(
            wire.rates(),
            Rates {
                read: 3,
                written: 4
            }
        );
    }

    #[test]
    fn a_device_with_no_node_or_a_node_already_listed_is_dropped() {
        let set = SetDevices {
            devices: vec![wire("sda1"), wire(""), wire("sda1"), wire("sdb1")],
            busy: false,
        }
        .checked();
        let nodes: Vec<_> = set.devices.iter().map(|d| d.node.as_str()).collect();
        assert_eq!(nodes, ["sda1", "sdb1"]);
    }

    #[test]
    fn a_long_history_keeps_only_its_newest_samples() {
        let mut device = wire("sda1");
        device.history = (0..(HISTORY as u64 + 5)).collect();
        let set = SetDevices {
            devices: vec![device],
            busy: false,
        }
        .checked();
        assert_eq!(set.devices[0].history.len(), HISTORY);
        assert_eq!(set.devices[0].history.last(), Some(&(HISTORY as u64 + 4)));
    }

    #[test]
    fn a_mount_path_with_an_escape_is_no_mount_and_a_goto_to_one_is_refused() {
        let mut device = wire("sda1");
        device.mount = Some("/mnt/\u{1b}[2J".into());
        let set = SetDevices {
            devices: vec![device],
            busy: false,
        }
        .checked();
        assert_eq!(set.devices[0].mount, None);
        let note = DevicesNote::Goto {
            path: "/mnt/\u{1b}x".into(),
            text: "t".into(),
        };
        assert_eq!(note.checked(), None);
    }

    #[test]
    fn a_note_is_cleaned_and_cut() {
        let note = DevicesNote::Status {
            text: format!("a\u{1b}b\u{202e}{}", "x".repeat(MAX_DEVICE_TEXT_CHARS * 2)),
        }
        .checked()
        .unwrap();
        let DevicesNote::Status { text } = note else {
            panic!("a status stays a status");
        };
        assert!(text.starts_with("abx"));
        assert_eq!(text.chars().count(), MAX_DEVICE_TEXT_CHARS);
    }

    proptest! {
        #[test]
        fn nothing_unsafe_or_over_long_or_repeated_survives_checking(
            nodes in proptest::collection::vec(".{0,20}", 0..12),
            label in ".{0,700}",
            history in proptest::collection::vec(any::<u64>(), 0..80),
        ) {
            let devices = nodes
                .iter()
                .map(|node| {
                    let mut device = wire(node);
                    device.label = Some(label.clone());
                    device.history = history.clone();
                    device
                })
                .collect();
            let set = SetDevices { devices, busy: false }.checked();
            let mut seen = std::collections::HashSet::new();
            for device in &set.devices {
                prop_assert!(!device.node.is_empty());
                prop_assert!(seen.insert(device.node.clone()));
                prop_assert!(!device.node.chars().any(is_unsafe));
                let label = device.label.as_deref().unwrap_or("");
                prop_assert!(!label.chars().any(is_unsafe));
                prop_assert!(label.chars().count() <= MAX_DEVICE_TEXT_CHARS);
                prop_assert!(device.history.len() <= HISTORY);
            }
            prop_assert_eq!(set.clone().checked(), set);
        }
    }
}
