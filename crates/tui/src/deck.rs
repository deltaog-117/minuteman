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

//! The deck's state (`V`, the deck view): the devices found, how busy each is, which card is
//! selected and whether the keys are going to the rail, and what is being done to a device. Like
//! `settings_popup` it is pure: no `Frame`, no terminal; `deck_view` draws it and `main` applies
//! the few things it asks for, which are to go to a folder and to show a message.
//!
//! Nothing here waits on the machine. A scan of the devices runs on the blocking pool once a
//! second (a `statvfs` on a network share that has gone away can block for a long time, and a
//! scan still running is simply not started again), and mounting, unmounting and ejecting run
//! there too. What arrives is picked up by `poll` on the loop's next turn.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use devices::Device;
use devices::activity::{Activity, Counters, Rates};
use devices::probe::Sys;
use ratatui::layout::{Position, Rect};

use crate::cargo_fit::{self, Load, Verdict};
use crate::deck_view;
use crate::device_actions::Actions;

/// How often the machine is looked at again.
const SCAN_EVERY: Duration = Duration::from_secs(1);

/// How long a card takes to slide in when a device is plugged in.
pub const SLIDE_IN: Duration = Duration::from_millis(260);

/// Shown when the keys go to the cards.
pub const HINT: &str =
    "devices: j/k choose  Enter open  p send marked  m mount/unmount  e eject  l/Esc back";

/// What a key or click did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not the deck's: the browser handles it as usual.
    Pass,
    /// Handled here.
    Handled,
    /// Go to this folder.
    Goto(PathBuf),
    /// Copy what is being carried into this folder, the root of the chosen device. The caller
    /// holds the files, so it does the copy; the deck has already checked that it will fit.
    Send(PathBuf),
}

pub use crate::device_actions::Event;

struct Scan {
    at: Instant,
    devices: Vec<Device>,
    counters: HashMap<String, Counters>,
}

pub struct DeckState {
    handle: tokio::runtime::Handle,
    devices: Vec<Device>,
    activity: Activity,
    /// When the counters behind `activity` were read, to turn the difference into a rate.
    activity_at: Option<Instant>,
    /// When each device was first seen, so a new one slides in. The first scan marks everything
    /// as long since seen.
    first_seen: HashMap<String, Instant>,
    scanned: bool,
    /// The node of the selected card, so the selection stays on its device as the list changes.
    selected_node: Option<String>,
    focused: bool,
    /// The first line item (a heading or a card) drawn, kept by the renderer between frames.
    first_card: Cell<usize>,
    scans: Receiver<Scan>,
    scan_tx: Sender<Scan>,
    scanning: Arc<AtomicBool>,
    last_scan: Option<Instant>,
    actions: Actions,
    /// What the user is carrying, for the cards to say whether it fits.
    load: Option<Load>,
}

