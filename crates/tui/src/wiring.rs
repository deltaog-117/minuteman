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

//! The wiring diagram's state (`V`, the wiring view): how the machine's storage is wired, which
//! node is selected, how busy each disk is, and what is being done to a device. Like the deck it is
//! pure: no `Frame`, no terminal. `wiring_view` draws it and `main` applies the few things it
//! asks for, which are to go to a folder and to copy the carried files somewhere.
//!
//! The machine is looked at once a second on the blocking pool, only while the view is showing, and
//! a scan still running is not started again, so a share that has gone away cannot stall the
//! interface. Mounting, unmounting and ejecting are the deck's (`device_actions`).
//!
//! The diagram goes on past the mount point: a filesystem, and then any folder, can be unfolded
//! into its subfolders, drawn as more boxes to the right (`wiring_folders`). The folders are kept
//! apart from the hardware, which is rebuilt every second, and added to a copy of it (`composed`)
//! that everything here reads. Opening the view unfolds the way down to the folder being browsed.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use devices::Device;
use devices::activity::{Activity, Counters, Rates};
use devices::probe::Sys;
use devices::topology::{NodeKind, Topology};
use ratatui::layout::{Position, Rect};
use shared::Vfs;

use crate::cargo_fit::{Load, Verdict, verdict};
use crate::device_actions::Actions;
use crate::wiring_folders::Folders;
use crate::wiring_layout::{self, Layout, Mode};

/// How often the machine is looked at again.
const SCAN_EVERY: Duration = Duration::from_secs(1);

/// Shown on entering the view.
pub const HINT: &str = "wiring: arrows move  →/click unfold folders  Enter open  p send  m mount  e eject  V next view";

/// What a key or click did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not the diagram's: the browser handles it as usual.
    Pass,
    /// Handled here.
    Handled,
    /// Go to this folder.
    Goto(PathBuf),
    /// Copy what is carried into this folder: the root of the chosen device, or the folder chosen.
    Send(PathBuf),
}

struct Scan {
    at: Instant,
    topology: Topology,
    counters: std::collections::HashMap<String, Counters>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Move {
    Up,
    Down,
    Left,
    Right,
}

pub struct WiringState {
    handle: tokio::runtime::Handle,
    /// The hardware, as the last scan found it.
    base: Topology,
    /// The hardware with every unfolded folder added below its filesystem: what is drawn.
    composed: Topology,
    folders: Folders,
    /// The folder last unfolded down to, so it happens once for each folder browsed.
    last_here: Option<PathBuf>,
    activity: Activity,
    activity_at: Option<Instant>,
    scanned: bool,
    /// The kernel name (or share, or bus label) of the selected node, so the selection stays on it
    /// as the machine changes.
    selected: Option<String>,
    /// The first canvas row and column drawn, kept by the renderer between frames.
    top: Cell<u16>,
    left: Cell<u16>,
    scans: Receiver<Scan>,
    scan_tx: Sender<Scan>,
    scanning: Arc<AtomicBool>,
    last_scan: Option<Instant>,
    actions: Actions,
    load: Option<Load>,
    /// The time activity marks are measured from, so they move at a steady pace.
    epoch: Instant,
}

impl WiringState {
    pub fn new(handle: tokio::runtime::Handle, vfs: Arc<dyn Vfs>, show_hidden: bool) -> Self {
        let (scan_tx, scans) = channel();
        Self {
            actions: Actions::new(handle.clone()),
            folders: Folders::new(handle.clone(), vfs, show_hidden),
            handle,
            base: Topology::default(),
            composed: Topology::default(),
            last_here: None,
            activity: Activity::new(),
            activity_at: None,
            scanned: false,
            selected: None,
            top: Cell::new(0),
            left: Cell::new(0),
            scans,
            scan_tx,
            scanning: Arc::new(AtomicBool::new(false)),
            last_scan: None,
            load: None,
            epoch: Instant::now(),
        }
    }

    /// A diagram that already holds `topology`, without ever looking at the real machine, over an
    /// empty filesystem.
    #[cfg(test)]
    pub fn with_topology(handle: tokio::runtime::Handle, topology: Topology) -> Self {
        Self::with_vfs(handle, topology, Arc::new(shared::MemVfs::new()))
    }

    /// Like [`with_topology`](Self::with_topology), with `vfs` to read folders from.
    #[cfg(test)]
    pub fn with_vfs(handle: tokio::runtime::Handle, topology: Topology, vfs: Arc<dyn Vfs>) -> Self {
        let mut state = Self::new(handle, vfs, false);
        // Never look at the real machine: the topology given is the machine.
        state.scanning.store(true, Ordering::Release);
        state.base = topology;
        state.scanned = true;
        state.rebuild();
        state.reselect(None);
        state
    }

    /// Feeds the activity counters as a scan would, for how the drawing shows a busy disk.
    #[cfg(test)]
    pub fn activity_update(&mut self, counters: &std::collections::HashMap<String, Counters>) {
        self.activity.update(counters, 1.0);
    }

    #[cfg(test)]
    pub fn epoch_for_test(&self) -> Instant {
        self.epoch
    }

    #[cfg(test)]
    pub fn select_for_test(&mut self, id: &str) {
        self.selected = Some(id.to_string());
    }

