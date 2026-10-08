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

//! How the machine's storage is wired: the bus a disk hangs off, the disk, its partitions, and
//! whatever is stacked on those (an LVM volume, an encrypted container, a RAID array), down to the
//! filesystems and where they are mounted. Read from the same places as the flat device list, plus
//! the kernel's `slaves` links that say what each stacked device is made of.
//!
//! The result is a graph, not a tree: a RAID array has several parents. Each node still has one
//! primary parent (its first), which is the one a drawing hangs it under; the others are listed so
//! the drawing can say so.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::probe::{Space, Sys, is_virtual, read};
use crate::{Device, Kind, mountinfo, udev};

/// What a stacked device is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Lvm,
    Crypt,
    Raid,
    Multipath,
    Other,
}

impl Layer {
    pub fn label(self) -> &'static str {
        match self {
            Layer::Lvm => "LVM",
            Layer::Crypt => "LUKS",
            Layer::Raid => "RAID",
            Layer::Multipath => "multipath",
            Layer::Other => "mapped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// The way a disk is attached: a PCIe slot, a USB port, a SATA link.
    Bus,
    Disk,
    Partition,
    Layer(Layer),
    /// A network share.
    Share,
    /// A folder inside a mounted filesystem, which a drawing can unfold. Not part of the
    /// hardware: whoever draws the folders adds these.
    Folder,
    /// "And 37 more": folders there was no room to show, or a listing still being read.
    More,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The kernel's name (`nvme0n1p2`, `dm-1`), the bus's label, or the share's remote name.
    pub id: String,
    pub kind: NodeKind,
    /// A device-mapper name (`ArchinstallVg-root`), for a stacked device.
    pub name: Option<String>,
    /// The make and model, for a disk.
    pub model: Option<String>,
    pub label: Option<String>,
    pub fs: Option<String>,
    /// What udev says it holds: `filesystem`, `crypto`, `raid`...
    pub usage: Option<String>,
    pub mount: Option<PathBuf>,
    /// For a `Folder`, where it is; for a `More`, the folder it belongs to.
    pub path: Option<PathBuf>,
    pub total: u64,
    pub free: u64,
    pub removable: bool,
    /// The nodes it is made from or attached to; the first is where a drawing hangs it.
    pub parents: Vec<usize>,
    pub children: Vec<usize>,
}

impl Node {
    /// A node with nothing known about it but what it is called and what kind it is.
    pub fn new(id: &str, kind: NodeKind) -> Self {
        Self {
            id: id.to_string(),
            kind,
            name: None,
            model: None,
            label: None,
            fs: None,
            usage: None,
            mount: None,
            path: None,
            total: 0,
            free: 0,
            removable: false,
            parents: Vec::new(),
            children: Vec::new(),
        }
    }

    /// The name to show: the label, the mapped name, the model for a disk, else the kernel's.
    pub fn title(&self) -> String {
        let named = match self.kind {
            NodeKind::Disk => self.model.as_deref(),
            _ => self.label.as_deref().or(self.name.as_deref()),
        };
        named
            .filter(|n| !n.is_empty())
            .map_or_else(|| self.id.clone(), str::to_string)
    }

    /// The folder this node stands for, which can be unfolded into its subfolders: where a mounted
    /// filesystem is mounted, or where a `Folder` is. `None` for everything else.
    pub fn folder_path(&self) -> Option<&Path> {
        match self.kind {
            NodeKind::Folder => self.path.as_deref(),
            NodeKind::Bus | NodeKind::More => None,
            _ if self.is_filesystem() => self.mount.as_deref(),
            _ => None,
        }
    }

    /// Whether it holds a filesystem that can be mounted, or is a share that already is.
    pub fn is_filesystem(&self) -> bool {
        self.kind == NodeKind::Share || self.usage.as_deref() == Some("filesystem")
    }
}

/// Everything found, as nodes that point at each other by index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Topology {
    pub nodes: Vec<Node>,
}

impl Topology {
    /// Adds a node and returns its index.
    pub fn add(&mut self, node: Node) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// Records that `child` is under (made from, or attached to) `parent`.
    pub fn link(&mut self, parent: usize, child: usize) {
        if !self.nodes[parent].children.contains(&child) {
            self.nodes[parent].children.push(child);
            self.nodes[child].parents.push(parent);
        }
    }

