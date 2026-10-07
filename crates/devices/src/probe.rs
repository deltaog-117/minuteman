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

//! Looks at the machine for its storage: what is mounted (`/proc/self/mountinfo`), what the kernel
//! and udev know about each block device (`/sys`, `/run/udev/data`), and how full each mount is
//! (`statvfs`). The places it reads are fields of `Sys`, so a test points them at a copy of the
//! files it wants and does not depend on the machine it runs on.

use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::activity::{Counters, parse_diskstats};
use crate::mountinfo::{self, Mount};
use crate::udev;
use crate::{Device, Kind};

/// How big a mounted filesystem is and how much of it an ordinary user can still fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    pub total: u64,
    pub free: u64,
}

/// Where to look. `Sys::real()` is this machine.
#[derive(Debug, Clone)]
pub struct Sys {
    /// `/dev`, where a mount's source (`/dev/mapper/root`) is a link to the real node (`dm-1`).
    pub dev_dir: PathBuf,
    /// `/proc`, holding `self/mountinfo` and `diskstats`.
    pub proc_dir: PathBuf,
    /// `/sys/class/block`, one entry per block device.
    pub class_block: PathBuf,
    /// `/sys/dev/block`, whose `major:minor` links lead to the same entries.
    pub dev_block: PathBuf,
    /// `/run/udev/data`, one `b<major>:<minor>` file per block device.
    pub udev_data: PathBuf,
    /// How full a mounted filesystem is; `None` if it cannot be asked.
    pub space: fn(&Path) -> Option<Space>,
}

impl Sys {
    pub fn real() -> Self {
        Self {
            dev_dir: "/dev".into(),
            proc_dir: "/proc".into(),
            class_block: "/sys/class/block".into(),
            dev_block: "/sys/dev/block".into(),
            udev_data: "/run/udev/data".into(),
            space: statvfs,
        }
    }

    /// Every device worth showing: the mounted ones, then the plugged-in ones that hold a
    /// filesystem but are not mounted. Internal disks first, then removable, then network, each
    /// group by name.
    pub fn scan(&self) -> Vec<Device> {
        let text = read(&self.proc_dir.join("self/mountinfo")).unwrap_or_default();
        let mounts = mountinfo::parse(&text);
        let mut devices: Vec<Device> = Vec::new();
        // What is mounted somewhere, by number and by name. Both are needed: a btrfs mount
        // reports a made-up number, and a mount that is not listed (a boot partition) has no
        // entry in `devices` to take its number from.
        let mut taken_numbers: HashSet<(u32, u32)> = HashSet::new();
        let mut taken_names: HashSet<String> = mountinfo::block_sources(&text)
            .iter()
            .map(|source| self.node_named(source))
            .collect();
        for mount in &mounts {
            taken_numbers.insert((mount.major, mount.minor));
            let device = self.mounted(mount);
            taken_names.insert(device.node.clone());
            devices.push(device);
        }
        devices.extend(self.unmounted(&taken_numbers, &taken_names));
        devices.sort_by(|a, b| {
            (a.kind, a.mount.is_none(), a.node.as_str()).cmp(&(
                b.kind,
                b.mount.is_none(),
                b.node.as_str(),
            ))
        });
        devices
    }

    /// The running byte counts of every disk, for `Activity`.
    pub fn counters(&self) -> HashMap<String, Counters> {
        parse_diskstats(&read(&self.proc_dir.join("diskstats")).unwrap_or_default())
    }

    fn mounted(&self, mount: &Mount) -> Device {
        let space = (self.space)(&mount.point);
        if mount.kind == Kind::Network {
            return Device {
                node: mount.source.clone(),
                disk: mount.source.clone(),
                label: None,
                fs: Some(mount.fs.clone()),
                mount: Some(mount.point.clone()),
                total: space.map_or(0, |s| s.total),
                free: space.map_or(0, |s| s.free),
                kind: Kind::Network,
            };
        }
        let node = self.node_named(&mount.source);
        let node = if self.class_block.join(&node).exists() {
            node
        } else {
            self.node_of(mount.major, mount.minor).unwrap_or(node)
        };
        let info = self.udev_for(&node, mount.major, mount.minor);
        let disk = self.disk_of(&node);
        let removable = self.is_removable(&disk) || info.is_usb();
        Device {
            disk,
            node,
            label: info.label,
            fs: info.fs.or_else(|| Some(mount.fs.clone())),
            mount: Some(mount.point.clone()),
            total: space.map_or(0, |s| s.total),
            free: space.map_or(0, |s| s.free),
            kind: if removable {
                Kind::Removable
            } else {
                Kind::Internal
            },
        }
    }