    #[cfg(test)]
    pub fn mark_scanned_for_test(&mut self) {
        self.scanned = true;
    }

    /// Takes in a finished scan and starts the next when it is time. `here` is the folder being
    /// browsed, which picks the first selection. Call it every turn of the loop; it never waits.
    pub fn poll(&mut self, now: Instant, here: &Path) {
        while let Ok(scan) = self.scans.try_recv() {
            let seconds = self
                .activity_at
                .map_or(1.0, |before| scan.at.duration_since(before).as_secs_f64());
            self.activity_at = Some(scan.at);
            self.activity.update(&scan.counters, seconds);
            self.base = scan.topology;
            self.scanned = true;
            self.rebuild();
        }
        if self.folders.poll() {
            self.rebuild();
        }
        self.follow(here);
        self.reselect(Some(here));
        if self.actions.poll() {
            // Whatever was just done changed what is mounted: look again at once.
            self.last_scan = None;
        }
        let due = self
            .last_scan
            .is_none_or(|at| now.duration_since(at) >= SCAN_EVERY);
        if due && !self.scanning.swap(true, Ordering::AcqRel) {
            self.last_scan = Some(now);
            let (tx, scanning) = (self.scan_tx.clone(), Arc::clone(&self.scanning));
            self.handle.spawn_blocking(move || {
                let sys = Sys::real();
                let topology = sys.topology();
                let counters = sys.counters();
                let _ = tx.send(Scan {
                    at: Instant::now(),
                    topology,
                    counters,
                });
                scanning.store(false, Ordering::Release);
            });
        }
    }

    /// Adds the unfolded folders to the hardware again, after either has changed.
    fn rebuild(&mut self) {
        self.composed = self.folders.compose(&self.base);
    }

    /// Unfolds the way down to `here`, the folder being browsed, and chooses it, once for each
    /// folder. Waits for the first scan, since the filesystem it hangs from is not known before.
    fn follow(&mut self, here: &Path) {
        if self.last_here.as_deref() == Some(here) {
            return;
        }
        let Some(fs) = self.base.holding(here) else {
            return;
        };
        let Some(root) = self.base.nodes[fs].folder_path().map(Path::to_path_buf) else {
            return;
        };
        self.last_here = Some(here.to_path_buf());
        self.folders.set_toward(Some(here.to_path_buf()));
        let mut path = root.clone();
        self.folders.expand(&path);
        if let Ok(rest) = here.strip_prefix(&root) {
            for part in rest.components() {
                path.push(part);
                self.folders.expand(&path);
            }
        }
        self.selected = Some(if path == root {
            self.base.nodes[fs].id.clone()
        } else {
            path.display().to_string()
        });
        self.rebuild();
    }

    /// Keeps the selection on its node, or chooses one when it has none, or its node has gone
    /// (a folder that was removed, one folded away): the node holding `here`, else the first in
    /// reading order. A folder still being read is waited for.
    fn reselect(&mut self, here: Option<&Path>) {
        if self.index().is_some() || (self.selected.is_some() && self.folders.pending()) {
            return;
        }
        let first = here
            .and_then(|dir| self.composed.holding(dir))
            .or_else(|| self.composed.order().first().copied());
        self.selected = first.map(|i| self.composed.nodes[i].id.clone());
    }

    pub fn topology(&self) -> &Topology {
        &self.composed
    }

    pub fn has_scanned(&self) -> bool {
        self.scanned
    }

    /// The index of the selected node in the topology.
    pub fn index(&self) -> Option<usize> {
        let id = self.selected.as_deref()?;
        self.composed.nodes.iter().position(|n| n.id == id)
    }

    pub fn top(&self) -> &Cell<u16> {
        &self.top
    }

    pub fn left(&self) -> &Cell<u16> {
        &self.left
    }

    /// Whether the node is a folder (or a filesystem) that is unfolded.
    pub fn is_unfolded(&self, node: usize) -> bool {
        self.composed.nodes[node]
            .folder_path()
            .is_some_and(|p| self.folders.is_expanded(p))
    }

    /// How many subfolders the node has, once they have been read.
    pub fn subfolders(&self, node: usize) -> Option<usize> {
        self.folders
            .count_of(self.composed.nodes[node].folder_path()?)
    }

    /// The node to light as "you are here" for the folder `dir`: the folder's own box if it is
    /// unfolded on screen, else the filesystem that holds it.
    pub fn here(&self, dir: &Path) -> Option<usize> {
        self.composed
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::Folder && n.path.as_deref() == Some(dir))
            .or_else(|| self.composed.holding(dir))
    }

    pub fn is_working(&self) -> bool {
        self.actions.is_working()
    }

    /// The next message or folder change from background work, if any.
    pub fn take_event(&mut self) -> Option<crate::device_actions::Event> {
        self.actions.take_event()
    }

    /// Tells the diagram what is being carried (or that nothing is), so each filesystem can say
    /// whether it fits.
    pub fn set_load(&mut self, load: Option<Load>) {
        self.load = load;
    }

    /// Whether the load fits on the filesystem at `node`, or `None` when nothing is carried or the
    /// node holds no filesystem.
    pub fn verdict(&self, node: usize) -> Option<Verdict> {
        let load = self.load.as_ref()?;
        let device = self.composed.device(node)?;
        let same_device = self.composed.holding(&load.source) == Some(node);
        Some(verdict(&device, load, same_device))
    }