    /// The nodes nothing is under: buses, and disks or shares with none. In the order a drawing
    /// lists them.
    pub fn roots(&self) -> Vec<usize> {
        let mut roots: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| self.nodes[i].parents.is_empty())
            .collect();
        let rank = |kind: NodeKind| match kind {
            NodeKind::Bus => 0,
            NodeKind::Share => 2,
            _ => 1,
        };
        roots.sort_by(|&a, &b| {
            (rank(self.nodes[a].kind), &self.nodes[a].id)
                .cmp(&(rank(self.nodes[b].kind), &self.nodes[b].id))
        });
        roots
    }

    /// How many steps each node is from a root along its longest chain, which is the column a
    /// drawing puts it in. A loop in the data (which sysfs should never give) stops being followed.
    pub fn depths(&self) -> Vec<usize> {
        let mut depths = vec![0usize; self.nodes.len()];
        for root in self.roots() {
            let mut seen = HashSet::new();
            self.deepen(root, 0, &mut depths, &mut seen);
        }
        depths
    }

    fn deepen(&self, at: usize, depth: usize, depths: &mut [usize], path: &mut HashSet<usize>) {
        if !path.insert(at) {
            return;
        }
        depths[at] = depths[at].max(depth);
        for &child in &self.nodes[at].children {
            self.deepen(child, depth + 1, depths, path);
        }
        path.remove(&at);
    }

    /// Every node once, depth first from the roots under each one's primary parent, children
    /// in name order: the order a drawing lists rows in.
    pub fn order(&self) -> Vec<usize> {
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut placed = HashSet::new();
        for root in self.roots() {
            self.walk(root, &mut order, &mut placed);
        }
        order
    }

    fn walk(&self, at: usize, order: &mut Vec<usize>, placed: &mut HashSet<usize>) {
        if !placed.insert(at) {
            return;
        }
        order.push(at);
        for &child in &self.nodes[at].children {
            // Under its first parent only; the others reach it by a stub.
            if self.nodes[child].parents.first() == Some(&at) {
                self.walk(child, order, placed);
            }
        }
    }

    /// The whole disk a node sits on, if it can be traced to one.
    pub fn disk_of(&self, mut at: usize) -> Option<usize> {
        let mut steps = 0;
        loop {
            if self.nodes[at].kind == NodeKind::Disk {
                return Some(at);
            }
            at = *self.nodes[at].parents.first()?;
            steps += 1;
            if steps > self.nodes.len() {
                return None;
            }
        }
    }

    /// The node as a device the deck's mount, eject and fit check understand, for a node that
    /// holds a filesystem (or is a share).
    pub fn device(&self, at: usize) -> Option<Device> {
        let node = &self.nodes[at];
        if !node.is_filesystem() {
            return None;
        }
        let disk = self
            .disk_of(at)
            .map_or_else(|| node.id.clone(), |d| self.nodes[d].id.clone());
        Some(Device {
            node: node.id.clone(),
            disk,
            label: node.label.clone(),
            fs: node.fs.clone(),
            mount: node.mount.clone(),
            total: node.total,
            free: node.free,
            kind: if node.kind == NodeKind::Share {
                Kind::Network
            } else if node.removable {
                Kind::Removable
            } else {
                Kind::Internal
            },
        })
    }

    /// The node whose mount holds `dir`, preferring the deepest mount.
    pub fn holding(&self, dir: &Path) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let mount = n.mount.as_deref()?;
                dir.starts_with(mount)
                    .then_some((mount.as_os_str().len(), i))
            })
            .max()
            .map(|(_, i)| i)
    }
}

