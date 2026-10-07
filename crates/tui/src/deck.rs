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
//! selected and whether the keys are going to the cards, and what is being done to a device. Like
//! `settings_popup` it is pure: no `Frame`, no terminal; `deck_view` draws it and `main` applies
//! the few things it asks for, which are to go to a folder and to show a message.
//!
//! Nothing here waits on the machine. A scan of the devices runs on the blocking pool once a
//! second (a `statvfs` on a network share that has gone away can block for a long time, and a
//! scan still running is simply not started again), and mounting, unmounting and ejecting run
//! there too. What arrives is picked up by `poll` on the loop's next turn.

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use devices::activity::{Activity, Counters, Rates};
use devices::probe::Sys;
use devices::udisks::{self, Action};
use devices::{Device, Kind};
use ratatui::layout::{Position, Rect};

use crate::cargo_fit::{self, Load, Verdict};
use crate::deck_view;

/// How often the machine is looked at again.
const SCAN_EVERY: Duration = Duration::from_secs(1);

/// How long a card takes to slide in when a device is plugged in.
pub const SLIDE_IN: Duration = Duration::from_millis(260);

/// Shown when the keys go to the cards.
pub const HINT: &str =
    "deck: ←/→ choose  Enter open  p send marked  m mount/unmount  e eject  Tab back";

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

/// What the deck asks the caller to show or do, from work that finished in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Status(String),
    /// Go to this folder, saying `String` once there.
    Goto(PathBuf, String),
}

struct Scan {
    at: Instant,
    devices: Vec<Device>,
    counters: HashMap<String, Counters>,
}

/// What to do to one device, in order.
struct Plan {
    title: String,
    steps: Vec<(Action, String)>,
    /// Go to where it was mounted, once it is.
    then_open: bool,
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
    /// The first card drawn, kept by the renderer between frames.
    first_card: Cell<usize>,
    scans: Receiver<Scan>,
    scan_tx: Sender<Scan>,
    scanning: Arc<AtomicBool>,
    last_scan: Option<Instant>,
    events: Receiver<Event>,
    event_tx: Sender<Event>,
    /// Whether something is being done to a device right now: one thing at a time.
    working: Arc<AtomicBool>,
    pending: VecDeque<Event>,
    /// What the user is carrying, for the cards to say whether it fits.
    load: Option<Load>,
}

