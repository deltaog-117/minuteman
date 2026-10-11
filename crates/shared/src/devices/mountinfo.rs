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

//! Reads `/proc/self/mountinfo`, the kernel's list of what is mounted where, and keeps the
//! mounts worth showing: real block devices and network shares, not the dozens of virtual
//! filesystems a Linux system mounts (`proc`, `sysfs`, `cgroup`, `tmpfs`, containers' layers).

use std::collections::HashMap;
use std::path::PathBuf;

use super::Kind;

/// One line of `mountinfo` that is a mount worth showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub major: u32,
    pub minor: u32,
    /// The part of the filesystem that is mounted (`/` for the whole of it, `/@home` for a btrfs
    /// subvolume).
    pub root: String,
    pub point: PathBuf,
    pub fs: String,
    /// The device (`/dev/sdb1`) or, for a share, the remote's name (`host:/export`).
    pub source: String,
    pub kind: Kind,
}

const NETWORK_FS: [&str; 7] = [
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "fuse.sshfs",
    "davfs",
];

/// Mount points that are plumbing, not somewhere a person keeps files.
const HIDDEN_UNDER: [&str; 6] = [
    "/boot",
    "/snap",
    "/var/snap",
    "/var/lib/docker",
    "/var/lib/containers",
    "/run/user",
];