    /// How fast data is moving through `node`: its own counters, or, for a bus, the sum of what
    /// is attached to it.
    pub fn rates(&self, node: usize) -> Rates {
        let n = &self.composed.nodes[node];
        if n.kind == NodeKind::Bus {
            return n.children.iter().fold(Rates::default(), |sum, &c| {
                let r = self.activity.rates(&self.composed.nodes[c].id);
                Rates {
                    read: sum.read + r.read,
                    written: sum.written + r.written,
                }
            });
        }
        self.activity.rates(&n.id)
    }

    /// How far along a moving mark is, as a count that rises with time.
    pub fn phase(&self, now: Instant) -> u64 {
        u64::try_from(now.saturating_duration_since(self.epoch).as_millis() / 90).unwrap_or(0)
    }

    /// The layout for a body `width` cells wide.
    pub fn layout(&self, width: u16) -> Layout {
        wiring_layout::layout(&self.composed, width)
    }

    /// Where the selection goes for `direction`: up and down along the page, left to what a node
    /// is under and right to what is under it.
    fn target(&self, direction: Move, layout: &Layout) -> Option<usize> {
        let at = self.index()?;
        let t = &self.composed;
        let first_child = || {
            t.nodes[at]
                .children
                .iter()
                .copied()
                .find(|&c| t.nodes[c].parents.first() == Some(&at))
        };
        match direction {
            Move::Left => t.nodes[at].parents.first().copied(),
            Move::Right => first_child(),
            Move::Up | Move::Down => {
                let step = |list: &[usize]| {
                    let here = list.iter().position(|&n| n == at)?;
                    match direction {
                        Move::Up => here.checked_sub(1),
                        _ => Some(here + 1).filter(|&i| i < list.len()),
                    }
                    .map(|i| list[i])
                };
                match layout.mode {
                    Mode::Narrow => {
                        let rows: Vec<usize> = layout.rows.iter().map(|r| r.node).collect();
                        step(&rows)
                    }
                    Mode::Wide => {
                        // Along its own column, to the box above or below.
                        let column = layout.rect_of(at)?.x;
                        let mut same: Vec<_> = layout
                            .placed
                            .iter()
                            .filter(|p| p.rect.x == column)
                            .collect();
                        same.sort_by_key(|p| p.rect.y);
                        let list: Vec<usize> = same.iter().map(|p| p.node).collect();
                        step(&list)
                    }
                }
            }
        }
    }

    /// The folder the selected node stands for, if it can be unfolded.
    fn selected_folder(&self) -> Option<PathBuf> {
        self.composed.nodes[self.index()?]
            .folder_path()
            .map(Path::to_path_buf)
    }

    /// Right unfolds a folder that is folded and otherwise goes to what is under the node; left
    /// folds a folder that is unfolded and otherwise goes to what the node is under.
    fn fold_or_go(&mut self, direction: Move, width: u16) {
        if let Some(path) = self.selected_folder() {
            let open = self.folders.is_expanded(&path);
            match (direction, open) {
                (Move::Right, false) => {
                    self.folders.expand(&path);
                    self.rebuild();
                    return;
                }
                (Move::Left, true) => {
                    self.folders.collapse(&path);
                    self.rebuild();
                    return;
                }
                _ => {}
            }
        }
        self.go(direction, width);
    }

    fn go(&mut self, direction: Move, width: u16) {
        let layout = self.layout(width);
        if let Some(next) = self.target(direction, &layout) {
            self.selected = Some(self.composed.nodes[next].id.clone());
        }
    }

    /// Moves the selection to the next or previous node in reading order, as the wheel does.
    pub fn scroll(&mut self, by: isize, width: u16) {
        let layout = self.layout(width);
        let order: Vec<usize> = match layout.mode {
            Mode::Narrow => layout.rows.iter().map(|r| r.node).collect(),
            Mode::Wide => layout.placed.iter().map(|p| p.node).collect(),
        };
        let Some(here) = self
            .index()
            .and_then(|a| order.iter().position(|&n| n == a))
        else {
            return;
        };
        let to = here
            .saturating_add_signed(by)
            .min(order.len().saturating_sub(1));
        if let Some(&node) = order.get(to) {
            self.selected = Some(self.composed.nodes[node].id.clone());
        }
    }

    /// The device the selected node is, when it holds a filesystem.
    fn selected_device(&self) -> Option<Device> {
        self.composed.device(self.index()?)
    }

    fn say(&self, text: impl Into<String>) {
        self.actions.say(text);
    }