/// How a disk is attached, from its path under `/sys/devices`: `PCIe 02:00.0` for an NVMe drive,
/// `USB 2-3` for a stick on a port (or `USB 1-2.4` behind a hub), `SATA ata1`. `None` for a
/// virtual device, or a bus not recognised.
pub fn bus_of(sys_path: &Path) -> Option<String> {
    let parts: Vec<&str> = sys_path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    let after = parts.iter().rposition(|p| *p == "devices")? + 1;
    let parts = &parts[after..];
    let is_pci = |s: &str| {
        let b = s.as_bytes();
        s.len() == 12 && b[4] == b':' && b[7] == b':' && b[10] == b'.'
    };
    // The last PCI address before an `nvme` segment is the slot the drive sits in.
    if let Some(nvme) = parts.iter().position(|p| *p == "nvme") {
        let slot = parts[..nvme].iter().rfind(|p| is_pci(p))?;
        return Some(format!("PCIe {}", &slot[5..]));
    }
    let is_port = |s: &str| {
        let mut halves = s.splitn(2, '-');
        let (Some(bus), Some(ports)) = (halves.next(), halves.next()) else {
            return false;
        };
        !bus.is_empty()
            && bus.bytes().all(|b| b.is_ascii_digit())
            && !ports.is_empty()
            && ports
                .split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    if parts
        .iter()
        .any(|p| p.starts_with("usb") && p[3..].bytes().all(|b| b.is_ascii_digit()))
    {
        // The last bare port segment, before the interface (`2-3:1.0`) is the device itself.
        let port = parts.iter().rfind(|p| is_port(p))?;
        return Some(format!("USB {port}"));
    }
    for (prefix, label) in [("ata", "SATA"), ("mmc", "MMC"), ("virtio", "virtio")] {
        if let Some(seg) = parts.iter().find(|p| {
            p.starts_with(prefix)
                && p.len() > prefix.len()
                && p[prefix.len()..].bytes().all(|b| b.is_ascii_digit())
        }) {
            return Some(if prefix == "virtio" {
                (*seg).to_string()
            } else {
                format!("{label} {seg}")
            });
        }
    }
    None
}

fn layer_of(uuid: &str) -> Layer {
    if uuid.starts_with("CRYPT") {
        Layer::Crypt
    } else if uuid.starts_with("LVM") {
        Layer::Lvm
    } else if uuid.starts_with("mpath") {
        Layer::Multipath
    } else {
        Layer::Other
    }
}

impl Sys {
    /// The machine's storage as a graph: buses, disks, partitions, stacked devices and shares,
    /// with what is mounted where. Like `scan`, it reads files and `statvfs`, so it belongs off
    /// the interface thread.
    pub fn topology(&self) -> Topology {
        let mut names: Vec<String> = std::fs::read_dir(&self.class_block)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !is_virtual(n))
            .collect();
        names.sort();

        let mut nodes: Vec<Node> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for name in &names {
            let here = self.class_block.join(name);
            let kind = if here.join("partition").exists() {
                NodeKind::Partition
            } else if here.join("dm").exists() {
                let uuid = read(&here.join("dm/uuid")).unwrap_or_default();
                NodeKind::Layer(layer_of(uuid.trim()))
            } else if here.join("md").exists() {
                NodeKind::Layer(Layer::Raid)
            } else {
                NodeKind::Disk
            };
            let mut node = Node::new(name, kind);
            node.name = read(&here.join("dm/name"))
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            node.model = read(&here.join("device/model"))
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            node.removable = read(&here.join("removable")).is_some_and(|t| t.trim() == "1");
            node.total = read(&here.join("size"))
                .and_then(|t| t.trim().parse::<u64>().ok())
                .unwrap_or(0)
                .saturating_mul(512);
            let numbers = read(&here.join("dev")).and_then(|t| {
                let (a, b) = t.trim().split_once(':')?;
                Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))
            });
            if let Some((major, minor)) = numbers {
                let info: udev::Info = self.udev_of(major, minor);
                node.label = info.label;
                node.fs = info.fs;
                node.usage = info.usage;
            }
            index.insert(name.clone(), nodes.len());
            nodes.push(node);
        }

        // What is under what: a partition under its disk, a stacked device under what it is
        // made of.
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for name in &names {
            let here = self.class_block.join(name);
            let child = index[name];
            if nodes[child].kind == NodeKind::Partition
                && let Some(parent) = std::fs::canonicalize(&here)
                    .ok()
                    .and_then(|real| {
                        Some(real.parent()?.file_name()?.to_string_lossy().into_owned())
                    })
                    .and_then(|disk| index.get(&disk).copied())
            {
                edges.push((parent, child));
            }
            let mut slaves: Vec<String> = std::fs::read_dir(here.join("slaves"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            slaves.sort();
            for slave in slaves {
                if let Some(&parent) = index.get(&slave) {
                    edges.push((parent, child));
                }
            }
        }

        // What is mounted where.
        let text = read(&self.proc_dir.join("self/mountinfo")).unwrap_or_default();
        for mount in mountinfo::parse_opts(&text, true) {
            let space: Option<Space> = (self.space)(&mount.point);
            if mount.kind == Kind::Network {
                let mut share = Node::new(&mount.source, NodeKind::Share);
                share.fs = Some(mount.fs.clone());
                share.mount = Some(mount.point.clone());
                share.total = space.map_or(0, |s| s.total);
                share.free = space.map_or(0, |s| s.free);
                index.insert(mount.source.clone(), nodes.len());
                nodes.push(share);
                continue;
            }
            let named = self.node_named(&mount.source);
            let found = index.get(&named).copied().or_else(|| {
                self.node_of(mount.major, mount.minor)
                    .and_then(|n| index.get(&n).copied())
            });
            if let Some(at) = found {
                let node = &mut nodes[at];
                node.mount = Some(mount.point.clone());
                node.fs.get_or_insert_with(|| mount.fs.clone());
                if let Some(space) = space {
                    node.total = space.total;
                    node.free = space.free;
                }
            }
        }

        // The bus a disk with nothing under it hangs off.
        let mut buses: HashMap<String, usize> = HashMap::new();
        for name in &names {
            let at = index[name];
            if nodes[at].kind != NodeKind::Disk {
                continue;
            }
            let Some(label) = std::fs::canonicalize(self.class_block.join(name))
                .ok()
                .and_then(|real| bus_of(&real))
            else {
                continue;
            };
            let bus = *buses.entry(label.clone()).or_insert_with(|| {
                nodes.push(Node::new(&label, NodeKind::Bus));
                nodes.len() - 1
            });
            if label.starts_with("USB") || label.starts_with("MMC") {
                nodes[at].removable = true;
            }
            edges.push((bus, at));
        }

        edges.sort_unstable();
        edges.dedup();
        for (parent, child) in edges {
            nodes[parent].children.push(child);
            nodes[child].parents.push(parent);
        }
        for node in &mut nodes {
            node.children.sort();
        }

        let mut topology = Topology { nodes };
        // Removable is a property of the disk; everything on it shares it.
        for at in topology.order() {
            let inherited = topology.nodes[at]
                .parents
                .iter()
                .any(|&p| topology.nodes[p].removable);
            if inherited {
                topology.nodes[at].removable = true;
            }
        }
        // A card reader with no card, or an empty disk, is not worth a box.
        let empty = |t: &Topology, i: usize| {
            let n = &t.nodes[i];
            n.kind == NodeKind::Disk && n.total == 0 && n.children.is_empty() && n.mount.is_none()
        };
        let mut keep: Vec<bool> = (0..topology.nodes.len())
            .map(|i| !empty(&topology, i))
            .collect();
        // A bus whose disks were all dropped has nothing left to attach.
        for i in 0..keep.len() {
            let node = &topology.nodes[i];
            if node.kind == NodeKind::Bus && node.children.iter().all(|&c| !keep[c]) {
                keep[i] = false;
            }
        }
        topology.retain(&keep);
        topology
    }
}

