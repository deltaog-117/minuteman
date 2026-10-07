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

//! Reads what udev has learned about a block device, from its file under `/run/udev/data`. The
//! file exists for every block device the system has looked at, mounted or not, and anyone can
//! read it, which is why a stick that is plugged in but not mounted can still be shown with its
//! label and filesystem.

/// The properties of a block device that are worth showing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    pub label: Option<String>,
    pub fs: Option<String>,
    /// What the device holds: `filesystem` for something that can be mounted, `crypto` for an
    /// encrypted container, `raid` for a member of an array, and so on.
    pub usage: Option<String>,
    /// How it is attached (`usb`, `ata`, `nvme`...).
    pub bus: Option<String>,
}

impl Info {
    /// Whether it holds a filesystem that could be mounted.
    pub fn is_filesystem(&self) -> bool {
        self.usage.as_deref() == Some("filesystem")
    }

    pub fn is_usb(&self) -> bool {
        self.bus.as_deref() == Some("usb")
    }
}

/// The `E:KEY=value` lines of a udev database file, read for the keys above.
pub fn parse(text: &str) -> Info {
    let mut info = Info::default();
    for line in text.lines() {
        let Some(pair) = line.strip_prefix("E:") else {
            continue;
        };
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = Some(value.to_string()).filter(|v| !v.is_empty());
        match key {
            "ID_FS_LABEL" => info.label = value,
            "ID_FS_TYPE" => info.fs = value,
            "ID_FS_USAGE" => info.usage = value,
            "ID_BUS" => info.bus = value,
            _ => {}
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_usb_stick_is_read_for_its_label_filesystem_and_bus() {
        let info = parse(
            "S:disk/by-label/HOLIDAY\nI:123\nE:ID_BUS=usb\nE:ID_FS_LABEL=HOLIDAY\nE:ID_FS_TYPE=vfat\nE:ID_FS_USAGE=filesystem\nG:systemd\n",
        );
        assert_eq!(info.label.as_deref(), Some("HOLIDAY"));
        assert_eq!(info.fs.as_deref(), Some("vfat"));
        assert!(info.is_filesystem());
        assert!(info.is_usb());
    }

    #[test]
    fn something_that_is_not_a_filesystem_is_not_one() {
        let info = parse("E:ID_FS_TYPE=crypto_LUKS\nE:ID_FS_USAGE=crypto\n");
        assert!(!info.is_filesystem());
        assert!(!parse("").is_filesystem());
    }

    #[test]
    fn empty_values_and_other_lines_are_ignored() {
        let info = parse("E:ID_FS_LABEL=\nX:ID_FS_LABEL=nope\nE:NOEQUALS\n");
        assert_eq!(info, Info::default());
    }
}