    /// The selection moves with the arrows (and `h`/`j`/`k`/`l`), `Enter` opens a filesystem
    /// (mounting it first if need be), `m` mounts or unmounts it, `e` ejects the removable disk the
    /// node is on, `p` sends the carried files to a filesystem. Every other key is the browser's,
    /// so `V`, `q`, `/` and `:` work as they always do.
    pub fn key(&mut self, key: KeyEvent, width: u16) -> Outcome {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT)
        {
            return Outcome::Pass;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.go(Move::Up, width),
            KeyCode::Down | KeyCode::Char('j') => self.go(Move::Down, width),
            KeyCode::Left | KeyCode::Char('h') => self.fold_or_go(Move::Left, width),
            KeyCode::Right | KeyCode::Char('l') => self.fold_or_go(Move::Right, width),
            KeyCode::Home => self.scroll(isize::MIN / 2, width),
            KeyCode::End => self.scroll(isize::MAX / 2, width),
            KeyCode::PageUp => self.scroll(-8, width),
            KeyCode::PageDown => self.scroll(8, width),
            KeyCode::Enter => return self.open(),
            KeyCode::Char('m') => self.toggle(),
            KeyCode::Char('e') => self.eject(),
            KeyCode::Char('p') => return self.send(),
            _ => return Outcome::Pass,
        }
        Outcome::Handled
    }

    /// A click on a node in a diagram drawn in `area` selects it, and unfolds it if it is a folded
    /// folder or folds it if it is an unfolded one. A click on nothing does nothing.
    pub fn click(&mut self, area: Rect, pointer: Position) -> Outcome {
        let layout = self.layout(area.width);
        let x = pointer
            .x
            .saturating_sub(area.x)
            .saturating_add(self.left.get());
        let y = pointer
            .y
            .saturating_sub(area.y)
            .saturating_add(self.top.get());
        let Some(node) = layout.node_at(x, y) else {
            return Outcome::Handled;
        };
        self.selected = Some(self.composed.nodes[node].id.clone());
        if let Some(path) = self.selected_folder() {
            if self.folders.is_expanded(&path) {
                self.folders.collapse(&path);
            } else {
                self.folders.expand(&path);
            }
            self.rebuild();
        }
        Outcome::Handled
    }

    fn open(&mut self) -> Outcome {
        // A folder, or the note about more folders in one, is opened in the browser.
        if let Some(at) = self.index() {
            let node = &self.composed.nodes[at];
            if matches!(node.kind, NodeKind::Folder | NodeKind::More)
                && let Some(path) = node.path.clone()
            {
                return Outcome::Goto(path);
            }
        }
        let Some(device) = self.selected_device() else {
            self.say("nothing to open here: choose a filesystem (to its right)");
            return Outcome::Handled;
        };
        match self.actions.open(&device) {
            Some(mount) => Outcome::Goto(mount),
            None => Outcome::Handled,
        }
    }

    fn toggle(&mut self) {
        match self.selected_device() {
            Some(device) => self.actions.toggle(&device),
            None => self.say("nothing to mount here: choose a filesystem"),
        }
    }

    /// Ejects the removable disk the selected node is on: every mounted volume on it is unmounted
    /// first, siblings included, then the disk is powered off.
    fn eject(&mut self) {
        let Some(at) = self.index() else { return };
        let Some(disk) = self.composed.disk_of(at) else {
            self.say("this is not on a disk that can be ejected");
            return;
        };
        let node = &self.composed.nodes[disk];
        if !node.removable {
            self.say(format!("{} cannot be ejected", node.title()));
            return;
        }
        let mounted = self.mounted_on(disk);
        self.actions.eject_all(&node.title(), &mounted, &node.id);
    }

    /// Every mounted filesystem anywhere under `disk`.
    fn mounted_on(&self, disk: usize) -> Vec<Device> {
        let mut found = Vec::new();
        let mut pending = vec![disk];
        let mut seen = std::collections::HashSet::new();
        while let Some(at) = pending.pop() {
            if !seen.insert(at) {
                continue;
            }
            if let Some(device) = self.composed.device(at).filter(Device::is_mounted) {
                found.push(device);
            }
            pending.extend(self.composed.nodes[at].children.iter().copied());
        }
        found.sort_by(|a, b| a.node.cmp(&b.node));
        found
    }

    fn send(&mut self) -> Outcome {
        let Some(at) = self.index() else {
            return Outcome::Handled;
        };
        // To a folder: the same check as for the filesystem it is on, and the files go in the
        // folder.
        let node = &self.composed.nodes[at];
        if matches!(node.kind, NodeKind::Folder | NodeKind::More)
            && let Some(path) = node.path.clone()
        {
            let on = self.composed.holding(&path);
            let Some((fs, device)) = on.and_then(|fs| Some((fs, self.composed.device(fs)?))) else {
                self.say("this folder is not on a filesystem that can be sent to");
                return Outcome::Handled;
            };
            return match self.actions.send_to(&device, self.verdict(fs)) {
                Some(_) => Outcome::Send(path),
                None => Outcome::Handled,
            };
        }
        let Some(device) = self.composed.device(at) else {
            self.say("choose a filesystem to send to");
            return Outcome::Handled;
        };
        match self.actions.send_to(&device, self.verdict(at)) {
            Some(mount) => Outcome::Send(mount),
            None => Outcome::Handled,
        }
    }
}

#[cfg(test)]
mod tests {
    use devices::topology::{Layer, Node};