    /// Block devices holding a filesystem that nothing has mounted.
    fn unmounted(&self, mounted: &HashSet<(u32, u32)>, taken: &HashSet<String>) -> Vec<Device> {
        let Ok(entries) = std::fs::read_dir(&self.class_block) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let node = entry.file_name().to_string_lossy().into_owned();
            if is_virtual(&node) || taken.contains(&node) {
                continue;
            }
            let Some((major, minor)) = read(&entry.path().join("dev"))
                .and_then(|text| {
                    text.trim()
                        .split_once(':')
                        .map(|(a, b)| (a.to_string(), b.to_string()))
                })
                .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
            else {
                continue;
            };
            if mounted.contains(&(major, minor)) {
                continue;
            }
            let info = self.udev_of(major, minor);
            if !info.is_filesystem() {
                continue;
            }
            let sectors: u64 = read(&entry.path().join("size"))
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(0);
            let disk = self.disk_of(&node);
            let removable = self.is_removable(&disk) || info.is_usb();
            found.push(Device {
                disk,
                node,
                label: info.label,
                fs: info.fs,
                mount: None,
                total: sectors.saturating_mul(512),
                free: 0,
                kind: if removable {
                    Kind::Removable
                } else {
                    Kind::Internal
                },
            });
        }
        found
    }

    /// The kernel's name for a mount's source: `/dev/mapper/root` is a link to `dm-1`, and
    /// `/dev/sdb1` is itself. A source that is not under `/dev`, or not there, keeps the last part
    /// of its name.
    fn node_named(&self, source: &str) -> String {
        source
            .strip_prefix("/dev/")
            .and_then(|rest| std::fs::canonicalize(self.dev_dir.join(rest)).ok())
            .and_then(|real| Some(real.file_name()?.to_string_lossy().into_owned()))
            .unwrap_or_else(|| basename(source))
    }

    /// udev's data for a device, found through the device's own number when the kernel knows it,
    /// since a btrfs mount's number leads nowhere, and through the mount's number otherwise.
    fn udev_for(&self, node: &str, major: u32, minor: u32) -> udev::Info {
        let numbers = read(&self.class_block.join(node).join("dev")).and_then(|text| {
            let (a, b) = text.trim().split_once(':')?;
            Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))
        });
        let (major, minor) = numbers.unwrap_or((major, minor));
        self.udev_of(major, minor)
    }

    /// The kernel's name for a device number, from the link `/sys/dev/block/<major>:<minor>`.
    fn node_of(&self, major: u32, minor: u32) -> Option<String> {
        let link = std::fs::read_link(self.dev_block.join(format!("{major}:{minor}"))).ok()?;
        Some(link.file_name()?.to_string_lossy().into_owned())
    }

    fn udev_of(&self, major: u32, minor: u32) -> udev::Info {
        read(&self.udev_data.join(format!("b{major}:{minor}")))
            .map(|text| udev::parse(&text))
            .unwrap_or_default()
    }

    /// The whole disk a partition belongs to (`sdb` for `sdb1`), or the device itself if it is one.
    fn disk_of(&self, node: &str) -> String {
        let here = self.class_block.join(node);
        if !here.join("partition").exists() {
            return node.to_string();
        }
        std::fs::canonicalize(&here)
            .ok()
            .and_then(|real| Some(real.parent()?.file_name()?.to_string_lossy().into_owned()))
            .unwrap_or_else(|| node.to_string())
    }

    fn is_removable(&self, disk: &str) -> bool {
        read(&self.class_block.join(disk).join("removable")).is_some_and(|t| t.trim() == "1")
    }
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn basename(source: &str) -> String {
    source.rsplit('/').next().unwrap_or(source).to_string()
}

/// Devices that are never a place to keep files: RAM disks, loop devices, and the like.
fn is_virtual(node: &str) -> bool {
    ["loop", "ram", "zram", "nbd", "sr", "fd"]
        .iter()
        .any(|prefix| node.starts_with(prefix))
}

