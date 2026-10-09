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

//! The atlas's state (`V`, the atlas view): a map of the machine's storage that zooms from the
//! volumes, to the biggest folders on one of them, to the files in a folder. Like the deck and the
//! wiring diagram it is pure: no `Frame`, no terminal. `atlas_view` draws it and `main` applies
//! the few things it asks for, which are to go to a folder and to show a message.
//!
//! The top level is one block per mounted or plugged-in volume, sized by capacity (`devices`, looked
//! at once every couple of seconds on the blocking pool while the view shows). Zooming into a
//! volume hands its mount point to the disk usage view's scanner (`disk_usage::DiskUsageView`),
//! which already scans one folder at a time on the blocking pool, cancels when dropped, sorts
//! biggest first and keeps a stack of the levels on the way down, so zooming back out is instant
//! and nothing holds a whole tree. The atlas only adds the picture: what a tile is, where it sits
//! (`atlas_layout`), and how to move between tiles.
//!
//! Nothing here waits on the disk. A scan in progress shows the rows it has so far, and the map
//! re-lays itself out as more arrive.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use devices::Device;
use devices::probe::Sys;
use ratatui::layout::{Position, Rect};
use shared::Vfs;

use crate::atlas_layout::{self, Dir, Item, Tile};
use crate::deck::holding;
use crate::disk_usage::{DiskUsageView, Kind as RowKind};

/// How often the machine's volumes are looked at again.
const SCAN_EVERY: Duration = Duration::from_secs(2);

/// Shown on entering the view.
pub const HINT: &str = "atlas: arrows move  Enter zoom in  Backspace out  o open in browser  Tab next  a size/apparent  r rescan  V next view";

/// What a key or click did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not the atlas's: the browser handles it as usual.
    Pass,
    /// Handled here.
    Handled,
    /// Go to this folder in the browser.
    Goto(PathBuf),
    /// Say this on the status line.
    Status(String),
}

/// A volume zoomed into.
struct Zoom {
    view: DiskUsageView,
    /// Folders gone down into below the volume's own mount point, so going back out knows when
    /// it has reached the volume again (the view itself would carry on up to the parent folder).
    depth: usize,
    /// The kernel name of the volume, so leaving puts the selection back on it.
    node: String,
}

pub struct AtlasState {
    handle: tokio::runtime::Handle,
    vfs: Arc<dyn Vfs>,
    /// The volumes, biggest first: the order the map lays them out in.
    devices: Vec<Device>,
    scanned: bool,
    /// The kernel name of the chosen volume, so the choice stays on it as the machine changes.
    selected_node: Option<String>,
    zoom: Option<Zoom>,
    scans: Receiver<Vec<Device>>,
    scan_tx: Sender<Vec<Device>>,
    scanning: Arc<AtomicBool>,
    last_scan: Option<Instant>,
}

impl AtlasState {
    pub fn new(handle: tokio::runtime::Handle, vfs: Arc<dyn Vfs>) -> Self {
        let (scan_tx, scans) = channel();
        Self {
            handle,
            vfs,
            devices: Vec::new(),
            scanned: false,
            selected_node: None,
            zoom: None,
            scans,
            scan_tx,
            scanning: Arc::new(AtomicBool::new(false)),
            last_scan: None,
        }
    }

    /// An atlas that already knows its volumes and never looks at the machine, for tests.
    #[cfg(test)]
    pub fn with_devices(
        handle: tokio::runtime::Handle,
        vfs: Arc<dyn Vfs>,
        devices: Vec<Device>,
    ) -> Self {
        let mut state = Self::new(handle, vfs);
        state.last_scan = Some(Instant::now() + Duration::from_secs(3600));
        state.take(devices);
        state
    }