    use super::*;
    use crate::device_actions::Event;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// bus -> nvme -> {boot (/boot), pv -> lvm -> root (/)}, and bus -> stick -> {a, b}.
    fn machine() -> Topology {
        let mut t = Topology::default();
        let mut node = |id: &str, kind: NodeKind, mount: Option<&str>, fs: Option<&str>| {
            let mut n = Node::new(id, kind);
            n.mount = mount.map(PathBuf::from);
            n.fs = fs.map(str::to_string);
            n.usage = fs.map(|_| "filesystem".to_string());
            n.total = 1000;
            n.free = 400;
            t.add(n)
        };
        let pcie = node("PCIe", NodeKind::Bus, None, None);
        let nvme = node("nvme0n1", NodeKind::Disk, None, None);
        let boot = node(
            "nvme0n1p1",
            NodeKind::Partition,
            Some("/boot"),
            Some("vfat"),
        );
        let pv = node("nvme0n1p2", NodeKind::Partition, None, None);
        let lvm = node("dm-0", NodeKind::Layer(Layer::Lvm), None, None);
        let root = node(
            "dm-1",
            NodeKind::Layer(Layer::Crypt),
            Some("/"),
            Some("btrfs"),
        );
        let usb = node("USB 2-3", NodeKind::Bus, None, None);
        let stick = node("sdb", NodeKind::Disk, None, None);
        let a = node(
            "sdb1",
            NodeKind::Partition,
            Some("/run/media/me/A"),
            Some("vfat"),
        );
        let b = node(
            "sdb2",
            NodeKind::Partition,
            Some("/run/media/me/B"),
            Some("ext4"),
        );
        for (p, c) in [
            (pcie, nvme),
            (nvme, boot),
            (nvme, pv),
            (pv, lvm),
            (lvm, root),
            (usb, stick),
            (stick, a),
            (stick, b),
        ] {
            t.link(p, c);
        }
        t.nodes[stick].removable = true;
        t.nodes[a].removable = true;
        t.nodes[b].removable = true;
        t
    }