/// `statvfs` on `path`: the size, and the space left for an ordinary user (not the part reserved
/// for root).
pub fn statvfs(path: &Path) -> Option<Space> {
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c_path` is a valid NUL-terminated string that outlives the call, and `stat` is a
    // writable `statvfs` the call fills in completely when it returns 0.
    let stat = unsafe {
        if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return None;
        }
        stat.assume_init()
    };
    // These fields are `u64` on a 64-bit machine and `u32` on a 32-bit one; the conversions are
    // needed there, and are the same type here.
    #[allow(clippy::useless_conversion)]
    let (unit, blocks, available) = (
        u64::from(stat.f_frsize),
        u64::from(stat.f_blocks),
        u64::from(stat.f_bavail),
    );
    Some(Space {
        total: blocks.saturating_mul(unit),
        free: available.saturating_mul(unit),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn fake_space(path: &Path) -> Option<Space> {
        (path.to_str()? != "/gone").then_some(Space {
            total: 1000,
            free: 400,
        })
    }

    /// A machine with a system disk (`nvme0n1p2`, mounted), a USB stick (`sdb1`, mounted), a
    /// second stick (`sdc1`, plugged in but not mounted) and a share.
    fn machine(label: &str) -> (PathBuf, Sys) {
        let root =
            std::env::temp_dir().join(format!("minuteman-devices-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let class_block = root.join("sys/class/block");
        let devices = root.join("sys/devices");
        let dev_block = root.join("sys/dev/block");
        let udev_data = root.join("udev");
        let proc_dir = root.join("proc");
        for dir in [
            &class_block,
            &devices,
            &dev_block,
            &udev_data,
            &proc_dir.join("self"),
        ] {
            fs::create_dir_all(dir).unwrap();
        }
        // (disk, partition, major:minor, removable, size in sectors, udev data)
        let nodes = [
            (
                "nvme0n1",
                "nvme0n1p2",
                "259:2",
                "0",
                "2000",
                "E:ID_FS_TYPE=ext4\nE:ID_FS_USAGE=filesystem\n",
            ),
            (
                "sdb",
                "sdb1",
                "8:17",
                "1",
                "4000",
                "E:ID_BUS=usb\nE:ID_FS_LABEL=HOLIDAY\nE:ID_FS_TYPE=vfat\nE:ID_FS_USAGE=filesystem\n",
            ),
            (
                "sdc",
                "sdc1",
                "8:33",
                "1",
                "8000",
                "E:ID_BUS=usb\nE:ID_FS_LABEL=BACKUP\nE:ID_FS_TYPE=exfat\nE:ID_FS_USAGE=filesystem\n",
            ),
            // Mounted at /boot/efi, which is not listed, so it must not show as unmounted.
            (
                "sdd",
                "sdd1",
                "8:34",
                "0",
                "2000",
                "E:ID_FS_TYPE=vfat\nE:ID_FS_USAGE=filesystem\n",
            ),
        ];
        for (disk, part, devno, removable, sectors, data) in nodes {
            let disk_dir = devices.join(disk);
            let part_dir = disk_dir.join(part);
            fs::create_dir_all(&part_dir).unwrap();
            fs::write(disk_dir.join("removable"), removable).unwrap();
            fs::write(part_dir.join("partition"), "2").unwrap();
            fs::write(part_dir.join("dev"), format!("{devno}\n")).unwrap();
            fs::write(part_dir.join("size"), sectors).unwrap();
            symlink(&disk_dir, class_block.join(disk)).unwrap();
            symlink(&part_dir, class_block.join(part)).unwrap();
            symlink(&part_dir, dev_block.join(devno)).unwrap();
            fs::write(udev_data.join(format!("b{devno}")), data).unwrap();
        }
        // A mapped device mounted by a made-up number (as btrfs does), reached through a link in
        // /dev; it is mounted, so it must show once, as mounted.
        let vault = devices.join("dm-1");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("dev"), "254:1\n").unwrap();
        fs::write(vault.join("size"), "6000").unwrap();
        symlink(&vault, class_block.join("dm-1")).unwrap();
        symlink(&vault, dev_block.join("254:1")).unwrap();
        fs::write(
            udev_data.join("b254:1"),
            "E:ID_FS_LABEL=vault\nE:ID_FS_TYPE=btrfs\nE:ID_FS_USAGE=filesystem\n",
        )
        .unwrap();
        let dev_dir = root.join("dev");
        fs::create_dir_all(dev_dir.join("mapper")).unwrap();
        symlink(&vault, dev_dir.join("dm-1")).unwrap();
        symlink("../dm-1", dev_dir.join("mapper/vault")).unwrap();
        // A loop device and a whole disk with no filesystem must not appear.
        let loop0 = devices.join("loop0");
        fs::create_dir_all(&loop0).unwrap();
        fs::write(loop0.join("dev"), "7:0\n").unwrap();
        symlink(&loop0, class_block.join("loop0")).unwrap();
        fs::write(
            udev_data.join("b7:0"),
            "E:ID_FS_TYPE=squashfs\nE:ID_FS_USAGE=filesystem\n",
        )
        .unwrap();

        fs::write(
            proc_dir.join("self/mountinfo"),
            "22 1 259:2 / / rw - ext4 /dev/nvme0n1p2 rw\n\
             23 22 0:21 / /proc rw - proc proc rw\n\
             25 22 8:34 / /boot/efi rw - vfat /dev/sdd1 rw\n\
             26 22 8:17 / /run/media/me/HOLIDAY rw - vfat /dev/sdb1 rw\n\
             27 22 0:99 / /mnt/vault rw - btrfs /dev/mapper/vault rw\n\
             30 22 0:50 / /mnt/nas rw - nfs4 nas:/export rw\n",
        )
        .unwrap();
        fs::write(
            proc_dir.join("diskstats"),
            "259 2 nvme0n1p2 1 0 10 0 1 0 20 0 0 0 0\n 8 17 sdb1 1 0 30 0 1 0 40 0 0 0 0\n",
        )
        .unwrap();
        let sys = Sys {
            dev_dir,
            proc_dir,
            class_block,
            dev_block,
            udev_data,
            space: fake_space,
        };
        (root, sys)
    }

    #[test]
    fn the_machine_is_scanned_into_internal_removable_and_network_devices() {
        let (root, sys) = machine("scan");
        let devices = sys.scan();
        let summary: Vec<(String, Kind, bool)> = devices
            .iter()
            .map(|d| (d.node.clone(), d.kind, d.is_mounted()))
            .collect();
        assert_eq!(
            summary,
            [
                ("dm-1".to_string(), Kind::Internal, true),
                ("nvme0n1p2".to_string(), Kind::Internal, true),
                ("sdb1".to_string(), Kind::Removable, true),
                ("sdc1".to_string(), Kind::Removable, false),
                ("nas:/export".to_string(), Kind::Network, true),
            ],
            "no /proc, no loop device, no /boot/efi twice; the mapped vault once, as mounted; the \
             unmounted stick after the mounted one"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_mounted_device_carries_its_label_disk_and_space() {
        let (root, sys) = machine("mounted");
        let devices = sys.scan();
        let stick = devices.iter().find(|d| d.node == "sdb1").unwrap();
        assert_eq!(stick.label.as_deref(), Some("HOLIDAY"));
        assert_eq!(stick.disk, "sdb");
        assert_eq!(stick.fs.as_deref(), Some("vfat"));
        assert_eq!((stick.total, stick.free), (1000, 400));
        assert!(stick.can_eject());
        let system = devices.iter().find(|d| d.node == "nvme0n1p2").unwrap();
        assert_eq!(system.disk, "nvme0n1");
        assert!(!system.can_eject());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_unmounted_device_is_sized_from_the_kernel_and_has_no_free_space() {
        let (root, sys) = machine("unmounted");
        let devices = sys.scan();
        let backup = devices.iter().find(|d| d.node == "sdc1").unwrap();
        assert_eq!(backup.label.as_deref(), Some("BACKUP"));
        assert_eq!(backup.mount, None);
        assert_eq!(backup.total, 8000 * 512);
        assert_eq!(backup.free, 0);
        assert!(backup.can_eject());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn counters_come_from_diskstats() {
        let (root, sys) = machine("counters");
        let counters = sys.counters();
        assert_eq!(counters["sdb1"].read, 30 * 512);
        assert_eq!(counters["sdb1"].written, 40 * 512);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_machine_with_nothing_to_read_has_no_devices() {
        let nowhere = PathBuf::from("/nonexistent/minuteman");
        let sys = Sys {
            dev_dir: nowhere.clone(),
            proc_dir: nowhere.clone(),
            class_block: nowhere.clone(),
            dev_block: nowhere.clone(),
            udev_data: nowhere,
            space: fake_space,
        };
        assert!(sys.scan().is_empty());
        assert!(sys.counters().is_empty());
    }

    #[test]
    fn the_real_root_has_a_size() {
        let space = statvfs(Path::new("/")).expect("the root can be asked");
        assert!(space.total > 0 && space.free <= space.total);
        assert_eq!(statvfs(Path::new("/nonexistent/minuteman")), None);
    }

    #[test]
    fn virtual_devices_are_recognised() {
        for node in ["loop3", "ram0", "zram0", "sr0", "nbd0"] {
            assert!(is_virtual(node), "{node}");
        }
        for node in ["sda1", "nvme0n1p1", "mmcblk0p1", "dm-0"] {
            assert!(!is_virtual(node), "{node}");
        }
    }
}