impl DeckState {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        let (scan_tx, scans) = channel();
        let (event_tx, events) = channel();
        Self {
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
            events,
            event_tx,
            working: Arc::new(AtomicBool::new(false)),
            pending: VecDeque::new(),
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
        while let Ok(event) = self.events.try_recv() {
            self.pending.push_back(event);
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
        self.working.load(Ordering::Acquire)
    }

    /// The next message or folder change from background work, if any.
    pub fn take_event(&mut self) -> Option<Event> {
        self.pending.pop_front()
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

    /// `Tab` takes the keys to the cards and gives them back; with them, `Left`/`Right` (and
    /// `h`/`l`) choose a card, `Enter` opens it (mounting it first if it is not mounted), `m`
    /// mounts or unmounts it, `e` ejects it and `Esc` gives the keys back. Every other key is the
    /// browser's, so `q` still quits and `j`/`k` still move through the files.
    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if key.code == KeyCode::Tab && key.modifiers == KeyModifiers::NONE {
            self.focused = !self.focused;
            return Outcome::Handled;
        }
        if !self.focused || key.modifiers.contains(KeyModifiers::CONTROL) {
            return Outcome::Pass;
        }
        match key.code {
            KeyCode::Esc => self.focused = false,
            KeyCode::Left | KeyCode::Char('h') => self.step(-1),
            KeyCode::Right | KeyCode::Char('l') => self.step(1),
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

    fn say(&self, text: impl Into<String>) {
        let _ = self.event_tx.send(Event::Status(text.into()));
    }

    fn open_selected(&mut self) -> Outcome {
        let Some(device) = self.selected_device().cloned() else {
            return Outcome::Handled;
        };
        match &device.mount {
            Some(mount) => Outcome::Goto(mount.clone()),
            None => {
                self.start(Plan {
                    title: device.title(),
                    steps: vec![(Action::Mount, device.node.clone())],
                    then_open: true,
                });
                Outcome::Handled
            }
        }
    }

    /// `p`: send what is carried to the chosen device, if it will fit. Refused here, with the
    /// reason, when it will not, which is the point of asking before the copy starts and not
    /// halfway through it. With nothing carried the caller says so.
    fn send_selected(&mut self) -> Outcome {
        let Some(device) = self.selected_device() else {
            return Outcome::Handled;
        };
        let (title, mount) = (device.title(), device.mount.clone());
        let Some(mount) = mount else {
            self.say(cargo_fit::refusal(&Verdict::Unmounted, &title));
            return Outcome::Handled;
        };
        match self.verdict(self.selected()) {
            Some(verdict) if !verdict.allows() => {
                self.say(cargo_fit::refusal(&verdict, &title));
                Outcome::Handled
            }
            _ => Outcome::Send(mount),
        }
    }

    fn toggle_selected(&mut self) {
        let Some(device) = self.selected_device().cloned() else {
            return;
        };
        if device.kind == Kind::Network {
            self.say("a network share cannot be mounted or unmounted from here");
            return;
        }
        let action = if device.is_mounted() {
            Action::Unmount
        } else {
            Action::Mount
        };
        self.start(Plan {
            title: device.title(),
            steps: vec![(action, device.node.clone())],
            then_open: false,
        });
    }

    fn eject_selected(&mut self) {
        let Some(device) = self.selected_device().cloned() else {
            return;
        };
        if !device.can_eject() {
            self.say(format!("{} cannot be ejected", device.title()));
            return;
        }
        let mut steps = Vec::new();
        if device.is_mounted() {
            steps.push((Action::Unmount, device.node.clone()));
        }
        steps.push((Action::PowerOff, device.disk.clone()));
        self.start(Plan {
            title: device.title(),
            steps,
            then_open: false,
        });
    }

    /// Runs `plan` on the blocking pool, one thing at a time, and reports what came of it.
    fn start(&mut self, plan: Plan) {
        if self.working.swap(true, Ordering::AcqRel) {
            self.say("still working on the last one");
            return;
        }
        self.say(match plan.steps.last() {
            Some((Action::Mount, _)) => format!("mounting {}…", plan.title),
            Some((Action::Unmount, _)) => format!("unmounting {}…", plan.title),
            _ => format!("ejecting {}…", plan.title),
        });
        let (tx, working) = (self.event_tx.clone(), Arc::clone(&self.working));
        self.handle.spawn_blocking(move || {
            let event = carry_out(&plan);
            let _ = tx.send(event);
            working.store(false, Ordering::Release);
        });
    }
}

/// Does each step of `plan` in turn, stopping at the first that fails.
fn carry_out(plan: &Plan) -> Event {
    let mut mounted_at = None;
    for (action, node) in &plan.steps {
        match udisks::run(*action, node) {
            Ok(done) => mounted_at = done.mounted_at.or(mounted_at),
            Err(why) => return Event::Status(format!("{}: {why}", plan.title)),
        }
    }
    match (plan.steps.last(), mounted_at) {
        (Some((Action::Mount, _)), Some(at)) if plan.then_open => {
            Event::Goto(at, format!("mounted {}", plan.title))
        }
        (Some((Action::Mount, _)), at) => Event::Status(match at {
            Some(at) => format!("mounted {} at {}", plan.title, at.display()),
            None => format!("mounted {}", plan.title),
        }),
        (Some((Action::Unmount, _)), _) => Event::Status(format!("unmounted {}", plan.title)),
        _ => Event::Status(format!("{} ejected, safe to remove", plan.title)),
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
    fn tab_takes_the_keys_to_the_cards_and_gives_them_back() {
        let (_rt, mut deck) = deck_with(three());
        assert!(!deck.focused());
        assert_eq!(
            deck.key(press(KeyCode::Right)),
            Outcome::Pass,
            "the browser's until Tab"
        );
        assert_eq!(deck.key(press(KeyCode::Tab)), Outcome::Handled);
        assert!(deck.focused());
        assert_eq!(deck.key(press(KeyCode::Esc)), Outcome::Handled);
        assert!(!deck.focused());
        deck.key(press(KeyCode::Tab));
        deck.key(press(KeyCode::Tab));
        assert!(!deck.focused());
    }

    #[test]
    fn the_arrows_choose_cards_and_stop_at_the_ends() {
        let (_rt, mut deck) = deck_with(three());
        deck.key(press(KeyCode::Tab));
        assert_eq!(deck.selected(), 0);
        deck.key(press(KeyCode::Left));
        assert_eq!(deck.selected(), 0);
        deck.key(press(KeyCode::Right));
        deck.key(press(KeyCode::Char('l')));
        deck.key(press(KeyCode::Right));
        assert_eq!(deck.selected(), 2);
        deck.key(press(KeyCode::Char('h')));
        assert_eq!(deck.selected(), 1);
    }

    #[test]
    fn keys_the_deck_does_not_use_stay_the_browsers_even_with_it_focused() {
        let (_rt, mut deck) = deck_with(three());
        deck.key(press(KeyCode::Tab));
        for code in [
            KeyCode::Char('q'),
            KeyCode::Char('j'),
            KeyCode::Down,
            KeyCode::Char('x'),
        ] {
            assert_eq!(deck.key(press(code)), Outcome::Pass, "{code:?}");
        }
        assert_eq!(
            deck.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL)),
            Outcome::Pass
        );
    }

    #[test]
    fn enter_on_a_mounted_card_goes_to_its_folder() {
        let (_rt, mut deck) = deck_with(three());
        deck.key(press(KeyCode::Tab));
        deck.key(press(KeyCode::Right));
        assert_eq!(
            deck.key(press(KeyCode::Enter)),
            Outcome::Goto(PathBuf::from("/run/media/me/STICK"))
        );
    }

    #[test]
    fn a_network_share_and_a_fixed_disk_refuse_what_they_cannot_do_and_say_why() {
        let (_rt, mut deck) = deck_with(vec![
            device("nas:/export", Some("/mnt/nas"), Kind::Network),
            device("sda1", Some("/"), Kind::Internal),
        ]);
        deck.key(press(KeyCode::Tab));
        deck.key(press(KeyCode::Char('m')));
        deck.key(press(KeyCode::Right));
        deck.key(press(KeyCode::Char('e')));
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
        deck.key(press(KeyCode::Tab));
        assert_eq!(
            deck.key(press(KeyCode::Char('p'))),
            Outcome::Send(PathBuf::from("/")),
            "the first card has room"
        );
        deck.key(press(KeyCode::Right));
        assert_eq!(deck.key(press(KeyCode::Char('p'))), Outcome::Handled);
        deck.key(press(KeyCode::Right));
        assert_eq!(deck.key(press(KeyCode::Char('p'))), Outcome::Handled);
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
        deck.key(press(KeyCode::Tab));
        assert_eq!(
            deck.key(press(KeyCode::Char('p'))),
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
        deck.key(press(KeyCode::Tab));
        assert!(matches!(
            deck.key(press(KeyCode::Char('p'))),
            Outcome::Send(_)
        ));
    }

    #[test]
    fn the_selection_follows_its_device_when_the_list_changes() {
        let (_rt, mut deck) = deck_with(three());
        deck.key(press(KeyCode::Tab));
        deck.key(press(KeyCode::Right));
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