impl Topology {
    /// Drops the nodes `keep` says no to and renumbers the rest.
    fn retain(&mut self, keep: &[bool]) {
        let mut renumber = vec![usize::MAX; self.nodes.len()];
        let mut next = 0;
        for (i, kept) in keep.iter().enumerate() {
            if *kept {
                renumber[i] = next;
                next += 1;
            }
        }
        let old = std::mem::take(&mut self.nodes);
        for (i, mut node) in old.into_iter().enumerate() {
            if !keep[i] {
                continue;
            }
            node.parents = node
                .parents
                .iter()
                .filter(|&&p| keep[p])
                .map(|&p| renumber[p])
                .collect();
            node.children = node
                .children
                .iter()
                .filter(|&&c| keep[c])
                .map(|&c| renumber[c])
                .collect();
            self.nodes.push(node);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn a_bus_is_read_from_where_the_disk_sits_under_sys_devices() {
        let at = |p: &str| bus_of(Path::new(p));
        assert_eq!(
            at("/sys/devices/pci0000:00/0000:00:01.2/0000:02:00.0/nvme/nvme0/nvme0n1").as_deref(),
            Some("PCIe 02:00.0")
        );
        assert_eq!(
            at("/sys/devices/pci0000:00/0000:00:14.0/usb2/2-3/2-3:1.0/host6/target6:0:0/6:0:0:0/block/sdb")
                .as_deref(),
            Some("USB 2-3")
        );
        assert_eq!(
            at("/sys/devices/pci0000:00/0000:00:14.0/usb1/1-2/1-2.4/1-2.4:1.0/host7/target7:0:0/7:0:0:0/block/sdc")
                .as_deref(),
            Some("USB 1-2.4"),
            "behind a hub"
        );
        assert_eq!(
            at("/sys/devices/pci0000:00/0000:00:17.0/ata1/host0/target0:0:0/0:0:0:0/block/sda")
                .as_deref(),
            Some("SATA ata1")
        );
        assert_eq!(
            at("/sys/devices/platform/soc/mmc_host/mmc0/mmc0:0001/block/mmcblk0").as_deref(),
            Some("MMC mmc0")
        );
        assert_eq!(
            at("/sys/devices/pci0000:00/0000:00:05.0/virtio2/block/vda").as_deref(),
            Some("virtio2")
        );
        assert_eq!(at("/sys/devices/virtual/block/dm-0"), None);
        assert_eq!(at("/no/devices/dir"), None);
        assert_eq!(at("/sys/devices"), None);
    }

    #[test]
    fn a_stacked_device_is_classified_by_what_device_mapper_calls_it() {
        assert_eq!(layer_of("CRYPT-LUKS2-abc-root"), Layer::Crypt);
        assert_eq!(layer_of("LVM-abcdef"), Layer::Lvm);
        assert_eq!(layer_of("mpath-3600508b4"), Layer::Multipath);
        assert_eq!(layer_of(""), Layer::Other);
        assert_eq!(Layer::Crypt.label(), "LUKS");
    }

    /// Builds a directory that looks like `/sys/class/block`, `/sys/devices`, `/run/udev/data`,
    /// `/proc` and `/dev` for the machine this was written on (an NVMe drive holding a boot
    /// partition and an LVM physical volume, with an encrypted container and then a btrfs
    /// filesystem stacked on it), a USB stick, and a RAID array of two partitions.
    struct Machine {
        root: PathBuf,
        sys: Sys,
    }

    impl Machine {
        fn node(&self, bus_path: &str, name: &str, dev: &str, sectors: u64, udev: &str) -> PathBuf {
            let dir = self.root.join("sys/devices").join(bus_path).join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("dev"), format!("{dev}\n")).unwrap();
            fs::write(dir.join("size"), sectors.to_string()).unwrap();
            symlink(&dir, self.sys.class_block.join(name)).unwrap();
            symlink(&dir, self.sys.dev_block.join(dev)).unwrap();
            if !udev.is_empty() {
                fs::write(self.sys.udev_data.join(format!("b{dev}")), udev).unwrap();
            }
            dir
        }

        fn new(label: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("minuteman-topology-{label}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let sys = Sys {
                dev_dir: root.join("dev"),
                proc_dir: root.join("proc"),
                class_block: root.join("sys/class/block"),
                dev_block: root.join("sys/dev/block"),
                udev_data: root.join("udev"),
                space: |path| {
                    Some(Space {
                        total: 1000,
                        free: if path == Path::new("/") { 60 } else { 500 },
                    })
                },
            };
            for dir in [
                &sys.class_block,
                &sys.dev_block,
                &sys.udev_data,
                &sys.proc_dir.join("self"),
                &sys.dev_dir.join("mapper"),
            ] {
                fs::create_dir_all(dir).unwrap();
            }
            let machine = Machine { root, sys };

            let nvme = "pci0000:00/0000:00:01.2/0000:02:00.0/nvme/nvme0";
            let disk = machine.node(nvme, "nvme0n1", "259:0", 8000, "");
            fs::create_dir_all(disk.join("device")).unwrap();
            fs::write(disk.join("device/model"), "Samsung SSD 980  \n").unwrap();
            fs::write(disk.join("removable"), "0").unwrap();
            for (part, dev, sectors, udev_data) in [
                (
                    "nvme0n1p1",
                    "259:1",
                    100,
                    "E:ID_FS_TYPE=vfat\nE:ID_FS_USAGE=filesystem\n",
                ),
                (
                    "nvme0n1p2",
                    "259:2",
                    7000,
                    "E:ID_FS_TYPE=LVM2_member\nE:ID_FS_USAGE=raid\n",
                ),
            ] {
                let dir = disk.join(part);
                fs::create_dir_all(&dir).unwrap();
                fs::write(dir.join("dev"), format!("{dev}\n")).unwrap();
                fs::write(dir.join("size"), sectors.to_string()).unwrap();
                fs::write(dir.join("partition"), "1").unwrap();
                symlink(&dir, machine.sys.class_block.join(part)).unwrap();
                symlink(&dir, machine.sys.dev_block.join(dev)).unwrap();
                fs::write(machine.sys.udev_data.join(format!("b{dev}")), udev_data).unwrap();
            }
            // The stack on the second partition: LVM, then LUKS, then the filesystem.
            for (name, dev, dm_name, uuid, slave, udev_data) in [
                (
                    "dm-0",
                    "254:0",
                    "Vg-root",
                    "LVM-abc",
                    "nvme0n1p2",
                    "E:ID_FS_TYPE=crypto_LUKS\nE:ID_FS_USAGE=crypto\n",
                ),
                (
                    "dm-1",
                    "254:1",
                    "root",
                    "CRYPT-LUKS2-def-root",
                    "dm-0",
                    "E:ID_FS_TYPE=btrfs\nE:ID_FS_USAGE=filesystem\n",
                ),
            ] {
                let dir = machine.node("virtual/block", name, dev, 6000, udev_data);
                fs::create_dir_all(dir.join("dm")).unwrap();
                fs::write(dir.join("dm/name"), format!("{dm_name}\n")).unwrap();
                fs::write(dir.join("dm/uuid"), format!("{uuid}\n")).unwrap();
                fs::create_dir_all(dir.join("slaves").join(slave)).unwrap();
            }
            symlink(
                machine.sys.class_block.join("dm-1"),
                machine.sys.dev_dir.join("mapper/root"),
            )
            .unwrap();

            // A USB stick with one partition.
            let usb = "pci0000:00/0000:00:14.0/usb2/2-3/2-3:1.0/host6/target6:0:0/6:0:0:0/block";
            let stick = machine.node(usb, "sdb", "8:16", 4000, "E:ID_BUS=usb\n");
            fs::write(stick.join("removable"), "1").unwrap();
            fs::create_dir_all(stick.join("device")).unwrap();
            fs::write(stick.join("device/model"), "Ultra\n").unwrap();
            let part = stick.join("sdb1");
            fs::create_dir_all(&part).unwrap();
            fs::write(part.join("dev"), "8:17\n").unwrap();
            fs::write(part.join("size"), "3900").unwrap();
            fs::write(part.join("partition"), "1").unwrap();
            symlink(&part, machine.sys.class_block.join("sdb1")).unwrap();
            symlink(&part, machine.sys.dev_block.join("8:17")).unwrap();
            fs::write(
                machine.sys.udev_data.join("b8:17"),
                "E:ID_BUS=usb\nE:ID_FS_LABEL=HOLIDAY\nE:ID_FS_TYPE=vfat\nE:ID_FS_USAGE=filesystem\n",
            )
            .unwrap();

            fs::write(
                machine.sys.proc_dir.join("self/mountinfo"),
                "22 1 0:40 / / rw - btrfs /dev/mapper/root rw\n\
                 23 22 259:1 / /boot rw - vfat /dev/nvme0n1p1 rw\n\
                 24 22 8:17 / /run/media/me/HOLIDAY rw - vfat /dev/sdb1 rw\n\
                 25 22 0:50 / /mnt/nas rw - nfs4 nas:/export rw\n",
            )
            .unwrap();
            machine
        }
    }

    impl Drop for Machine {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn find(t: &Topology, id: &str) -> usize {
        t.nodes
            .iter()
            .position(|n| n.id == id)
            .unwrap_or_else(|| panic!("no {id}"))
    }

    fn ids(t: &Topology, list: &[usize]) -> Vec<String> {
        list.iter().map(|&i| t.nodes[i].id.clone()).collect()
    }

    #[test]
    fn the_nvme_stack_is_drawn_from_the_bus_down_to_the_mounted_filesystem() {
        let machine = Machine::new("stack");
        let t = machine.sys.topology();
        let bus = find(&t, "PCIe 02:00.0");
        assert_eq!(t.nodes[bus].kind, NodeKind::Bus);
        let disk = find(&t, "nvme0n1");
        assert_eq!(t.nodes[disk].parents, [bus]);
        assert_eq!(ids(&t, &t.nodes[disk].children), ["nvme0n1p1", "nvme0n1p2"]);
        let p2 = find(&t, "nvme0n1p2");
        assert_eq!(ids(&t, &t.nodes[p2].children), ["dm-0"]);
        let lvm = find(&t, "dm-0");
        assert_eq!(t.nodes[lvm].kind, NodeKind::Layer(Layer::Lvm));
        assert_eq!(ids(&t, &t.nodes[lvm].children), ["dm-1"]);
        let fs = find(&t, "dm-1");
        assert_eq!(t.nodes[fs].kind, NodeKind::Layer(Layer::Crypt));
        assert_eq!(t.nodes[fs].mount.as_deref(), Some(Path::new("/")));
        assert_eq!(t.nodes[fs].fs.as_deref(), Some("btrfs"));
        assert_eq!(
            (t.nodes[fs].total, t.nodes[fs].free),
            (1000, 60),
            "sized by statvfs"
        );
    }

    #[test]
    fn the_boot_partition_is_part_of_the_picture_and_titles_prefer_names_over_kernel_ids() {
        let machine = Machine::new("titles");
        let t = machine.sys.topology();
        let boot = find(&t, "nvme0n1p1");
        assert_eq!(t.nodes[boot].mount.as_deref(), Some(Path::new("/boot")));
        assert_eq!(t.nodes[find(&t, "nvme0n1")].title(), "Samsung SSD 980");
        assert_eq!(t.nodes[find(&t, "dm-0")].title(), "Vg-root");
        assert_eq!(t.nodes[find(&t, "dm-1")].title(), "root");
        assert_eq!(t.nodes[find(&t, "sdb1")].title(), "HOLIDAY");
        assert_eq!(t.nodes[find(&t, "nvme0n1p2")].title(), "nvme0n1p2");
    }

    #[test]
    fn a_stick_hangs_off_its_usb_port_and_everything_on_it_is_removable() {
        let machine = Machine::new("usb");
        let t = machine.sys.topology();
        let port = find(&t, "USB 2-3");
        let stick = find(&t, "sdb");
        assert_eq!(t.nodes[stick].parents, [port]);
        assert!(t.nodes[stick].removable && t.nodes[find(&t, "sdb1")].removable);
        assert!(!t.nodes[find(&t, "nvme0n1p1")].removable);
        let device = t
            .device(find(&t, "sdb1"))
            .expect("a filesystem is a device");
        assert_eq!(
            (device.kind, device.disk.as_str()),
            (Kind::Removable, "sdb")
        );
        assert!(device.can_eject());
    }

    #[test]
    fn a_share_is_a_root_of_its_own_and_layers_that_hold_no_filesystem_are_not_devices() {
        let machine = Machine::new("share");
        let t = machine.sys.topology();
        let share = find(&t, "nas:/export");
        assert_eq!(t.nodes[share].kind, NodeKind::Share);
        assert!(t.nodes[share].parents.is_empty());
        assert_eq!(t.device(share).map(|d| d.kind), Some(Kind::Network));
        assert!(
            t.device(find(&t, "nvme0n1p2")).is_none(),
            "an LVM member is not mountable"
        );
        assert!(
            t.device(find(&t, "dm-0")).is_none(),
            "nor is an encrypted container"
        );
        assert!(t.device(find(&t, "dm-1")).is_some());
    }

    #[test]
    fn rows_come_depth_first_and_columns_are_the_longest_chain() {
        let machine = Machine::new("order");
        let t = machine.sys.topology();
        let order = ids(&t, &t.order());
        let at = |id: &str| order.iter().position(|o| o == id).unwrap();
        assert_eq!(order.len(), t.nodes.len(), "every node once");
        assert!(at("PCIe 02:00.0") < at("nvme0n1") && at("nvme0n1") < at("nvme0n1p1"));
        assert!(at("nvme0n1p2") < at("dm-0") && at("dm-0") < at("dm-1"));
        assert!(at("nvme0n1p1") < at("nvme0n1p2"), "siblings in name order");
        let depths = t.depths();
        let d = |id: &str| depths[find(&t, id)];
        assert_eq!(
            [
                d("PCIe 02:00.0"),
                d("nvme0n1"),
                d("nvme0n1p2"),
                d("dm-0"),
                d("dm-1")
            ],
            [0, 1, 2, 3, 4]
        );
        assert_eq!(d("USB 2-3"), 0);
        assert_eq!(d("nas:/export"), 0);
    }

    #[test]
    fn the_folder_being_browsed_belongs_to_the_deepest_mount_that_holds_it() {
        let machine = Machine::new("here");
        let t = machine.sys.topology();
        assert_eq!(t.holding(Path::new("/home/me")), Some(find(&t, "dm-1")));
        assert_eq!(
            t.holding(Path::new("/boot/grub")),
            Some(find(&t, "nvme0n1p1"))
        );
        assert_eq!(
            t.holding(Path::new("/run/media/me/HOLIDAY/photos")),
            Some(find(&t, "sdb1"))
        );
    }

    #[test]
    fn a_raid_array_has_two_parents_but_is_drawn_once_under_the_first() {
        let machine = Machine::new("raid");
        let usb = "pci0000:00/0000:00:17.0/ata2/host1/target1:0:0/1:0:0:0/block";
        for (disk, part, dev_d, dev_p) in [
            ("sdd", "sdd1", "8:48", "8:49"),
            ("sde", "sde1", "8:64", "8:65"),
        ] {
            let d = machine.node(usb, disk, dev_d, 4000, "");
            let p = d.join(part);
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("dev"), format!("{dev_p}\n")).unwrap();
            fs::write(p.join("size"), "3900").unwrap();
            fs::write(p.join("partition"), "1").unwrap();
            symlink(&p, machine.sys.class_block.join(part)).unwrap();
            symlink(&p, machine.sys.dev_block.join(dev_p)).unwrap();
        }
        let md = machine.node(
            "virtual/block",
            "md0",
            "9:0",
            3800,
            "E:ID_FS_TYPE=ext4\nE:ID_FS_USAGE=filesystem\n",
        );
        fs::create_dir_all(md.join("md")).unwrap();
        for slave in ["sdd1", "sde1"] {
            fs::create_dir_all(md.join("slaves").join(slave)).unwrap();
        }
        let t = machine.sys.topology();
        let array = find(&t, "md0");
        assert_eq!(t.nodes[array].kind, NodeKind::Layer(Layer::Raid));
        assert_eq!(ids(&t, &t.nodes[array].parents), ["sdd1", "sde1"]);
        let order = t.order();
        assert_eq!(
            order.iter().filter(|&&i| i == array).count(),
            1,
            "drawn once"
        );
        let first = find(&t, "sdd1");
        assert!(order.iter().position(|&i| i == first) < order.iter().position(|&i| i == array));
        assert_eq!(t.disk_of(array), Some(find(&t, "sdd")));
    }

    #[test]
    fn an_empty_card_reader_is_not_a_box() {
        let machine = Machine::new("empty");
        machine.node(
            "pci0000:00/0000:00:14.0/usb1/1-1/1-1:1.0/host2/target2:0:0/2:0:0:0/block",
            "sdz",
            "65:0",
            0,
            "",
        );
        let t = machine.sys.topology();
        assert!(t.nodes.iter().all(|n| n.id != "sdz"));
        assert!(
            t.nodes.iter().all(|n| n.id != "USB 1-1"),
            "nor is the port it hangs off"
        );
    }

    #[test]
    fn only_a_mounted_filesystem_or_a_folder_can_be_unfolded() {
        let machine = Machine::new("unfold");
        let t = machine.sys.topology();
        let path = |id: &str| t.nodes[find(&t, id)].folder_path().map(Path::to_path_buf);
        assert_eq!(path("dm-1"), Some(PathBuf::from("/")));
        assert_eq!(path("nvme0n1p1"), Some(PathBuf::from("/boot")));
        assert_eq!(path("nas:/export"), Some(PathBuf::from("/mnt/nas")));
        assert_eq!(path("nvme0n1"), None, "a disk is not a folder");
        assert_eq!(path("nvme0n1p2"), None, "nor an LVM member");
        assert_eq!(path("PCIe 02:00.0"), None);
        let mut folder = Node::new("/usr", NodeKind::Folder);
        assert_eq!(
            folder.folder_path(),
            None,
            "a folder without a path is nothing"
        );
        folder.path = Some("/usr".into());
        assert_eq!(folder.folder_path(), Some(Path::new("/usr")));
        let mut more = Node::new("/usr…more", NodeKind::More);
        more.path = Some("/usr".into());
        assert_eq!(
            more.folder_path(),
            None,
            "a note about more folders cannot be unfolded"
        );
    }

    #[test]
    fn a_machine_with_nothing_to_read_has_an_empty_picture() {
        let nowhere = PathBuf::from("/nonexistent/minuteman");
        let sys = Sys {
            dev_dir: nowhere.clone(),
            proc_dir: nowhere.clone(),
            class_block: nowhere.clone(),
            dev_block: nowhere.clone(),
            udev_data: nowhere,
            space: |_| None,
        };
        let t = sys.topology();
        assert!(t.nodes.is_empty() && t.roots().is_empty() && t.order().is_empty());
    }
}