    /// Picks up what the scans have found and keeps the zoomed-into folder's scan moving. Call
    /// once per turn of the loop; `here` is the folder the browser is in, which the choice starts
    /// on.
    pub fn poll(&mut self, now: Instant, here: &std::path::Path) {
        while let Ok(devices) = self.scans.try_recv() {
            self.take(devices);
        }
        if self.selected_node.is_none() {
            let at = holding(&self.devices, here).or(if self.devices.is_empty() {
                None
            } else {
                Some(0)
            });
            self.selected_node = at.map(|i| self.devices[i].node.clone());
        }
        if let Some(zoom) = self.zoom.as_mut() {
            zoom.view.poll();
        }
        let due = self
            .last_scan
            .is_none_or(|at| now.duration_since(at) >= SCAN_EVERY);
        if due && !self.scanning.swap(true, Ordering::AcqRel) {
            self.last_scan = Some(now);
            let (tx, scanning) = (self.scan_tx.clone(), Arc::clone(&self.scanning));
            // A `statvfs` on a share that has gone away can block for a long time, which is why
            // this is off the loop and a scan still running is not started again.
            self.handle.spawn_blocking(move || {
                let _ = tx.send(Sys::real().scan());
                scanning.store(false, Ordering::Release);
            });
        }
    }