/// `\040` style escapes (a space, a tab, a newline, a backslash) back into the characters.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = (bytes[i] == b'\\' && i + 3 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 4]).ok())
            .flatten()
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        match octal {
            Some(byte) => {
                out.push(byte);
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether `point` is plumbing. A boot partition is plumbing to the deck, which lists places to
/// keep files, but part of the machine to the wiring diagram, which draws what is wired.
fn is_hidden(point: &str, include_boot: bool) -> bool {
    HIDDEN_UNDER
        .iter()
        .filter(|prefix| !(include_boot && **prefix == "/boot"))
        .any(|prefix| point == *prefix || point.starts_with(&format!("{prefix}/")))
}

fn kind_of(fs: &str, source: &str, point: &str, include_boot: bool) -> Option<Kind> {
    if is_hidden(point, include_boot) {
        return None;
    }
    if NETWORK_FS.contains(&fs) {
        return Some(Kind::Network);
    }
    let real_block =
        source.starts_with("/dev/") && !source.starts_with("/dev/loop") && fs != "squashfs";
    // Whether a block device is removable is the probe's to say, from `/sys`; here it is only
    // known to be a disk.
    real_block.then_some(Kind::Internal)
}

/// The mounts worth showing in `text`, in file order, one per device: a device mounted in several
/// places (a bind mount, a btrfs subvolume) is listed once, at the mount of its whole filesystem
/// if there is one, else the shortest path.
pub fn parse(text: &str) -> Vec<Mount> {
    parse_opts(text, false)
}

/// Like [`parse`], and with `include_boot` also the boot partition and anything under `/boot`.
pub fn parse_opts(text: &str, include_boot: bool) -> Vec<Mount> {
    let mut best: HashMap<String, usize> = HashMap::new();
    let mut mounts: Vec<Mount> = Vec::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let left: Vec<&str> = left.split_whitespace().collect();
        let right: Vec<&str> = right.split_whitespace().collect();
        let (Some(devno), Some(root), Some(point), Some(fs), Some(source)) = (
            left.get(2),
            left.get(3),
            left.get(4),
            right.first(),
            right.get(1),
        ) else {
            continue;
        };
        let Some((major, minor)) = devno.split_once(':') else {
            continue;
        };
        let (Ok(major), Ok(minor)) = (major.parse(), minor.parse()) else {
            continue;
        };
        let (point, source) = (unescape(point), unescape(source));
        let Some(kind) = kind_of(fs, &source, &point, include_boot) else {
            continue;
        };
        let mount = Mount {
            major,
            minor,
            root: unescape(root),
            point: PathBuf::from(point),
            fs: (*fs).to_string(),
            source,
            kind,
        };
        // A device is one thing however often it is mounted; a share is one thing per remote.
        let key = if mount.kind == Kind::Network {
            mount.source.clone()
        } else {
            format!("{}:{}", mount.major, mount.minor)
        };
        match best.get(&key).copied() {
            None => {
                best.insert(key, mounts.len());
                mounts.push(mount);
            }
            Some(at) => {
                let current = &mounts[at];
                let better = (
                    mount.root == "/",
                    std::cmp::Reverse(mount.point.as_os_str().len()),
                ) > (
                    current.root == "/",
                    std::cmp::Reverse(current.point.as_os_str().len()),
                );
                if better {
                    mounts[at] = mount;
                }
            }
        }
    }
    mounts
}

/// The device behind every mount in `text`, including the ones `parse` leaves out (a boot
/// partition, a snap's loop device), so a partition that is mounted somewhere unlisted is not
/// taken for one that is not mounted at all.
pub fn block_sources(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let (_, right) = line.split_once(" - ")?;
            let source = unescape(right.split_whitespace().nth(1)?);
            source.starts_with("/dev/").then_some(source)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
23 22 0:21 / /proc rw,nosuid shared:12 - proc proc rw
24 22 0:22 / /sys rw,nosuid shared:2 - sysfs sysfs rw
25 22 259:1 / /boot/efi rw shared:30 - vfat /dev/nvme0n1p1 rw
26 22 8:17 / /run/media/me/MY\\040STICK rw,nosuid shared:40 - vfat /dev/sdb1 rw
27 22 259:3 /@home /home rw shared:41 - btrfs /dev/nvme0n1p3 rw
28 22 259:3 / /mnt/pool rw shared:42 - btrfs /dev/nvme0n1p3 rw
29 22 7:0 / /snap/core/1 ro shared:50 - squashfs /dev/loop0 ro
30 22 0:50 / /mnt/nas rw shared:60 - nfs4 nas:/export rw
31 22 0:51 / /tmp rw shared:61 - tmpfs tmpfs rw
";

    #[test]
    fn only_real_disks_and_shares_are_kept() {
        let mounts = parse(SAMPLE);
        let points: Vec<&str> = mounts.iter().map(|m| m.point.to_str().unwrap()).collect();
        assert_eq!(
            points,
            ["/", "/run/media/me/MY STICK", "/mnt/pool", "/mnt/nas"],
            "no /proc, /sys, /tmp, /boot, snap loop device; the btrfs device once"
        );
    }

    #[test]
    fn a_device_mounted_twice_is_listed_at_its_whole_filesystem() {
        let mounts = parse(SAMPLE);
        let pool = mounts
            .iter()
            .find(|m| m.source == "/dev/nvme0n1p3")
            .unwrap();
        assert_eq!(pool.root, "/");
        assert_eq!(pool.point, PathBuf::from("/mnt/pool"));
    }

    #[test]
    fn escapes_in_a_mount_point_are_undone() {
        assert_eq!(unescape("MY\\040STICK"), "MY STICK");
        assert_eq!(unescape("plain"), "plain");
        assert_eq!(unescape("tab\\011x"), "tab\tx");
        assert_eq!(
            unescape("short\\04"),
            "short\\04",
            "a cut escape stays as it is"
        );
    }

    #[test]
    fn every_device_that_is_mounted_anywhere_is_named_even_if_it_is_not_listed() {
        let sources = block_sources(SAMPLE);
        assert!(
            sources.contains(&"/dev/nvme0n1p1".to_string()),
            "the hidden /boot/efi"
        );
        assert!(sources.contains(&"/dev/loop0".to_string()));
        assert!(!sources.iter().any(|s| s == "proc" || s == "nas:/export"));
    }

    #[test]
    fn the_boot_partition_is_listed_only_when_asked_for() {
        let with: Vec<String> = parse_opts(SAMPLE, true)
            .iter()
            .map(|m| m.point.display().to_string())
            .collect();
        assert!(with.contains(&"/boot/efi".to_string()), "{with:?}");
        assert!(!parse(SAMPLE).iter().any(|m| m.point.starts_with("/boot")));
        assert!(!with.iter().any(|p| p.starts_with("/snap") || p == "/tmp"));
    }

    #[test]
    fn a_share_is_a_network_mount_named_by_its_remote() {
        let nas = parse(SAMPLE).into_iter().find(|m| m.fs == "nfs4").unwrap();
        assert_eq!(nas.kind, Kind::Network);
        assert_eq!(nas.source, "nas:/export");
    }

    #[test]
    fn lines_that_are_not_mounts_are_skipped() {
        assert!(parse("").is_empty());
        assert!(parse("garbage\nmore garbage - x").is_empty());
        assert!(parse("1 2 notnumbers / /x rw - ext4 /dev/sda1 rw").is_empty());
    }
}
