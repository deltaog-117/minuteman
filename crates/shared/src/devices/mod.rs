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

//! What storage this machine has: internal disks, removable sticks, network mounts, mounted or
//! not, with their size, what is on them and how busy they are, and the means to mount, unmount
//! and eject them. Linux only, and nothing here draws anything: the parsers take text and are
//! tested on it, `probe` reads the real `/proc` and `/sys` (or a copy of them, for tests), and
//! `udisks` runs the `udisksctl` program, which changes a mount without needing root.
//!
//! Everything is built to be called off the interface thread. A `statvfs` on a network mount that
//! has gone away can block for a long time, so nothing here may be called from a render loop.

pub mod activity;
pub mod mountinfo;
pub mod plan;
pub mod probe;
pub mod topology;
pub mod udev;
pub mod udisks;

use std::path::PathBuf;

/// Where a device is attached, which decides how it is grouped and what can be done to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A disk built into the machine.
    Internal,
    /// A USB stick or card, or anything else the kernel marks removable.
    Removable,
    /// A share on another machine.
    Network,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// What the kernel calls it (`sdb1`, `nvme0n1p2`), or the remote's own name for a network
    /// mount (`host:/export`). Unique among the devices listed.
    pub node: String,
    /// The whole disk this is part of (`sdb` for `sdb1`), which is what gets powered off to eject.
    pub disk: String,
    /// The volume label, when the filesystem has one.
    pub label: Option<String>,
    pub fs: Option<String>,
    /// Where it is mounted; `None` for a device that is plugged in but not mounted.
    pub mount: Option<PathBuf>,
    /// Size in bytes, and how much of it is free to an ordinary user. Zero when unknown.
    pub total: u64,
    pub free: u64,
    pub kind: Kind,
}

impl Device {
    /// The name to show: the label, else the folder it is mounted on, else the kernel's name.
    pub fn title(&self) -> String {
        if let Some(label) = self.label.as_deref().filter(|l| !l.is_empty()) {
            return label.to_string();
        }
        match self.mount.as_deref() {
            Some(path) if path == std::path::Path::new("/") => "System".to_string(),
            Some(path) => path
                .file_name()
                .map_or_else(|| self.node.clone(), |n| n.to_string_lossy().into_owned()),
            None => self.node.clone(),
        }
    }

    pub fn is_mounted(&self) -> bool {
        self.mount.is_some()
    }

    /// How much of the device is used, as a whole percent, or `None` when its size is unknown.
    pub fn used_percent(&self) -> Option<u8> {
        if self.total == 0 {
            return None;
        }
        let used = self.total.saturating_sub(self.free);
        // At most 100 by construction, so the narrowing cannot lose anything.
        Some(u8::try_from(u128::from(used) * 100 / u128::from(self.total)).unwrap_or(100))
    }

    /// Whether it can be ejected: a removable device that is a real disk, not a network share.
    pub fn can_eject(&self) -> bool {
        self.kind == Kind::Removable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Device {
        Device {
            node: "sdb1".into(),
            disk: "sdb".into(),
            label: None,
            fs: Some("vfat".into()),
            mount: None,
            total: 1000,
            free: 250,
            kind: Kind::Removable,
        }
    }

    #[test]
    fn the_title_prefers_the_label_then_the_mount_folder_then_the_kernel_name() {
        let mut d = device();
        assert_eq!(d.title(), "sdb1");
        d.mount = Some("/run/media/me/STICK".into());
        assert_eq!(d.title(), "STICK");
        d.label = Some("Holiday".into());
        assert_eq!(d.title(), "Holiday");
        d.label = Some(String::new());
        assert_eq!(d.title(), "STICK", "an empty label is no label");
        d.mount = Some("/".into());
        d.label = None;
        assert_eq!(d.title(), "System");
    }

    #[test]
    fn usage_is_a_whole_percent_and_unknown_without_a_size() {
        let mut d = device();
        assert_eq!(d.used_percent(), Some(75));
        d.free = 0;
        assert_eq!(d.used_percent(), Some(100));
        d.free = 5000;
        assert_eq!(
            d.used_percent(),
            Some(0),
            "free beyond total never goes negative"
        );
        d.total = 0;
        assert_eq!(d.used_percent(), None);
    }

    #[test]
    fn only_a_removable_device_can_be_ejected() {
        let mut d = device();
        assert!(d.can_eject());
        d.kind = Kind::Network;
        assert!(!d.can_eject());
        d.kind = Kind::Internal;
        assert!(!d.can_eject());
    }
}