    fn take(&mut self, mut devices: Vec<Device>) {
        // The map is laid out in this order, so the biggest volume comes first and a stable
        // tie-break keeps a volume from jumping about between scans.
        devices.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.node.cmp(&b.node)));
        self.devices = devices;
        self.scanned = true;
        let kept = self
            .selected_node
            .as_ref()
            .is_some_and(|node| self.devices.iter().any(|d| &d.node == node));
        if !kept {
            self.selected_node = None;
        }
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn has_scanned(&self) -> bool {
        self.scanned
    }

    /// The scan of the folder zoomed into, when zoomed into one.
    pub fn zoomed(&self) -> Option<&DiskUsageView> {
        self.zoom.as_ref().map(|zoom| &zoom.view)
    }

    /// The volume zoomed into, when zoomed into one.
    pub fn zoomed_device(&self) -> Option<&Device> {
        let node = &self.zoom.as_ref()?.node;
        self.devices.iter().find(|d| &d.node == node)
    }

    /// The size of each thing on the map, in the order the map is laid out in: the volumes'
    /// capacities, or the zoomed-into folder's rows by the measure chosen.
    pub fn weights(&self) -> Vec<u64> {
        match &self.zoom {
            None => self.devices.iter().map(|d| d.total).collect(),
            Some(zoom) => {
                let measure = zoom.view.measure();
                zoom.view
                    .rows()
                    .iter()
                    .map(|row| row.sizes.of(measure))
                    .collect()
            }
        }
    }

    pub fn tiles(&self, map: Rect) -> Vec<Tile> {
        atlas_layout::treemap(&self.weights(), map)
    }

    /// The position, among `weights`, of the chosen thing.
    pub fn selected(&self) -> Option<usize> {
        match &self.zoom {
            None => {
                let node = self.selected_node.as_ref()?;
                self.devices.iter().position(|d| &d.node == node)
            }
            Some(zoom) => (!zoom.view.rows().is_empty()).then(|| zoom.view.selected()),
        }
    }

    fn select(&mut self, index: usize) {
        match self.zoom.as_mut() {
            None => {
                if let Some(device) = self.devices.get(index) {
                    self.selected_node = Some(device.node.clone());
                }
            }
            Some(zoom) => zoom.view.move_to(index),
        }
    }

    /// The thing the browsed folder `here` is inside, if it is on the map: the volume mounted
    /// there, or the folder (one of the rows) it is under. Marked on the map as "you are here".
    pub fn pin(&self, here: &std::path::Path) -> Option<usize> {
        match &self.zoom {
            None => holding(&self.devices, here),
            Some(zoom) => zoom
                .view
                .rows()
                .iter()
                .enumerate()
                .filter(|(_, row)| row.kind == RowKind::Dir && here.starts_with(&row.path))
                .max_by_key(|(_, row)| row.path.as_os_str().len())
                .map(|(i, _)| i),
        }
    }

    /// The arrows (and `h`/`j`/`k`/`l`) move between tiles, `Tab` and `Shift-Tab` walk through
    /// them biggest first, `Enter` zooms into the chosen tile (a file's own tile opens its folder
    /// in the browser instead), `Backspace` or `Esc` zooms back out, `o` opens the chosen tile's
    /// folder in the browser, `a` switches between size on disk and apparent size and `r` looks
    /// again. Every other key is the browser's, so `V`, `q`, `/` and `:` work as they always do.
    pub fn key(&mut self, key: KeyEvent, map: Rect) -> Outcome {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT)
        {
            return Outcome::Pass;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.go(Dir::Up, map),
            KeyCode::Down | KeyCode::Char('j') => self.go(Dir::Down, map),
            KeyCode::Left | KeyCode::Char('h') => self.go(Dir::Left, map),
            KeyCode::Right | KeyCode::Char('l') => self.go(Dir::Right, map),
            KeyCode::Tab => self.walk(1, map),
            KeyCode::BackTab => self.walk(-1, map),
            KeyCode::Enter => return self.activate(),
            KeyCode::Backspace | KeyCode::Esc => {
                if !self.zoom_out() {
                    return Outcome::Pass;
                }
            }
            KeyCode::Char('o') => return self.open_in_browser(),
            KeyCode::Char('a') => {
                let Some(zoom) = self.zoom.as_mut() else {
                    return Outcome::Pass;
                };
                zoom.view.toggle_measure();
            }
            KeyCode::Char('r') => self.rescan(),
            _ => return Outcome::Pass,
        }
        Outcome::Handled
    }

    /// A click on a tile chooses it; a click on the one already chosen zooms into it. A click on
    /// nothing, or on a small tile (which stands for several entries), does nothing.
    pub fn click(&mut self, map: Rect, pointer: Position) -> Outcome {
        let tiles = self.tiles(map);
        match atlas_layout::tile_at(&tiles, pointer).map(|tile| tile.item) {
            Some(Item::Entry(i)) if self.selected() == Some(i) => self.activate(),
            Some(Item::Entry(i)) => {
                self.select(i);
                Outcome::Handled
            }
            Some(Item::Small(count)) => Outcome::Status(format!(
                "{count} smaller {} folded into one block; open the folder in the browser to see them",
                if count == 1 { "entry" } else { "entries" }
            )),
            None => Outcome::Handled,
        }
    }

    fn go(&mut self, dir: Dir, map: Rect) {
        let tiles = self.tiles(map);
        let from = self.selected().unwrap_or(0);
        if let Some(to) = atlas_layout::neighbor(&tiles, from, dir) {
            self.select(to);
        }
    }

    fn walk(&mut self, step: isize, map: Rect) {
        let tiles = self.tiles(map);
        let from = self.selected().unwrap_or(0);
        if let Some(to) = atlas_layout::cycle(&tiles, from, step) {
            self.select(to);
        }
    }

    fn activate(&mut self) -> Outcome {
        let Some(chosen) = self.selected() else {
            return Outcome::Handled;
        };
        let Some(zoom) = self.zoom.as_mut() else {
            return self.zoom_into_volume(chosen);
        };
        let Some(row) = zoom.view.selected_row() else {
            return Outcome::Handled;
        };
        let (kind, path, name) = (row.kind, row.path.clone(), row.name.clone());
        match kind {
            RowKind::Dir => {
                if zoom.view.enter() {
                    zoom.depth += 1;
                }
                Outcome::Handled
            }
            RowKind::File | RowKind::Link | RowKind::Other => match path.parent() {
                Some(parent) => Outcome::Goto(parent.to_path_buf()),
                None => Outcome::Handled,
            },
            RowKind::OtherFilesystem => Outcome::Status(format!(
                "{name} is another filesystem; zoom into it from the map of volumes"
            )),
            RowKind::Rest => Outcome::Status(
                "these are the smallest entries, folded into one row; open the folder in the browser to see them"
                    .to_string(),
            ),
        }
    }

    fn zoom_into_volume(&mut self, index: usize) -> Outcome {
        let Some(device) = self.devices.get(index) else {
            return Outcome::Handled;
        };
        let Some(mount) = device.mount.clone() else {
            return Outcome::Status(format!(
                "{} is not mounted; mount it from the deck (m) to look inside",
                device.title()
            ));
        };
        let node = device.node.clone();
        let view = DiskUsageView::open(self.handle.clone(), Arc::clone(&self.vfs), mount);
        self.zoom = Some(Zoom {
            view,
            depth: 0,
            node,
        });
        Outcome::Handled
    }

    /// Goes up one level: out of a folder, or out of a volume back to the map of volumes.
    /// Returns whether there was anywhere to go.
    fn zoom_out(&mut self) -> bool {
        let Some(zoom) = self.zoom.as_mut() else {
            return false;
        };
        if zoom.depth > 0 {
            if zoom.view.leave() {
                zoom.depth -= 1;
            }
            return true;
        }
        self.selected_node = Some(zoom.node.clone());
        // Dropping the view cancels a scan still running.
        self.zoom = None;
        true
    }

    fn open_in_browser(&mut self) -> Outcome {
        let Some(chosen) = self.selected() else {
            return Outcome::Handled;
        };
        let Some(zoom) = self.zoom.as_ref() else {
            let device = &self.devices[chosen];
            return match device.mount.clone() {
                Some(mount) => Outcome::Goto(mount),
                None => Outcome::Status(format!("{} is not mounted", device.title())),
            };
        };
        let Some(row) = zoom.view.selected_row() else {
            return Outcome::Goto(zoom.view.path().to_path_buf());
        };
        match row.kind {
            RowKind::Dir | RowKind::OtherFilesystem => Outcome::Goto(row.path.clone()),
            // A file, or the folded rest, is shown by the folder it is in.
            _ => Outcome::Goto(zoom.view.path().to_path_buf()),
        }
    }

    fn rescan(&mut self) {
        match self.zoom.as_mut() {
            Some(zoom) => zoom.view.rescan(),
            // Look at the machine again on the next turn.
            None => self.last_scan = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use devices::Kind;
    use shared::LocalVfs;

    use super::*;

    fn volume(node: &str, mount: Option<&str>, total: u64, free: u64) -> Device {
        Device {
            node: node.into(),
            disk: node.trim_end_matches(char::is_numeric).into(),
            label: None,
            fs: Some("ext4".into()),
            mount: mount.map(PathBuf::from),
            total,
            free,
            kind: Kind::Internal,
        }
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("minuteman-atlas-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, len: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; len]).unwrap();
    }

    fn settle(state: &mut AtlasState) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            state.poll(Instant::now(), Path::new("/"));
            if state.zoomed().is_some_and(|view| view.end().is_some()) {
                return;
            }
            assert!(Instant::now() < deadline, "the scan never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    const MAP: Rect = Rect::new(0, 0, 80, 24);

    #[test]
    fn volumes_are_laid_out_biggest_first_whatever_order_they_were_found_in() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let state = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![
                volume("sdb1", None, 100, 50),
                volume("sda2", Some("/"), 900, 100),
                volume("sda1", Some("/boot"), 100, 90),
            ],
        );
        let nodes: Vec<_> = state.devices().iter().map(|d| d.node.as_str()).collect();
        assert_eq!(nodes, ["sda2", "sda1", "sdb1"], "ties by name");
        assert_eq!(state.weights(), [900, 100, 100]);
    }

    #[test]
    fn the_choice_starts_on_the_volume_holding_the_browsed_folder() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut state = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![
                volume("sda2", Some("/"), 900, 100),
                volume("sda3", Some("/home"), 500, 100),
            ],
        );
        state.poll(Instant::now(), Path::new("/home/me/docs"));
        let chosen = state.selected().unwrap();
        assert_eq!(state.devices()[chosen].node, "sda3");
        assert_eq!(state.pin(Path::new("/home/me/docs")), Some(chosen));
        assert_eq!(
            state.devices()[state.pin(Path::new("/usr")).unwrap()].node,
            "sda2"
        );
    }

    #[test]
    fn the_choice_stays_on_its_volume_when_the_list_changes_and_falls_back_when_it_goes() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut state = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![
                volume("sda2", Some("/"), 900, 1),
                volume("sdb1", None, 100, 1),
            ],
        );
        state.poll(Instant::now(), Path::new("/"));
        state.select(1);
        state.take(vec![
            volume("sdc1", None, 5000, 1),
            volume("sda2", Some("/"), 900, 1),
            volume("sdb1", None, 100, 1),
        ]);
        assert_eq!(state.devices()[state.selected().unwrap()].node, "sdb1");
        state.take(vec![volume("sda2", Some("/"), 900, 1)]);
        assert_eq!(
            state.selected(),
            None,
            "gone: no choice until the next poll"
        );
        state.poll(Instant::now(), Path::new("/"));
        assert_eq!(state.devices()[state.selected().unwrap()].node, "sda2");
    }

    #[test]
    fn an_unmounted_volume_cannot_be_zoomed_into_and_says_why() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut state = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![volume("sdb1", None, 100, 50)],
        );
        state.poll(Instant::now(), Path::new("/"));
        match state.key(press(KeyCode::Enter), MAP) {
            Outcome::Status(text) => assert!(text.contains("not mounted"), "{text}"),
            other => panic!("{other:?}"),
        }
        assert!(state.zoomed().is_none());
        assert!(matches!(
            state.key(press(KeyCode::Char('o')), MAP),
            Outcome::Status(_)
        ));
    }

    #[test]
    fn zooming_goes_volume_to_folder_to_file_and_back_out_one_level_at_a_time() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("zoom");
        write(&root.join("small"), 10);
        write(&root.join("sub/big"), 500_000);
        write(&root.join("sub/other"), 100);
        let mount = root.to_str().unwrap();
        let mut state = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![volume("sda1", Some(mount), 1_000_000, 400_000)],
        );
        state.poll(Instant::now(), Path::new("/"));

        // Into the volume: its biggest folder first.
        assert_eq!(state.key(press(KeyCode::Enter), MAP), Outcome::Handled);
        settle(&mut state);
        assert_eq!(state.zoomed_device().unwrap().node, "sda1");
        assert_eq!(state.zoomed().unwrap().rows()[0].name, "sub");
        assert_eq!(state.selected(), Some(0));

        // Into the folder, then onto its biggest file, which opens its folder in the browser.
        assert_eq!(state.key(press(KeyCode::Enter), MAP), Outcome::Handled);
        settle(&mut state);
        assert_eq!(state.zoomed().unwrap().path(), root.join("sub"));
        assert_eq!(state.zoomed().unwrap().rows()[0].name, "big");
        assert_eq!(
            state.key(press(KeyCode::Enter), MAP),
            Outcome::Goto(root.join("sub"))
        );

        // Back out: to the volume's folder, then to the map, with the volume chosen.
        assert_eq!(state.key(press(KeyCode::Backspace), MAP), Outcome::Handled);
        assert_eq!(state.zoomed().unwrap().path(), root);
        assert_eq!(state.key(press(KeyCode::Esc), MAP), Outcome::Handled);
        assert!(state.zoomed().is_none());
        assert_eq!(state.devices()[state.selected().unwrap()].node, "sda1");
        // Nothing further out: the browser's own key.
        assert_eq!(state.key(press(KeyCode::Esc), MAP), Outcome::Pass);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn going_out_of_a_volume_never_climbs_to_the_parent_of_its_mount_point() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("mount-top");
        write(&root.join("f"), 10);
        let mut state = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![volume("sda1", root.to_str(), 1_000_000, 400_000)],
        );
        state.poll(Instant::now(), Path::new("/"));
        state.key(press(KeyCode::Enter), MAP);
        settle(&mut state);
        state.key(press(KeyCode::Backspace), MAP);
        assert!(
            state.zoomed().is_none(),
            "at the volume's own top, out means the map of volumes"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn o_opens_the_chosen_folder_or_the_folder_a_file_is_in() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("open");
        write(&root.join("sub/big"), 500_000);
        write(&root.join("top.txt"), 100);
        let mut state = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![volume("sda1", root.to_str(), 1_000_000, 400_000)],
        );
        state.poll(Instant::now(), Path::new("/"));
        assert_eq!(
            state.key(press(KeyCode::Char('o')), MAP),
            Outcome::Goto(root.clone()),
            "a volume opens at its mount point"
        );
        state.key(press(KeyCode::Enter), MAP);
        settle(&mut state);
        assert_eq!(
            state.key(press(KeyCode::Char('o')), MAP),
            Outcome::Goto(root.join("sub"))
        );
        let file = state
            .zoomed()
            .unwrap()
            .rows()
            .iter()
            .position(|r| r.name == "top.txt")
            .unwrap();
        state.select(file);
        assert_eq!(
            state.key(press(KeyCode::Char('o')), MAP),
            Outcome::Goto(root.clone())
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_pin_follows_the_browsed_folder_down_into_the_rows() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("pin");
        write(&root.join("sub/deep/x"), 500_000);
        write(&root.join("elsewhere/y"), 10);
        let mut state = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![volume("sda1", root.to_str(), 1_000_000, 400_000)],
        );
        state.poll(Instant::now(), Path::new("/"));
        state.key(press(KeyCode::Enter), MAP);
        settle(&mut state);
        let rows = state.zoomed().unwrap().rows();
        let sub = rows.iter().position(|r| r.name == "sub").unwrap();
        assert_eq!(state.pin(&root.join("sub/deep")), Some(sub));
        assert_eq!(state.pin(Path::new("/nowhere")), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_measure_and_rescan_keys_belong_to_the_atlas_only_once_zoomed_in() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("measure");
        write(&root.join("f"), 10);
        let mut state = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![volume("sda1", root.to_str(), 1_000_000, 400_000)],
        );
        state.poll(Instant::now(), Path::new("/"));
        assert_eq!(
            state.key(press(KeyCode::Char('a')), MAP),
            Outcome::Pass,
            "on the map of volumes `a` is still the appearance popup"
        );
        state.key(press(KeyCode::Enter), MAP);
        settle(&mut state);
        let before = state.zoomed().unwrap().measure();
        assert_eq!(state.key(press(KeyCode::Char('a')), MAP), Outcome::Handled);
        assert_ne!(state.zoomed().unwrap().measure(), before);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn arrows_move_between_blocks_and_tab_walks_them_all() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut state = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![
                volume("a", Some("/a"), 600, 1),
                volume("b", Some("/b"), 300, 1),
                volume("c", Some("/c"), 300, 1),
            ],
        );
        state.poll(Instant::now(), Path::new("/a"));
        assert_eq!(state.selected(), Some(0));
        let tiles = state.tiles(MAP);
        assert_eq!(tiles.len(), 3, "{tiles:?}");
        // The big block is on the left, the two small ones stacked on the right.
        assert_eq!(state.key(press(KeyCode::Right), MAP), Outcome::Handled);
        let on_right = state.selected().unwrap();
        assert_ne!(on_right, 0);
        state.key(press(KeyCode::Char('j')), MAP);
        state.key(press(KeyCode::Char('k')), MAP);
        state.key(press(KeyCode::Left), MAP);
        assert_eq!(state.selected(), Some(0));

        let mut seen = vec![0];
        for _ in 0..3 {
            state.key(press(KeyCode::Tab), MAP);
            seen.push(state.selected().unwrap());
        }
        assert_eq!(seen, [0, 1, 2, 0], "Tab goes through all, then round");
        state.key(press(KeyCode::BackTab), MAP);
        assert_eq!(state.selected(), Some(2));
    }

    #[test]
    fn a_click_chooses_a_block_and_a_second_click_zooms_into_it() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("click");
        write(&root.join("f"), 10);
        let mut state = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![
                volume("big", root.to_str(), 800, 1),
                volume("small", Some("/nowhere-small"), 200, 1),
            ],
        );
        state.poll(Instant::now(), Path::new("/"));
        let tiles = state.tiles(MAP);
        let small = tiles
            .iter()
            .find(|t| t.item == Item::Entry(1))
            .unwrap()
            .rect;
        let at = Position::new(small.x + 1, small.y + 1);
        assert_eq!(state.click(MAP, at), Outcome::Handled);
        assert_eq!(state.selected(), Some(1));
        assert!(state.zoomed().is_none(), "the first click only chooses");
        // Outside the map: nothing.
        assert_eq!(state.click(MAP, Position::new(200, 200)), Outcome::Handled);
        assert_eq!(state.selected(), Some(1));
        let big = tiles
            .iter()
            .find(|t| t.item == Item::Entry(0))
            .unwrap()
            .rect;
        let on_big = Position::new(big.x + 1, big.y + 1);
        state.click(MAP, on_big);
        assert_eq!(state.selected(), Some(0));
        state.click(MAP, on_big);
        assert!(state.zoomed().is_some(), "the second click zooms in");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn other_keys_are_the_browsers() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut state = AtlasState::with_devices(rt.handle().clone(), Arc::new(LocalVfs), vec![]);
        for code in [
            KeyCode::Char('q'),
            KeyCode::Char('V'),
            KeyCode::Char('/'),
            KeyCode::Char(':'),
        ] {
            assert_eq!(state.key(press(code), MAP), Outcome::Pass, "{code:?}");
        }
        let ctrl = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert_eq!(state.key(ctrl, MAP), Outcome::Pass);
        // With nothing on the map, keys that move or open do nothing and do not panic.
        for code in [
            KeyCode::Enter,
            KeyCode::Char('o'),
            KeyCode::Tab,
            KeyCode::Right,
        ] {
            assert_eq!(state.key(press(code), MAP), Outcome::Handled, "{code:?}");
        }
    }
}