impl DeckState {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        let (scan_tx, scans) = channel();
        Self {
            actions: Actions::new(handle.clone()),
            handle,
            devices: Vec::new(),
            activity: Activity::new(),
            activity_at: None,
            first_seen: HashMap::new(),
            scanned: false,
            selected_node: None,
            focused: false,
            first_card: Cell::new(0),
            scans,
            scan_tx,
            scanning: Arc::new(AtomicBool::new(false)),
            last_scan: None,
            load: None,
        }
    }

    /// A deck that already holds `devices`, without ever looking at the real machine: for the
    /// tests of this module and of how it is drawn.
    #[cfg(test)]
    pub fn with_devices(handle: tokio::runtime::Handle, devices: Vec<Device>) -> Self {
        let mut deck = Self::new(handle);
        deck.devices = devices;
        deck.scanned = true;
        deck.reselect();
        deck
    }

    /// Records that `node` appeared at `at`, so its card is mid-slide then.
    #[cfg(test)]
    pub fn mark_arrived(&mut self, node: &str, at: Instant) {
        self.first_seen.insert(node.to_string(), at);
    }

    #[cfg(test)]
    pub fn devices_mut(&mut self) -> &mut Vec<Device> {
        &mut self.devices
    }

    #[cfg(test)]
    pub fn set_scanned(&mut self, scanned: bool) {
        self.scanned = scanned;
    }

    /// Takes in a finished scan and starts the next when it is time. Call it every turn of the
    /// loop; it never waits.
    pub fn poll(&mut self, now: Instant) {
        while let Ok(scan) = self.scans.try_recv() {
            self.take(scan, now);
        }
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
                let devices = sys.scan();
                let counters = sys.counters();
                let _ = tx.send(Scan {
                    at: Instant::now(),
                    devices,
                    counters,
                });
                scanning.store(false, Ordering::Release);
            });
        }
    }

    fn take(&mut self, scan: Scan, now: Instant) {
        let seconds = self
            .activity_at
            .map_or(1.0, |before| scan.at.duration_since(before).as_secs_f64());
        self.activity_at = Some(scan.at);
        self.activity.update(&scan.counters, seconds);
        // A device seen for the first time slides in, except on the very first scan, when
        // everything is already there.
        let long_ago = now.checked_sub(SLIDE_IN * 4).unwrap_or(now);
        self.first_seen
            .retain(|node, _| scan.devices.iter().any(|d| &d.node == node));
        for device in &scan.devices {
            self.first_seen
                .entry(device.node.clone())
                .or_insert(if self.scanned { now } else { long_ago });
        }
        self.scanned = true;
        self.devices = scan.devices;
        self.reselect();
    }

    /// Puts the selection back on the device it was on, or on the nearest card if that is gone.
    fn reselect(&mut self) {
        let at = self
            .selected_node
            .as_deref()
            .and_then(|node| self.devices.iter().position(|d| d.node == node))
            .unwrap_or_else(|| self.selected().min(self.devices.len().saturating_sub(1)));
        self.selected_node = self.devices.get(at).map(|d| d.node.clone());
    }

    /// Tells the deck what is being carried (or that nothing is), so each card can say whether it
    /// fits. Cheap; call it every turn.
    pub fn set_load(&mut self, load: Option<Load>) {
        self.load = load;
    }

    /// Whether the load fits on the device at `index`, or `None` when nothing is carried.
    pub fn verdict(&self, index: usize) -> Option<Verdict> {
        let load = self.load.as_ref()?;
        let device = self.devices.get(index)?;
        let same_device = self.holding(&load.source) == Some(index);
        Some(cargo_fit::verdict(device, load, same_device))
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Whether the first scan has come back: until it has, an empty deck means "looking".
    pub fn has_scanned(&self) -> bool {
        self.scanned
    }

    pub fn selected(&self) -> usize {
        self.selected_node
            .as_deref()
            .and_then(|node| self.devices.iter().position(|d| d.node == node))
            .unwrap_or(0)
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn unfocus(&mut self) {
        self.focused = false;
    }

    pub fn first_card(&self) -> &Cell<usize> {
        &self.first_card
    }

    pub fn rates(&self, device: &Device) -> Rates {
        self.activity.rates(&device.node)
    }

    pub fn history(&self, device: &Device) -> Vec<u64> {
        self.activity.history(&device.node).collect()
    }

    /// How far through its slide in the card of `device` is, from 0 (just plugged in) to 1.
    pub fn slide(&self, device: &Device, now: Instant) -> f64 {
        self.first_seen.get(&device.node).map_or(1.0, |seen| {
            (now.saturating_duration_since(*seen).as_secs_f64() / SLIDE_IN.as_secs_f64()).min(1.0)
        })
    }

    /// Whether any card is still sliding in, so the loop should tick fast.
    pub fn animating(&self, now: Instant) -> bool {
        self.devices.iter().any(|d| self.slide(d, now) < 1.0)
    }

    /// Whether a mount, unmount or eject is in progress.
    pub fn is_working(&self) -> bool {
        self.actions.is_working()
    }

    /// The next message or folder change from background work, if any.
    pub fn take_event(&mut self) -> Option<Event> {
        self.actions.take_event()
    }

    /// The device whose mount holds `dir`, if any: the one with the longest mount path that is a
    /// parent of it, so a folder on a separately mounted `/home` belongs to that and not to `/`.
    pub fn holding(&self, dir: &Path) -> Option<usize> {
        holding(&self.devices, dir)
    }

    fn step(&mut self, by: isize) {
        if self.devices.is_empty() {
            return;
        }
        let last = self.devices.len() - 1;
        let at = self.selected().saturating_add_signed(by).min(last);
        self.selected_node = Some(self.devices[at].node.clone());
    }

    /// Where the keys go in a deck whose rail is at the left of the files (`visible`; with no
    /// room for the rail they are all the browser's). With the keys on the files, `Tab` or `h`
    /// takes them to the rail, and `l` is nothing, since going into a file or folder is `Enter`'s
    /// alone here. With them on the rail, `j`/`k` (and the up and down arrows) choose a card,
    /// `Enter` opens it (mounting it first if it is not mounted), `m` mounts or unmounts it, `e`
    /// ejects it, `p` sends what is carried, and `l`, `Tab` or `Esc` give the keys back. Every
    /// other key is the browser's, so `q` still quits.
    pub fn key(&mut self, key: KeyEvent, visible: bool) -> Outcome {
        if !visible {
            self.focused = false;
            return Outcome::Pass;
        }
        if key.code == KeyCode::Tab && key.modifiers == KeyModifiers::NONE {
            self.focused = !self.focused;
            return Outcome::Handled;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Outcome::Pass;
        }
        let plain = key.modifiers == KeyModifiers::NONE;
        if !self.focused {
            return match key.code {
                KeyCode::Char('h') if plain => {
                    self.focused = true;
                    Outcome::Handled
                }
                KeyCode::Char('l') if plain => Outcome::Handled,
                _ => Outcome::Pass,
            };
        }
        match key.code {
            KeyCode::Esc | KeyCode::Right | KeyCode::Char('l') => self.focused = false,
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            // The rail is the leftmost thing there is: nowhere further to go.
            KeyCode::Left | KeyCode::Char('h') => {}
            KeyCode::Enter => return self.open_selected(),
            KeyCode::Char('p') => return self.send_selected(),
            KeyCode::Char('m') => self.toggle_selected(),
            KeyCode::Char('e') => self.eject_selected(),
            _ => return Outcome::Pass,
        }
        Outcome::Handled
    }

    /// A click on the card under `pointer` in a deck drawn in `area` selects it and takes the
    /// keys; a click on the card that is already selected opens it. A click on no card is
    /// handled and does nothing.
    pub fn click(&mut self, area: Rect, pointer: Position) -> Outcome {
        let Some(index) = deck_view::card_at(area, self, pointer) else {
            return Outcome::Handled;
        };
        let already = self.focused && index == self.selected();
        self.focused = true;
        self.selected_node = Some(self.devices[index].node.clone());
        if already {
            self.open_selected()
        } else {
            Outcome::Handled
        }
    }

    /// Moves the selection by `by` cards, as the wheel does.
    pub fn scroll(&mut self, by: isize) {
        self.step(by);
    }

    fn selected_device(&self) -> Option<&Device> {
        self.devices.get(self.selected())
    }

    fn open_selected(&mut self) -> Outcome {
        let Some(device) = self.selected_device().cloned() else {
            return Outcome::Handled;
        };
        match self.actions.open(&device) {
            Some(mount) => Outcome::Goto(mount),
            None => Outcome::Handled,
        }
    }

    /// `p`: send what is carried to the chosen device, if it will fit. Refused here, with the
    /// reason, when it will not, which is the point of asking before the copy starts and not
    /// halfway through it. With nothing carried the caller says so.
    fn send_selected(&mut self) -> Outcome {
        let Some(device) = self.selected_device().cloned() else {
            return Outcome::Handled;
        };
        match self.actions.send_to(&device, self.verdict(self.selected())) {
            Some(mount) => Outcome::Send(mount),
            None => Outcome::Handled,
        }
    }

    fn toggle_selected(&mut self) {
        if let Some(device) = self.selected_device().cloned() {
            self.actions.toggle(&device);
        }
    }

    fn eject_selected(&mut self) {
        if let Some(device) = self.selected_device().cloned() {
            self.actions.eject(&device);
        }
    }
}

/// The index of the device whose mount holds `dir`, preferring the deepest mount.
pub fn holding(devices: &[Device], dir: &Path) -> Option<usize> {
    devices
        .iter()
        .enumerate()
        .filter_map(|(index, device)| {
            let mount = device.mount.as_deref()?;
            dir.starts_with(mount)
                .then_some((mount.as_os_str().len(), index))
        })
        .max()
        .map(|(_, index)| index)
}

#[cfg(test)]
mod tests {
    use devices::Kind;

    use super::*;

    fn device(node: &str, mount: Option<&str>, kind: Kind) -> Device {
        Device {
            node: node.into(),
            disk: node.trim_end_matches(char::is_numeric).into(),
            label: None,
            fs: Some("ext4".into()),
            mount: mount.map(PathBuf::from),
            total: 1000,
            free: 500,
            kind,
        }
    }

    /// A deck that already holds `devices`, without ever looking at the real machine.
    fn deck_with(devices: Vec<Device>) -> (tokio::runtime::Runtime, DeckState) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let deck = DeckState::with_devices(runtime.handle().clone(), devices);
        (runtime, deck)
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn three() -> Vec<Device> {
        vec![
            device("sda1", Some("/"), Kind::Internal),
            device("sdb1", Some("/run/media/me/STICK"), Kind::Removable),
            device("sdc1", None, Kind::Removable),
        ]
    }

    #[test]
    fn the_deepest_mount_holding_a_folder_is_the_device_it_is_on() {
        let devices = vec![
            device("sda1", Some("/"), Kind::Internal),
            device("sda2", Some("/home"), Kind::Internal),
            device("sdc1", None, Kind::Removable),
        ];
        assert_eq!(holding(&devices, Path::new("/home/me/docs")), Some(1));
        assert_eq!(holding(&devices, Path::new("/etc")), Some(0));
        assert_eq!(
            holding(&devices, Path::new("/homestead")),
            Some(0),
            "a name, not a prefix"
        );
        assert_eq!(holding(&[], Path::new("/")), None);
    }

    #[test]
    fn tab_takes_the_keys_to_the_rail_and_gives_them_back() {
        let (_rt, mut deck) = deck_with(three());
        assert!(!deck.focused());
        assert_eq!(
            deck.key(press(KeyCode::Down), true),
            Outcome::Pass,
            "the browser's until Tab"
        );
        assert_eq!(deck.key(press(KeyCode::Tab), true), Outcome::Handled);
        assert!(deck.focused());
        assert_eq!(deck.key(press(KeyCode::Esc), true), Outcome::Handled);
        assert!(!deck.focused());
        deck.key(press(KeyCode::Tab), true);
        deck.key(press(KeyCode::Tab), true);
        assert!(!deck.focused());
    }

    #[test]
    fn h_goes_left_to_the_rail_and_l_comes_back_to_the_files() {
        let (_rt, mut deck) = deck_with(three());
        assert_eq!(deck.key(press(KeyCode::Char('h')), true), Outcome::Handled);
        assert!(deck.focused());
        assert_eq!(deck.key(press(KeyCode::Char('h')), true), Outcome::Handled);
        assert!(deck.focused(), "nowhere further left");
        assert_eq!(deck.key(press(KeyCode::Char('l')), true), Outcome::Handled);
        assert!(!deck.focused());
        deck.key(press(KeyCode::Tab), true);
        deck.key(press(KeyCode::Right), true);
        assert!(!deck.focused());
    }

    #[test]
    fn l_alone_no_longer_goes_into_the_file_under_the_cursor() {
        let (_rt, mut deck) = deck_with(three());
        assert_eq!(
            deck.key(press(KeyCode::Char('l')), true),
            Outcome::Handled,
            "swallowed, so it never reaches the browser's Enter"
        );
        assert_eq!(deck.key(press(KeyCode::Enter), true), Outcome::Pass);
        let alt = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::ALT);
        assert_eq!(
            deck.key(alt, true),
            Outcome::Pass,
            "the Alt layer is not ours"
        );
    }

    #[test]
    fn j_and_k_choose_cards_and_stop_at_the_ends() {
        let (_rt, mut deck) = deck_with(three());
        deck.key(press(KeyCode::Tab), true);
        assert_eq!(deck.selected(), 0);
        deck.key(press(KeyCode::Char('k')), true);
        assert_eq!(deck.selected(), 0);
        deck.key(press(KeyCode::Char('j')), true);
        deck.key(press(KeyCode::Down), true);
        deck.key(press(KeyCode::Char('j')), true);
        assert_eq!(deck.selected(), 2);
        deck.key(press(KeyCode::Up), true);
        assert_eq!(deck.selected(), 1);
    }

    #[test]
    fn keys_the_deck_does_not_use_stay_the_browsers_even_with_it_focused() {
        let (_rt, mut deck) = deck_with(three());
        deck.key(press(KeyCode::Tab), true);
        for code in [KeyCode::Char('q'), KeyCode::Char('x'), KeyCode::Backspace] {
            assert_eq!(deck.key(press(code), true), Outcome::Pass, "{code:?}");
        }
        assert_eq!(
            deck.key(
                KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL),
                true
            ),
            Outcome::Pass
        );
    }

    #[test]
    fn with_no_room_for_the_rail_every_key_is_the_browsers() {
        let (_rt, mut deck) = deck_with(three());
        deck.key(press(KeyCode::Tab), true);
        for code in [KeyCode::Tab, KeyCode::Char('h'), KeyCode::Char('l')] {
            assert_eq!(deck.key(press(code), false), Outcome::Pass, "{code:?}");
        }
        assert!(
            !deck.focused(),
            "the keys cannot stay on a rail that is not there"
        );
    }

    #[test]
    fn enter_on_a_mounted_card_goes_to_its_folder() {
        let (_rt, mut deck) = deck_with(three());
        deck.key(press(KeyCode::Tab), true);
        deck.key(press(KeyCode::Down), true);
        assert_eq!(
            deck.key(press(KeyCode::Enter), true),
            Outcome::Goto(PathBuf::from("/run/media/me/STICK"))
        );
    }

    #[test]
    fn a_network_share_and_a_fixed_disk_refuse_what_they_cannot_do_and_say_why() {
        let (_rt, mut deck) = deck_with(vec![
            device("nas:/export", Some("/mnt/nas"), Kind::Network),
            device("sda1", Some("/"), Kind::Internal),
        ]);
        deck.key(press(KeyCode::Tab), true);
        deck.key(press(KeyCode::Char('m')), true);
        deck.key(press(KeyCode::Down), true);
        deck.key(press(KeyCode::Char('e')), true);
        deck.poll(Instant::now());
        let mut said = Vec::new();
        while let Some(Event::Status(text)) = deck.take_event() {
            said.push(text);
        }
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].contains("network share"));
        assert!(said[1].contains("cannot be ejected"));
        assert!(!deck.is_working());
    }

    fn load(bytes: u64, largest: u64) -> Load {
        Load {
            bytes,
            exact: true,
            largest,
            carry: cargo_fit::Carry::Copy,
            source: "/home/me/photos".into(),
        }
    }

    fn with_sizes(mut devices: Vec<Device>, free: &[u64]) -> Vec<Device> {
        for (device, free) in devices.iter_mut().zip(free) {
            device.free = *free;
            device.total = 1 << 40;
        }
        devices
    }

    #[test]
    fn each_card_gets_its_own_verdict_for_what_is_carried() {
        let (_rt, mut deck) = deck_with(with_sizes(three(), &[1 << 39, 1 << 20, 0]));
        assert_eq!(deck.verdict(0), None, "nothing carried, nothing to judge");
        deck.set_load(Some(load(1 << 30, 1 << 20)));
        assert!(matches!(deck.verdict(0), Some(Verdict::Fits { .. })));
        assert!(matches!(deck.verdict(1), Some(Verdict::Short { .. })));
        assert_eq!(deck.verdict(2), Some(Verdict::Unmounted));
        assert_eq!(deck.verdict(9), None);
        deck.set_load(None);
        assert_eq!(deck.verdict(0), None);
    }

    #[test]
    fn p_sends_to_a_device_it_fits_on_and_refuses_one_it_does_not_with_the_reason() {
        let (_rt, mut deck) = deck_with(with_sizes(three(), &[1 << 39, 1 << 20, 0]));
        deck.set_load(Some(load(1 << 30, 1 << 20)));
        deck.key(press(KeyCode::Tab), true);
        assert_eq!(
            deck.key(press(KeyCode::Char('p')), true),
            Outcome::Send(PathBuf::from("/")),
            "the first card has room"
        );
        deck.key(press(KeyCode::Down), true);
        assert_eq!(deck.key(press(KeyCode::Char('p')), true), Outcome::Handled);
        deck.key(press(KeyCode::Down), true);
        assert_eq!(deck.key(press(KeyCode::Char('p')), true), Outcome::Handled);
        deck.poll(Instant::now());
        let mut said = Vec::new();
        while let Some(Event::Status(text)) = deck.take_event() {
            said.push(text);
        }
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(
            said[0].contains("needs") && said[0].contains("more room"),
            "{said:?}"
        );
        assert!(said[1].contains("not mounted"), "{said:?}");
    }

    #[test]
    fn p_with_nothing_carried_still_goes_to_the_caller_who_says_so() {
        let (_rt, mut deck) = deck_with(with_sizes(three(), &[1 << 39, 1 << 39, 0]));
        deck.key(press(KeyCode::Tab), true);
        assert_eq!(
            deck.key(press(KeyCode::Char('p')), true),
            Outcome::Send(PathBuf::from("/"))
        );
    }

    #[test]
    fn a_move_within_the_disk_it_is_on_is_allowed_whatever_is_free() {
        let mut devices = with_sizes(three(), &[0, 1 << 39, 0]);
        devices[0].mount = Some("/home".into());
        let (_rt, mut deck) = deck_with(devices);
        let mut moving = load(1 << 38, 1 << 20);
        moving.carry = cargo_fit::Carry::Move;
        deck.set_load(Some(moving));
        assert_eq!(deck.verdict(0), Some(Verdict::SameDisk));
        deck.key(press(KeyCode::Tab), true);
        assert!(matches!(
            deck.key(press(KeyCode::Char('p')), true),
            Outcome::Send(_)
        ));
    }

    #[test]
    fn the_selection_follows_its_device_when_the_list_changes() {
        let (_rt, mut deck) = deck_with(three());
        deck.key(press(KeyCode::Tab), true);
        deck.key(press(KeyCode::Down), true);
        assert_eq!(deck.devices()[deck.selected()].node, "sdb1");
        let mut shifted = three();
        shifted.insert(0, device("nvme0n1p1", Some("/data"), Kind::Internal));
        deck.devices = shifted;
        deck.reselect();
        assert_eq!(deck.devices()[deck.selected()].node, "sdb1");
        deck.devices.retain(|d| d.node != "sdb1");
        deck.reselect();
        assert!(
            deck.selected() < deck.devices().len(),
            "it lands on a card that exists"
        );
    }

    #[test]
    fn a_device_plugged_in_after_the_first_scan_slides_in_and_the_first_ones_do_not() {
        let (_rt, mut deck) = deck_with(Vec::new());
        let now = Instant::now();
        deck.scanned = false;
        deck.take(
            Scan {
                at: now,
                devices: vec![device("sda1", Some("/"), Kind::Internal)],
                counters: HashMap::new(),
            },
            now,
        );
        assert!(
            !deck.animating(now),
            "everything already there is not an arrival"
        );
        let later = now + Duration::from_secs(2);
        deck.take(
            Scan {
                at: later,
                devices: three(),
                counters: HashMap::new(),
            },
            later,
        );
        let stick = deck
            .devices()
            .iter()
            .find(|d| d.node == "sdb1")
            .unwrap()
            .clone();
        assert!(deck.slide(&stick, later) < 0.01);
        assert!(deck.animating(later));
        assert_eq!(deck.slide(&stick, later + SLIDE_IN), 1.0);
        assert!(!deck.animating(later + SLIDE_IN));
        let system = deck
            .devices()
            .iter()
            .find(|d| d.node == "sda1")
            .unwrap()
            .clone();
        assert_eq!(deck.slide(&system, later), 1.0);
    }
}