    fn state() -> (tokio::runtime::Runtime, WiringState) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let state = WiringState::with_topology(runtime.handle().clone(), machine());
        (runtime, state)
    }

    fn at(s: &WiringState) -> String {
        s.selected.clone().unwrap()
    }

    const WIDE: u16 = 200;
    const NARROW: u16 = 40;

    #[test]
    fn the_first_node_in_reading_order_is_selected_to_begin_with() {
        let (_rt, s) = state();
        assert_eq!(at(&s), "PCIe");
    }

    #[test]
    fn right_goes_to_what_is_under_a_node_and_left_to_what_it_is_under() {
        for width in [WIDE, NARROW] {
            let (_rt, mut s) = state();
            for _ in 0..2 {
                s.key(press(KeyCode::Right), width);
            }
            assert_eq!(
                at(&s),
                "nvme0n1p1",
                "width {width}: bus, disk, then the first partition"
            );
            s.key(press(KeyCode::Left), width);
            assert_eq!(at(&s), "nvme0n1");
            s.key(press(KeyCode::Left), width);
            s.key(press(KeyCode::Left), width);
            assert_eq!(at(&s), "PCIe", "stays at the top of the stack");
        }
    }

    #[test]
    fn up_and_down_move_along_the_page_in_either_layout() {
        let (_rt, mut s) = state();
        s.selected = Some("nvme0n1p1".into());
        s.key(press(KeyCode::Down), WIDE);
        assert_eq!(at(&s), "nvme0n1p2", "in the same column, the box below");
        s.key(press(KeyCode::Down), WIDE);
        assert_eq!(
            at(&s),
            "sdb1",
            "the next box down that column is the stick's partition"
        );
        s.key(press(KeyCode::Down), WIDE);
        s.key(press(KeyCode::Down), WIDE);
        assert_eq!(at(&s), "sdb2", "and nothing below the last one");
        s.selected = Some("nvme0n1p2".into());
        s.key(press(KeyCode::Char('k')), WIDE);
        assert_eq!(at(&s), "nvme0n1p1");
        s.selected = Some("PCIe".into());
        s.key(press(KeyCode::Down), NARROW);
        assert_eq!(at(&s), "nvme0n1", "in the tree, the next row");
        s.key(press(KeyCode::Char('j')), NARROW);
        assert_eq!(at(&s), "nvme0n1p1");
    }

    #[test]
    fn home_end_and_the_page_keys_move_through_reading_order_and_stop_at_the_ends() {
        let (_rt, mut s) = state();
        s.key(press(KeyCode::End), NARROW);
        assert_eq!(at(&s), "sdb2");
        s.key(press(KeyCode::Home), NARROW);
        assert_eq!(at(&s), "PCIe");
        s.key(press(KeyCode::PageDown), NARROW);
        assert_eq!(at(&s), "sdb1", "eight rows down");
        s.key(press(KeyCode::PageUp), NARROW);
        assert_eq!(at(&s), "PCIe");
    }

    #[test]
    fn keys_the_diagram_does_not_use_are_the_browsers() {
        let (_rt, mut s) = state();
        for code in [
            KeyCode::Char('q'),
            KeyCode::Char('V'),
            KeyCode::Char('/'),
            KeyCode::Char(':'),
            KeyCode::Tab,
            KeyCode::Esc,
        ] {
            assert_eq!(s.key(press(code), WIDE), Outcome::Pass, "{code:?}");
        }
        assert_eq!(
            s.key(
                KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL),
                WIDE
            ),
            Outcome::Pass
        );
    }

    #[test]
    fn enter_opens_a_mounted_filesystem_and_says_what_to_do_on_anything_else() {
        let (_rt, mut s) = state();
        assert_eq!(s.key(press(KeyCode::Enter), WIDE), Outcome::Handled);
        s.selected = Some("dm-1".into());
        assert_eq!(
            s.key(press(KeyCode::Enter), WIDE),
            Outcome::Goto(PathBuf::from("/"))
        );
        s.actions.poll();
        let Some(Event::Status(text)) = s.take_event() else {
            panic!("expected a message")
        };
        assert!(text.contains("nothing to open"), "{text}");
    }

    #[test]
    fn m_and_p_refuse_a_node_that_holds_no_filesystem() {
        let (_rt, mut s) = state();
        s.selected = Some("dm-0".into());
        assert_eq!(s.key(press(KeyCode::Char('m')), WIDE), Outcome::Handled);
        assert_eq!(s.key(press(KeyCode::Char('p')), WIDE), Outcome::Handled);
        s.actions.poll();
        let mut said = Vec::new();
        while let Some(Event::Status(t)) = s.take_event() {
            said.push(t);
        }
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].contains("nothing to mount") && said[1].contains("choose a filesystem"));
    }

    #[test]
    fn p_sends_to_a_filesystem_it_fits_on_and_refuses_one_it_does_not() {
        let (_rt, mut s) = state();
        s.set_load(Some(Load {
            bytes: 300,
            exact: true,
            largest: 10,
            carry: crate::cargo_fit::Carry::Copy,
            source: "/home/me/photos".into(),
        }));
        s.selected = Some("sdb1".into());
        assert_eq!(
            s.key(press(KeyCode::Char('p')), WIDE),
            Outcome::Send(PathBuf::from("/run/media/me/A"))
        );
        s.set_load(Some(Load {
            bytes: 900,
            exact: true,
            largest: 10,
            carry: crate::cargo_fit::Carry::Copy,
            source: "/home/me/photos".into(),
        }));
        assert_eq!(s.key(press(KeyCode::Char('p')), WIDE), Outcome::Handled);
        s.actions.poll();
        let Some(Event::Status(text)) = s.take_event() else {
            panic!("expected a refusal")
        };
        assert!(text.contains("needs"), "{text}");
    }

    #[test]
    fn a_verdict_exists_only_for_a_filesystem_and_only_while_something_is_carried() {
        let (_rt, mut s) = state();
        let boot = s
            .composed
            .nodes
            .iter()
            .position(|n| n.id == "nvme0n1p1")
            .unwrap();
        let pv = s
            .composed
            .nodes
            .iter()
            .position(|n| n.id == "nvme0n1p2")
            .unwrap();
        assert_eq!(s.verdict(boot), None);
        s.set_load(Some(Load {
            bytes: 1,
            exact: true,
            largest: 1,
            carry: crate::cargo_fit::Carry::Copy,
            source: "/x".into(),
        }));
        assert!(matches!(
            s.verdict(boot),
            Some(Verdict::Fits { .. } | Verdict::Tight { .. })
        ));
        assert_eq!(s.verdict(pv), None, "an LVM member holds no filesystem");
    }

    #[test]
    fn ejecting_names_the_whole_disk_and_every_volume_on_it() {
        let (_rt, s) = state();
        let stick = s.composed.nodes.iter().position(|n| n.id == "sdb").unwrap();
        let mounted: Vec<String> = s.mounted_on(stick).iter().map(|d| d.node.clone()).collect();
        assert_eq!(mounted, ["sdb1", "sdb2"]);
        let nvme = s
            .composed
            .nodes
            .iter()
            .position(|n| n.id == "nvme0n1")
            .unwrap();
        assert_eq!(
            s.mounted_on(nvme).len(),
            2,
            "the boot partition and the root"
        );
    }

    #[test]
    fn ejecting_a_fixed_disk_or_a_share_is_refused_with_the_reason() {
        let (_rt, mut s) = state();
        s.selected = Some("dm-1".into());
        s.key(press(KeyCode::Char('e')), WIDE);
        s.selected = Some("PCIe".into());
        s.key(press(KeyCode::Char('e')), WIDE);
        s.actions.poll();
        let mut said = Vec::new();
        while let Some(Event::Status(t)) = s.take_event() {
            said.push(t);
        }
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].contains("cannot be ejected"), "{said:?}");
        assert!(said[1].contains("not on a disk"), "{said:?}");
        assert!(!s.is_working());
    }

    #[test]
    fn a_click_chooses_a_node_and_does_nothing_on_empty_space() {
        let (_rt, mut s) = state();
        let width = 200;
        let layout = s.layout(width);
        let find =
            |s: &WiringState, id: &str| s.composed.nodes.iter().position(|n| n.id == id).unwrap();
        let disk = layout.rect_of(find(&s, "nvme0n1")).unwrap();
        let area = Rect::new(3, 2, width, 40);
        let inside = Position::new(area.x + disk.x + 2, area.y + disk.y + 2);
        s.selected = Some("PCIe".into());
        assert_eq!(s.click(area, inside), Outcome::Handled);
        assert_eq!(at(&s), "nvme0n1");
        assert_eq!(
            s.click(area, inside),
            Outcome::Handled,
            "a disk is not a folder: a second click is just a click"
        );
        assert_eq!(
            s.click(area, Position::new(area.x, area.y + 39)),
            Outcome::Handled,
            "empty space"
        );
        assert_eq!(at(&s), "nvme0n1");
    }

    #[test]
    fn a_bus_adds_up_what_is_attached_to_it_and_other_nodes_use_their_own_counters() {
        let (_rt, mut s) = state();
        let mut counters = std::collections::HashMap::new();
        counters.insert(
            "nvme0n1".to_string(),
            Counters {
                read: 0,
                written: 0,
            },
        );
        counters.insert(
            "dm-1".to_string(),
            Counters {
                read: 0,
                written: 0,
            },
        );
        s.activity.update(&counters, 1.0);
        counters.insert(
            "nvme0n1".to_string(),
            Counters {
                read: 4000,
                written: 1000,
            },
        );
        counters.insert(
            "dm-1".to_string(),
            Counters {
                read: 500,
                written: 0,
            },
        );
        s.activity.update(&counters, 1.0);
        let find = |id: &str| s.composed.nodes.iter().position(|n| n.id == id).unwrap();
        assert_eq!(
            s.rates(find("PCIe")),
            Rates {
                read: 4000,
                written: 1000
            }
        );
        assert_eq!(
            s.rates(find("dm-1")),
            Rates {
                read: 500,
                written: 0
            }
        );
        assert_eq!(s.rates(find("sdb")), Rates::default());
    }

    #[test]
    fn marks_advance_with_time_at_a_steady_pace() {
        let (_rt, s) = state();
        let t0 = s.epoch;
        assert_eq!(s.phase(t0), 0);
        assert_eq!(s.phase(t0 + Duration::from_millis(900)), 10);
        assert!(s.phase(t0 + Duration::from_secs(5)) > s.phase(t0 + Duration::from_secs(4)));
    }

    // ---- unfolding folders ----

    fn dirs() -> Arc<shared::MemVfs> {
        let vfs = shared::MemVfs::new();
        for dir in [
            "/usr/lib",
            "/etc",
            "/home/me/projects/minuteman",
            "/boot/grub",
            "/boot/efi",
            "/run/media/me/A/photos",
        ] {
            vfs.add_dir(dir);
        }
        Arc::new(vfs)
    }

    /// A folder no mount holds (it is not absolute), so opening the view follows nowhere.
    const ELSEWHERE: &str = "elsewhere";

    fn folder_state() -> (tokio::runtime::Runtime, WiringState) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let state = WiringState::with_vfs(runtime.handle().clone(), machine(), dirs());
        (runtime, state)
    }

    /// Lets every folder being read arrive, with `here` as the folder being browsed.
    fn settle(s: &mut WiringState, here: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll(Instant::now(), Path::new(here));
            if !s.folders.pending() {
                return;
            }
            assert!(Instant::now() < deadline, "a folder was never read");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn titles(s: &WiringState, kind: NodeKind) -> Vec<String> {
        s.composed
            .nodes
            .iter()
            .filter(|n| n.kind == kind)
            .map(devices::topology::Node::title)
            .collect()
    }

    fn click_node(s: &mut WiringState, id: &str) {
        let width = 400;
        let layout = s.layout(width);
        let at = s.composed.nodes.iter().position(|n| n.id == id).unwrap();
        let rect = layout.rect_of(at).unwrap();
        let area = Rect::new(0, 0, width, 100);
        s.click(area, Position::new(rect.x + 2, rect.y + 1));
    }

    #[test]
    fn a_click_on_a_filesystem_unfolds_its_folders_and_another_click_folds_them() {
        let (_rt, mut s) = folder_state();
        click_node(&mut s, "nvme0n1p1");
        assert_eq!(at(&s), "nvme0n1p1");
        assert!(s.is_unfolded(s.index().unwrap()));
        settle(&mut s, ELSEWHERE);
        assert_eq!(titles(&s, NodeKind::Folder), ["efi", "grub"]);
        click_node(&mut s, "nvme0n1p1");
        assert!(titles(&s, NodeKind::Folder).is_empty());
        assert!(!s.is_unfolded(s.index().unwrap()));
    }

    #[test]
    fn a_click_on_a_folder_unfolds_it_in_turn() {
        let (_rt, mut s) = folder_state();
        click_node(&mut s, "dm-1");
        settle(&mut s, ELSEWHERE);
        assert_eq!(
            titles(&s, NodeKind::Folder),
            ["boot", "etc", "home", "run", "usr"]
        );
        click_node(&mut s, "/usr");
        settle(&mut s, ELSEWHERE);
        assert!(titles(&s, NodeKind::Folder).contains(&"lib".to_string()));
        assert_eq!(at(&s), "/usr");
    }

    #[test]
    fn right_unfolds_a_folded_folder_then_goes_into_it_and_left_goes_back_then_folds() {
        let (_rt, mut s) = folder_state();
        s.selected = Some("nvme0n1p1".into());
        assert_eq!(s.key(press(KeyCode::Right), WIDE), Outcome::Handled);
        assert_eq!(at(&s), "nvme0n1p1", "unfolding does not move the selection");
        settle(&mut s, ELSEWHERE);
        s.key(press(KeyCode::Right), WIDE);
        assert_eq!(
            at(&s),
            "/boot/efi",
            "a second Right goes into the first folder"
        );
        s.key(press(KeyCode::Left), WIDE);
        assert_eq!(
            at(&s),
            "nvme0n1p1",
            "Left from an unopened folder goes back to what it is under"
        );
        s.key(press(KeyCode::Left), WIDE);
        assert!(
            titles(&s, NodeKind::Folder).is_empty(),
            "and Left on an unfolded one folds it"
        );
        assert_eq!(at(&s), "nvme0n1p1");
    }

    #[test]
    fn opening_the_view_unfolds_the_way_down_to_the_folder_being_browsed_and_chooses_it() {
        let (_rt, mut s) = folder_state();
        settle(&mut s, "/home/me/projects");
        assert_eq!(at(&s), "/home/me/projects");
        let folders = titles(&s, NodeKind::Folder);
        for want in ["home", "me", "projects", "minuteman"] {
            assert!(
                folders.contains(&want.to_string()),
                "{want} missing: {folders:?}"
            );
        }
        let here = s.here(Path::new("/home/me/projects")).unwrap();
        assert_eq!(
            s.composed.nodes[here].kind,
            NodeKind::Folder,
            "its own box is the one lit"
        );
        assert_eq!(
            s.here(Path::new("/etc/ssh")),
            s.composed.holding(Path::new("/")),
            "a folder with no box of its own lights the filesystem"
        );
    }

    #[test]
    fn following_happens_once_for_each_folder_browsed_and_does_not_undo_a_fold() {
        let (_rt, mut s) = folder_state();
        settle(&mut s, "/home/me/projects");
        click_node(&mut s, "/home");
        assert!(!s.folders.is_expanded(Path::new("/home")));
        settle(&mut s, "/home/me/projects");
        assert!(
            !s.folders.is_expanded(Path::new("/home")),
            "the same folder is not followed again"
        );
        settle(&mut s, "/etc");
        assert_eq!(at(&s), "/etc", "a new folder is");
    }

    #[test]
    fn enter_on_a_folder_opens_it_in_the_browser_and_on_a_note_opens_the_folder_it_belongs_to() {
        let (_rt, mut s) = folder_state();
        settle(&mut s, "/home/me/projects");
        s.selected = Some("/home/me".into());
        assert_eq!(
            s.key(press(KeyCode::Enter), WIDE),
            Outcome::Goto(PathBuf::from("/home/me"))
        );
        let vfs = shared::MemVfs::new();
        for i in 0..20 {
            vfs.add_dir(format!("/big/d{i:02}"));
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut topology = Topology::default();
        let mut node = devices::topology::Node::new("sdz1", NodeKind::Partition);
        node.mount = Some("/big".into());
        node.usage = Some("filesystem".into());
        node.total = 100 << 30;
        node.free = 50 << 30;
        topology.add(node);
        let mut t = WiringState::with_vfs(runtime.handle().clone(), topology, Arc::new(vfs));
        click_node(&mut t, "sdz1");
        settle(&mut t, ELSEWHERE);
        let note = t
            .composed
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::More)
            .unwrap();
        assert_eq!(note.title(), "+8 more");
        t.selected = Some(note.id.clone());
        assert_eq!(
            t.key(press(KeyCode::Enter), WIDE),
            Outcome::Goto(PathBuf::from("/big"))
        );
    }

    #[test]
    fn p_on_a_folder_sends_into_that_folder_if_its_filesystem_has_room_and_refuses_if_not() {
        let (_rt, mut s) = folder_state();
        settle(&mut s, "/boot");
        let carry = |bytes: u64| Load {
            bytes,
            exact: true,
            largest: 10,
            carry: crate::cargo_fit::Carry::Copy,
            source: "/home/me/photos".into(),
        };
        s.selected = Some("/boot/grub".into());
        s.set_load(Some(carry(100)));
        assert_eq!(
            s.key(press(KeyCode::Char('p')), WIDE),
            Outcome::Send(PathBuf::from("/boot/grub")),
            "/boot has 400 bytes free and the load is 100"
        );
        s.set_load(Some(carry(900 << 40)));
        assert_eq!(s.key(press(KeyCode::Char('p')), WIDE), Outcome::Handled);
        s.actions.poll();
        let Some(Event::Status(text)) = s.take_event() else {
            panic!("expected a refusal")
        };
        assert!(text.contains("needs"), "{text}");
    }

    #[test]
    fn a_selected_folder_that_disappears_leaves_the_selection_on_something_that_exists() {
        let (_rt, mut s) = folder_state();
        settle(&mut s, "/home/me/projects");
        s.folders.collapse(Path::new("/"));
        s.rebuild();
        assert!(s.index().is_none(), "its box is gone");
        s.reselect(Some(Path::new("/home/me/projects")));
        assert!(s.index().is_some());
    }

    #[test]
    fn m_and_e_on_a_folder_say_what_they_cannot_do() {
        let (_rt, mut s) = folder_state();
        settle(&mut s, "/boot/grub");
        s.selected = Some("/boot/grub".into());
        s.key(press(KeyCode::Char('m')), WIDE);
        s.key(press(KeyCode::Char('e')), WIDE);
        s.actions.poll();
        let mut said = Vec::new();
        while let Some(Event::Status(t)) = s.take_event() {
            said.push(t);
        }
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].contains("nothing to mount"), "{said:?}");
        assert!(
            said[1].contains("cannot be ejected"),
            "a fixed disk, found through its folder: {said:?}"
        );
        assert!(!s.is_working());
    }
}
